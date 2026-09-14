//! Downloads a FEC bulk `.zip`, decodes its single pipe-delimited text
//! entry, and bulk-loads it into Postgres via `COPY FROM STDIN`.
//!
//! The work happens in two phases so the slow, CPU/network-bound parts
//! (download + zip decode, both synchronous) never block the async
//! runtime the fast part (the actual `COPY`) needs:
//!
//! 1. **Stage** (blocking thread): stream the zip -- from a local path or
//!    straight from an HTTP response body, neither of which needs to be
//!    seekable, via [`zip::read::read_zipfile_from_stream`] -- into a
//!    local temporary CSV file, re-delimited for Postgres `COPY` and with
//!    the `cycle` column appended to every row. A `limit` stops both the
//!    zip decode *and* the underlying HTTP download early, so sampling
//!    the first N rows of a multi-gigabyte source (`schedule_a`,
//!    `committee_to_committee_transactions`) never requires downloading
//!    the whole file.
//! 2. **Load** (async): stream that staged file straight into
//!    `COPY <table> (...) FROM STDIN (FORMAT csv, DELIMITER '|')` via
//!    [`sqlx::postgres::PgPoolCopyExt`].

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;

use sqlx::PgPool;
use sqlx::postgres::PgPoolCopyExt;

use super::error::{BulkError, Result};
use super::source::BulkSource;

/// Where to read the zip archive from.
pub enum Input {
    LocalFile(PathBuf),
    Url(String),
}

/// Outcome of staging + loading one bulk source.
pub struct LoadReport {
    pub rows_loaded: u64,
    pub table: &'static str,
}

/// Downloads/opens `input`, decodes `source`'s single entry, and COPYs it
/// into `source.table` tagged with `cycle`. `limit` caps the number of
/// data rows read (useful for sampling multi-gigabyte sources without a
/// full download); `None` loads every row.
pub async fn load(
    pool: &PgPool,
    source: &'static BulkSource,
    input: Input,
    cycle: u16,
    limit: Option<u64>,
) -> Result<LoadReport> {
    let staged =
        tokio::task::spawn_blocking(move || stage_to_temp_csv(source, input, cycle, limit))
            .await
            .map_err(|e| BulkError::Io(std::io::Error::other(e)))??;

    let rows_loaded = copy_into_table(pool, source, &staged.path).await?;
    // Best-effort cleanup; a leftover temp file is harmless.
    let _ = std::fs::remove_file(&staged.path);

    Ok(LoadReport {
        rows_loaded,
        table: source.table,
    })
}

struct StagedFile {
    path: PathBuf,
}

/// Blocking: reads the zip (local file or streamed HTTP body), decodes the
/// first entry, re-delimits + appends `cycle`, and writes to a fresh temp
/// file. Runs entirely off the async runtime.
fn stage_to_temp_csv(
    source: &BulkSource,
    input: Input,
    cycle: u16,
    limit: Option<u64>,
) -> Result<StagedFile> {
    let reader: Box<dyn Read> = match input {
        Input::LocalFile(path) => Box::new(BufReader::new(File::open(path)?)),
        Input::Url(url) => {
            let resp = ureq::get(&url)
                .call()
                .map_err(|e| BulkError::Http(e.to_string()))?;
            let body = resp.into_body();
            let pb = match body.content_length() {
                Some(len) => indicatif::ProgressBar::new(len).with_style(
                    indicatif::ProgressStyle::with_template(
                        "downloading {msg} [{elapsed_precise}] [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, ETA {eta})",
                    )
                    .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar())
                    .progress_chars("#>-"),
                ),
                None => indicatif::ProgressBar::new_spinner(),
            };
            pb.set_message(source.name.to_string());
            Box::new(BufReader::new(pb.wrap_read(body.into_reader())))
        }
    };

    let mut reader = reader;
    let entry = match zip::read::read_zipfile_from_stream(&mut reader)? {
        Some(file) => file,
        None => {
            return Err(BulkError::Zip(format!(
                "no entries found in {} archive",
                source.name
            )));
        }
    };

    let out_path = std::env::temp_dir().join(format!(
        "hardmoney-{}-{}.stage.csv",
        source.name,
        std::process::id()
    ));
    let mut out = std::io::BufWriter::new(File::create(&out_path)?);

    let lines = BufReader::new(entry).lines();
    let expected = source.columns.len();
    let mut written: u64 = 0;
    for (line_no, line) in (1u64..).zip(lines) {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let mut fields: Vec<&str> = line.split('|').collect();
        if fields.len() < expected {
            return Err(BulkError::MalformedRow {
                line_no,
                expected,
                found: fields.len(),
            });
        }
        if fields.len() > expected {
            if !source.allow_extra_trailing_fields {
                return Err(BulkError::MalformedRow {
                    line_no,
                    expected,
                    found: fields.len(),
                });
            }
            fields.truncate(expected);
        }

        for field in &fields {
            write_csv_field(&mut out, field)?;
            out.write_all(b"|")?;
        }
        writeln!(out, "{cycle}")?;
        written += 1;

        if let Some(limit) = limit
            && written >= limit
        {
            break;
        }
    }
    out.flush()?;

    Ok(StagedFile { path: out_path })
}

/// Writes `field` in Postgres `COPY ... (FORMAT csv)` form: bare unless it
/// contains the delimiter, a double quote, or a newline, in which case it
/// is wrapped in quotes with internal quotes doubled per RFC 4180. Real
/// FEC bulk rows essentially never need this, but a stray character in a
/// free-text field (a payee name, a memo) should never silently corrupt
/// column alignment.
fn write_csv_field(out: &mut impl Write, field: &str) -> std::io::Result<()> {
    if field.contains(['|', '"', '\n', '\r']) {
        write!(out, "\"{}\"", field.replace('"', "\"\""))
    } else {
        write!(out, "{field}")
    }
}

async fn copy_into_table(pool: &PgPool, source: &BulkSource, staged_path: &PathBuf) -> Result<u64> {
    let mut columns: Vec<&str> = source.columns.to_vec();
    columns.push("cycle");
    let column_list = columns.join(", ");
    let copy_sql = format!(
        "COPY {table} ({columns}) FROM STDIN (FORMAT csv, DELIMITER '|', NULL '')",
        table = source.table,
        columns = column_list
    );

    let mut copy_in = pool.copy_in_raw(&copy_sql).await?;
    let file = tokio::fs::File::open(staged_path).await?;
    copy_in.read_from(file).await?;
    let rows = copy_in.finish().await?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn written_field(field: &str) -> String {
        let mut buf: Vec<u8> = Vec::new();
        write_csv_field(&mut buf, field).unwrap();
        String::from_utf8(buf).unwrap()
    }

    #[test]
    fn plain_fields_are_written_bare() {
        // The overwhelmingly common case for real FEC bulk rows -- no
        // quoting overhead for a field that doesn't need it.
        assert_eq!(written_field("SMITH, JANE"), "SMITH, JANE");
        assert_eq!(written_field(""), "");
    }

    #[test]
    fn a_field_containing_the_pipe_delimiter_is_quoted() {
        // If this weren't quoted, a stray `|` in a free-text field (a
        // memo, a payee name) would silently shift every later column in
        // the row -- exactly the kind of corruption RFC 4180 quoting is
        // meant to prevent.
        assert_eq!(written_field("A|B"), "\"A|B\"");
    }

    #[test]
    fn an_embedded_double_quote_is_doubled_and_the_field_is_quoted() {
        assert_eq!(written_field(r#"say "hi"#), "\"say \"\"hi\"");
    }

    #[test]
    fn embedded_newlines_and_carriage_returns_are_quoted() {
        assert_eq!(written_field("line1\nline2"), "\"line1\nline2\"");
        assert_eq!(written_field("a\rb"), "\"a\rb\"");
    }

    #[test]
    fn a_field_needing_both_quote_doubling_and_pipe_quoting_handles_both() {
        assert_eq!(written_field(r#"a|"b"#), "\"a|\"\"b\"");
    }
}
