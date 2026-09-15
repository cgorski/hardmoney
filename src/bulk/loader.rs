//! Downloads a FEC bulk `.zip`, decodes its single pipe-delimited text
//! entry, and bulk-loads it into Postgres via `COPY FROM STDIN`.
//!
//! The work happens in two phases so the slow, CPU/network-bound parts
//! (download + zip decode, both synchronous) never block the async
//! runtime the fast part (the actual `COPY`) needs:
//!
//! 1. **Stage** (blocking thread): stream the zip -- from a local path or
//!    straight from an HTTP response body, neither of which needs to be
//!    seekable -- into a local temporary CSV file, re-delimited for
//!    Postgres `COPY`, with parsed `DATE` twins for each raw date column
//!    and the `cycle` column appended to every row. A `limit` stops the
//!    zip decode *and* the underlying download early (see
//!    the `Abortable` reader), so sampling the first N rows of a multi-gigabyte
//!    source never requires downloading the whole file.
//! 2. **Load** (async, one transaction): optionally delete the existing
//!    rows for this cycle ([`LoadMode::Replace`]), `COPY` the staged file
//!    in, and record the load in `loads`. If anything fails the
//!    transaction rolls back and the previous data is untouched.
//!
//! # Re-running a load
//!
//! [`LoadMode::Replace`] is the default: the FEC re-publishes each cycle's
//! files weekly with rows added, changed, *and removed*, and a plain
//! upsert cannot remove the removed ones. Replace-in-a-transaction is the
//! only mode that reproduces the FEC's current file exactly.
//! [`LoadMode::Append`] is for genuine partial loads.

use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use sqlx::PgPool;

use super::error::{BulkError, Result};
use super::source::{BulkSource, DateColumn};
use crate::Cycle;

/// Where to read the zip archive from.
#[derive(Debug, Clone)]
pub enum Input {
    LocalFile(PathBuf),
    Url(String),
}

/// What to do with rows already present for this source + cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum LoadMode {
    /// Delete this cycle's existing rows, then COPY, in one transaction.
    /// Reproduces the FEC's current file exactly (including deletions).
    #[default]
    Replace,
    /// COPY on top of whatever is there. Fails with a duplicate-key error
    /// if any row already exists.
    Append,
}

impl LoadMode {
    pub fn as_str(self) -> &'static str {
        match self {
            LoadMode::Replace => "replace",
            LoadMode::Append => "append",
        }
    }
}

/// Everything that controls one load beyond the source and input.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct LoadOptions {
    pub cycle: Cycle,
    pub mode: LoadMode,
    /// Cap the number of data rows read. Loads with a limit are recorded
    /// in `loads` with `row_limit` set and never treated as "current".
    pub limit: Option<u64>,
    /// [`LoadMode::Replace`] deletes existing rows first. If more than
    /// this many would be deleted, refuse unless `confirmed` is set --
    /// a guard against nuking a production `schedule_a` by accident.
    pub confirm_threshold: i64,
    pub confirmed: bool,
    /// Skip the load entirely if the remote `ETag`/`Last-Modified`
    /// matches the most recent full load of this source + cycle in
    /// `loads`. Only meaningful for [`Input::Url`].
    pub if_changed: bool,
}

impl LoadOptions {
    pub fn new(cycle: Cycle) -> Self {
        Self {
            cycle,
            mode: LoadMode::Replace,
            limit: None,
            confirm_threshold: 1_000_000,
            confirmed: false,
            if_changed: false,
        }
    }

    pub fn mode(mut self, mode: LoadMode) -> Self {
        self.mode = mode;
        self
    }

    pub fn limit(mut self, limit: Option<u64>) -> Self {
        self.limit = limit;
        self
    }

    pub fn confirmed(mut self, yes: bool) -> Self {
        self.confirmed = yes;
        self
    }

    pub fn if_changed(mut self, yes: bool) -> Self {
        self.if_changed = yes;
        self
    }
}

/// Outcome of staging + loading one bulk source.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LoadReport {
    pub table: &'static str,
    pub cycle: Cycle,
    pub mode: LoadMode,
    pub rows_loaded: u64,
    /// Rows deleted before the COPY (always 0 for [`LoadMode::Append`]).
    pub rows_replaced: u64,
    /// Date cells that were blank/zero/unparseable and became NULL in the
    /// parsed `*_date` column (the raw `*_dt` text is always preserved).
    pub dates_nulled: u64,
    /// True if `if_changed` was set and the remote file was unchanged;
    /// nothing was loaded.
    pub skipped_unchanged: bool,
    pub load_id: Option<i64>,
}

/// Downloads/opens `input`, decodes `source`'s single entry, and loads it
/// into `source.table` tagged with `options.cycle`.
pub async fn load(
    pool: &PgPool,
    source: &'static BulkSource,
    input: Input,
    options: LoadOptions,
) -> Result<LoadReport> {
    let status = crate::db::migration_status(pool).await?;
    if !status.is_current() {
        return Err(BulkError::SchemaOutOfDate {
            pending: status.pending.len(),
        });
    }

    // `--if-changed`: one HEAD request against the last recorded full load.
    let remote_meta = match &input {
        Input::Url(url) => head_metadata(url),
        Input::LocalFile(_) => RemoteMeta::default(),
    };
    if options.if_changed
        && let Input::Url(_) = &input
        && let Some(prev) = last_full_load(pool, source, options.cycle).await?
        && remote_meta.matches(&prev)
    {
        return Ok(LoadReport {
            table: source.table,
            cycle: options.cycle,
            mode: options.mode,
            rows_loaded: 0,
            rows_replaced: 0,
            dates_nulled: 0,
            skipped_unchanged: true,
            load_id: None,
        });
    }

    // Replace-mode guard runs before the (possibly multi-GB) download.
    if options.mode == LoadMode::Replace && !options.confirmed {
        let existing = count_rows(pool, source, options.cycle).await?;
        if existing > options.confirm_threshold {
            return Err(BulkError::ConfirmationRequired {
                table: source.table,
                cycle: i32::from(options.cycle),
                existing,
            });
        }
    }

    let cycle = options.cycle;
    let limit = options.limit;
    let source_url = match &input {
        Input::Url(u) => Some(u.clone()),
        Input::LocalFile(_) => None,
    };
    let staged =
        tokio::task::spawn_blocking(move || stage_to_temp_csv(source, input, cycle, limit))
            .await??;
    // Whatever happens below, don't leave the temp file behind.
    let _cleanup = RemoveOnDrop(staged.path.clone());

    let mut tx = pool.begin().await?;

    let rows_replaced = if options.mode == LoadMode::Replace {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DELETE FROM {} WHERE cycle = $1",
            source.table
        )))
        .bind(i32::from(options.cycle))
        .execute(&mut *tx)
        .await?
        .rows_affected()
    } else {
        0
    };

    let rows_loaded = copy_into_table(&mut tx, source, &staged.path).await?;

    let load_id: i64 = sqlx::query_scalar(
        "INSERT INTO loads (source, cycle, mode, row_count, row_limit, dates_nulled, \
                            source_url, source_etag, source_last_modified, hardmoney_version) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) RETURNING load_id",
    )
    .bind(source.name)
    .bind(i32::from(options.cycle))
    .bind(options.mode.as_str())
    .bind(i64::try_from(rows_loaded).unwrap_or(i64::MAX))
    .bind(options.limit.map(|l| i64::try_from(l).unwrap_or(i64::MAX)))
    .bind(i64::try_from(staged.dates_nulled).unwrap_or(i64::MAX))
    .bind(source_url)
    .bind(&remote_meta.etag)
    .bind(remote_meta.last_modified)
    .bind(env!("CARGO_PKG_VERSION"))
    .fetch_one(&mut *tx)
    .await?;

    tx.commit().await?;

    Ok(LoadReport {
        table: source.table,
        cycle: options.cycle,
        mode: options.mode,
        rows_loaded,
        rows_replaced,
        dates_nulled: staged.dates_nulled,
        skipped_unchanged: false,
        load_id: Some(load_id),
    })
}

async fn count_rows(pool: &PgPool, source: &BulkSource, cycle: Cycle) -> Result<i64> {
    let n: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "SELECT count(*) FROM {} WHERE cycle = $1",
        source.table
    )))
    .bind(i32::from(cycle))
    .fetch_one(pool)
    .await?;
    Ok(n)
}

/// ETag / Last-Modified of the most recent successful *full* load.
async fn last_full_load(
    pool: &PgPool,
    source: &BulkSource,
    cycle: Cycle,
) -> Result<Option<RemoteMeta>> {
    let row: Option<(Option<String>, Option<chrono::DateTime<chrono::Utc>>)> = sqlx::query_as(
        "SELECT source_etag, source_last_modified FROM loads \
         WHERE source = $1 AND cycle = $2 AND row_limit IS NULL \
         ORDER BY loaded_at DESC LIMIT 1",
    )
    .bind(source.name)
    .bind(i32::from(cycle))
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(etag, last_modified)| RemoteMeta {
        etag,
        last_modified,
    }))
}

/// What the FEC's S3-backed server tells us about a bulk file without
/// downloading it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct RemoteMeta {
    etag: Option<String>,
    last_modified: Option<chrono::DateTime<chrono::Utc>>,
}

impl RemoteMeta {
    /// Unchanged if the ETag matches, or (no ETag) Last-Modified matches.
    /// If the server gave us nothing, we can't tell, so assume changed.
    fn matches(&self, prev: &RemoteMeta) -> bool {
        match (&self.etag, &prev.etag) {
            (Some(a), Some(b)) => a == b,
            _ => match (self.last_modified, prev.last_modified) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    }
}

fn head_metadata(url: &str) -> RemoteMeta {
    let Ok(resp) = ureq::head(url).call() else {
        return RemoteMeta::default();
    };
    let header = |name: &str| resp.headers().get(name).and_then(|v| v.to_str().ok());
    let etag = header("etag").map(|s| s.trim_matches('"').to_string());
    let last_modified = header("last-modified")
        .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
        .map(|d| d.with_timezone(&chrono::Utc));
    RemoteMeta {
        etag,
        last_modified,
    }
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct StagedFile {
    path: PathBuf,
    dates_nulled: u64,
}

/// A reader that can be told to report EOF immediately.
///
/// `zip`'s streaming `ZipFile` drains the remainder of its entry on `Drop`
/// to leave the underlying stream positioned at the next entry. For a
/// multi-gigabyte entry behind an HTTP body that means `--limit 1000` still
/// downloads the whole file. Flipping `aborted` makes every subsequent
/// `read` return `Ok(0)`, so the drain finishes instantly and the HTTP
/// connection is dropped.
struct Abortable<R> {
    inner: R,
    aborted: Arc<AtomicBool>,
}

impl<R: Read> Read for Abortable<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.aborted.load(Ordering::Relaxed) {
            return Ok(0);
        }
        self.inner.read(buf)
    }
}

/// Blocking: reads the zip (local file or streamed HTTP body), decodes the
/// first entry, normalizes each row, and writes to a fresh temp file. Runs
/// entirely off the async runtime.
fn stage_to_temp_csv(
    source: &BulkSource,
    input: Input,
    cycle: Cycle,
    limit: Option<u64>,
) -> Result<StagedFile> {
    let raw_reader: Box<dyn Read> = match input {
        Input::LocalFile(path) => Box::new(BufReader::new(File::open(path)?)),
        Input::Url(url) => {
            let resp = ureq::get(&url).call().map_err(|e| BulkError::Http {
                url: url.clone(),
                detail: e.to_string(),
            })?;
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

    let aborted = Arc::new(AtomicBool::new(false));
    let mut reader = Abortable {
        inner: raw_reader,
        aborted: Arc::clone(&aborted),
    };
    let entry =
        zip::read::read_zipfile_from_stream(&mut reader)?.ok_or(BulkError::EmptyArchive {
            source_name: source.name,
        })?;

    // Unique per process *and* per call, so two concurrent loads of the
    // same source (e.g. different cycles) never share a staging file.
    static STAGE_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let out_path = std::env::temp_dir().join(format!(
        "hardmoney-{}-{}-{}-{}.stage.csv",
        source.name,
        cycle,
        std::process::id(),
        STAGE_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let mut out = std::io::BufWriter::new(File::create(&out_path)?);

    // Column positions of each raw date column, resolved once.
    let date_positions: Vec<(usize, DateColumn)> = source
        .date_columns
        .iter()
        .filter_map(|dc| {
            source
                .columns
                .iter()
                .position(|c| *c == dc.raw)
                .map(|i| (i, *dc))
        })
        .collect();

    let expected = source.columns.len();
    let mut written: u64 = 0;
    let mut dates_nulled: u64 = 0;
    let mut line_no: u64 = 0;

    // Read as bytes and decode leniently: the FEC's bulk files are ASCII
    // in practice, but a single stray Windows-1252 byte in a memo field
    // must not fail a 2 GB load (the `.fec` parser is equally tolerant).
    let mut lines = BufReader::new(entry);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        buf.clear();
        if lines.read_until(b'\n', &mut buf)? == 0 {
            break;
        }
        line_no += 1;
        let line = decode_line(&buf);
        let line = line.trim_end_matches(['\r', '\n']);
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
        for (pos, _) in &date_positions {
            match fields.get(*pos).and_then(|raw| normalize_bulk_date(raw)) {
                Some(iso) => write!(out, "{iso}")?,
                None => {
                    if fields.get(*pos).is_some_and(|raw| !raw.trim().is_empty()) {
                        dates_nulled += 1;
                    }
                }
            }
            out.write_all(b"|")?;
        }
        writeln!(out, "{cycle}")?;
        written += 1;

        if let Some(limit) = limit
            && written >= limit
        {
            aborted.store(true, Ordering::Relaxed);
            break;
        }
    }
    out.flush()?;
    // `entry` drops here; with `aborted` set its drain sees EOF at once.
    drop(lines);

    Ok(StagedFile {
        path: out_path,
        dates_nulled,
    })
}

fn decode_line(bytes: &[u8]) -> std::borrow::Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(s) => std::borrow::Cow::Borrowed(s),
        Err(_) => {
            let (decoded, _, _) = encoding_rs::WINDOWS_1252.decode(bytes);
            decoded
        }
    }
}

/// Parses the two date formats the FEC ships in bulk files -- `MMDDYYYY`
/// (`indiv`, `oth`, `pas2`) and `MM/DD/YYYY` (`oppexp`, `weball`, `webk`)
/// -- into ISO `YYYY-MM-DD` for `COPY`. Blank, all-zero, and calendar-
/// invalid values return `None` (the raw text column keeps the original).
pub fn normalize_bulk_date(raw: &str) -> Option<String> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    let (m, d, y) = match t.as_bytes() {
        // MMDDYYYY
        [m1, m2, d1, d2, y @ ..] if y.len() == 4 && t.bytes().all(|b| b.is_ascii_digit()) => (
            std::str::from_utf8(&[*m1, *m2]).ok()?.to_string(),
            std::str::from_utf8(&[*d1, *d2]).ok()?.to_string(),
            std::str::from_utf8(y).ok()?.to_string(),
        ),
        // MM/DD/YYYY
        [m1, m2, b'/', d1, d2, b'/', y @ ..] if y.len() == 4 => (
            std::str::from_utf8(&[*m1, *m2]).ok()?.to_string(),
            std::str::from_utf8(&[*d1, *d2]).ok()?.to_string(),
            std::str::from_utf8(y).ok()?.to_string(),
        ),
        _ => return None,
    };
    let (m, d, y): (u32, u32, i32) = (m.parse().ok()?, d.parse().ok()?, y.parse().ok()?);
    if y == 0 {
        return None;
    }
    chrono::NaiveDate::from_ymd_opt(y, m, d).map(|dt| dt.format("%Y-%m-%d").to_string())
}

/// Writes `field` in Postgres `COPY ... (FORMAT csv)` form: bare unless it
/// contains the delimiter, a double quote, or a newline, in which case it
/// is wrapped in quotes with internal quotes doubled per RFC 4180.
fn write_csv_field(out: &mut impl Write, field: &str) -> std::io::Result<()> {
    if field.contains(['|', '"', '\n', '\r']) {
        write!(out, "\"{}\"", field.replace('"', "\"\""))
    } else {
        write!(out, "{field}")
    }
}

async fn copy_into_table(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    source: &BulkSource,
    staged_path: &PathBuf,
) -> Result<u64> {
    let mut columns: Vec<&str> = source.columns.to_vec();
    columns.extend(source.date_columns.iter().map(|dc| dc.parsed));
    columns.push("cycle");
    let copy_sql = format!(
        "COPY {table} ({columns}) FROM STDIN (FORMAT csv, DELIMITER '|', NULL '')",
        table = source.table,
        columns = columns.join(", ")
    );

    let mut copy_in = tx.copy_in_raw(&copy_sql).await?;
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
        assert_eq!(written_field("SMITH, JANE"), "SMITH, JANE");
        assert_eq!(written_field(""), "");
    }

    #[test]
    fn a_field_containing_the_pipe_delimiter_is_quoted() {
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
    fn normalizes_both_fec_date_formats() {
        assert_eq!(
            normalize_bulk_date("12312024").as_deref(),
            Some("2024-12-31")
        );
        assert_eq!(
            normalize_bulk_date("01/15/2025").as_deref(),
            Some("2025-01-15")
        );
        assert_eq!(
            normalize_bulk_date(" 09302025 ").as_deref(),
            Some("2025-09-30")
        );
    }

    #[test]
    fn rejects_blank_zero_and_invalid_dates_without_panicking() {
        for bad in [
            "",
            " ",
            "00000000",
            "13452026",
            "02302025",
            "2024-12-31",
            "1231202",
            "12/31/24",
            "ab/cd/efgh",
            "99999999",
            "00/00/0000",
        ] {
            assert_eq!(normalize_bulk_date(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn decode_line_tolerates_windows_1252() {
        assert_eq!(decode_line(b"SMITH|JANE"), "SMITH|JANE");
        // 0xE9 is 'é' in Windows-1252 and invalid as a lone UTF-8 byte.
        assert_eq!(decode_line(b"CAF\xC9|X"), "CAF\u{c9}|X");
    }

    #[test]
    fn abortable_reader_reports_eof_once_aborted() {
        let aborted = Arc::new(AtomicBool::new(false));
        let mut r = Abortable {
            inner: std::io::Cursor::new(vec![1u8; 1000]),
            aborted: Arc::clone(&aborted),
        };
        let mut buf = [0u8; 10];
        assert_eq!(r.read(&mut buf).unwrap(), 10);
        aborted.store(true, Ordering::Relaxed);
        assert_eq!(r.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn remote_meta_matching() {
        let a = RemoteMeta {
            etag: Some("x".into()),
            last_modified: None,
        };
        let b = RemoteMeta {
            etag: Some("x".into()),
            last_modified: None,
        };
        assert!(a.matches(&b));
        assert!(!RemoteMeta::default().matches(&RemoteMeta::default()));
        let c = RemoteMeta {
            etag: Some("y".into()),
            last_modified: None,
        };
        assert!(!a.matches(&c));
    }

    #[test]
    fn stages_a_local_zip_with_dates_limit_and_extra_fields() {
        // Build a tiny `oppexp`-shaped zip in memory: 25 columns + the
        // known trailing extra field, MM/DD/YYYY dates, one blank date,
        // one bogus date, four rows, limit 3.
        use std::io::Write as _;
        let src = &super::super::source::DISBURSEMENTS;
        let mk = |sub_id: &str, dt: &str| {
            let mut f: Vec<String> = (0..src.columns.len()).map(|_| String::new()).collect();
            let pos = |c: &str| src.columns.iter().position(|x| *x == c).unwrap();
            f[pos("cmte_id")] = "C00000001".into();
            f[pos("transaction_dt")] = dt.into();
            f[pos("transaction_amt")] = "12.50".into();
            f[pos("sub_id")] = sub_id.into();
            f.push(String::new()); // the FEC's undocumented trailing field
            f.join("|")
        };
        let body = [
            mk("1", "01/15/2025"),
            mk("2", ""),
            mk("3", "13/45/2025"),
            mk("4", "02/01/2025"),
        ]
        .join("\n");
        let mut zip_bytes = std::io::Cursor::new(Vec::new());
        {
            let mut zw = zip::ZipWriter::new(&mut zip_bytes);
            zw.start_file("oppexp.txt", zip::write::SimpleFileOptions::default())
                .unwrap();
            zw.write_all(body.as_bytes()).unwrap();
            zw.finish().unwrap();
        }
        let dir = std::env::temp_dir().join(format!("hardmoney-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let zip_path = dir.join("oppexp_test.zip");
        std::fs::write(&zip_path, zip_bytes.into_inner()).unwrap();

        let staged = stage_to_temp_csv(
            src,
            Input::LocalFile(zip_path.clone()),
            Cycle::new(2026).unwrap(),
            Some(3),
        )
        .unwrap();
        let out = std::fs::read_to_string(&staged.path).unwrap();
        let rows: Vec<&str> = out.lines().collect();
        assert_eq!(rows.len(), 3, "{out}");
        // 25 source cols + 1 date col + cycle = 27 fields => 26 pipes.
        for r in &rows {
            assert_eq!(r.matches('|').count(), 26, "{r}");
            assert!(r.ends_with("|2026"), "{r}");
        }
        assert!(rows[0].ends_with("|2025-01-15|2026"), "{}", rows[0]);
        assert!(rows[1].ends_with("||2026"), "{}", rows[1]); // blank -> NULL, not counted
        assert!(rows[2].ends_with("||2026"), "{}", rows[2]); // bogus -> NULL, counted
        assert_eq!(staged.dates_nulled, 1);

        let _ = std::fs::remove_file(&staged.path);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
