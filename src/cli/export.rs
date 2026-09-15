//! `hardmoney export`: write a `.fec` filing's records out as tabular data
//! -- CSV, JSON Lines, Parquet, or SQLite -- one table per schedule/form,
//! streamed through `FilingReader` so a 135 MB filing exports in constant
//! memory. See [`hardmoney::export`] for the column set and type mapping.

use std::path::PathBuf;

use clap::Args;
use hardmoney::Table;
use hardmoney::export::{ExportOptions, ExportReport, Format, export_path, resolve_table};

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Path to a `.fec` file.
    pub path: PathBuf,

    /// Output format. csv/jsonl/parquet write one `<Table>.<ext>` file per
    /// table into a directory; sqlite writes one database file.
    #[arg(long, short, value_enum, default_value_t = Format::Csv)]
    pub format: Format,

    /// Output directory (csv/jsonl/parquet) or database file (sqlite).
    /// Created if missing; existing per-table files are replaced, existing
    /// SQLite tables are appended to. [default: ./<file-stem>.<format>]
    #[arg(long, short)]
    pub out: Option<PathBuf>,

    /// Export only these tables, comma-separated: table names (SchA, F3X,
    /// TEXT; case-insensitive) or upper-case form-type tokens (SA, SB21B,
    /// SE). The cover line is included only if its table is listed.
    #[arg(long, value_delimiter = ',', value_parser = parse_only_token)]
    pub only: Option<Vec<Table>>,

    /// Prepend a `filing_id` column, taken from the file name (the last run
    /// of 4+ digits in its stem, e.g. 2011821 for F3XA_2011821.fec). Fails
    /// if the name has none.
    #[arg(long)]
    pub include_filing_id: bool,

    /// Skip body lines that cannot be parsed (unknown form type, or no
    /// column layout for this spec version) instead of failing; the count
    /// is reported on stderr.
    #[arg(long)]
    pub lenient: bool,
}

fn parse_only_token(token: &str) -> Result<Table, String> {
    resolve_table(token).map_err(|e| e.to_string())
}

pub fn run(args: ExportArgs) -> super::CliResult {
    let mut opts = ExportOptions::new(args.format).lenient(args.lenient);
    if let Some(only) = args.only {
        opts = opts.only(only);
    }
    if args.include_filing_id {
        opts = opts.include_filing_id_from(&args.path)?;
    }
    let out = args
        .out
        .unwrap_or_else(|| args.format.default_out(&args.path));

    let report = export_path(&args.path, &out, &opts)?;

    if report.skipped > 0 {
        eprintln!(
            "warning: {} unparseable line(s) skipped (--lenient)",
            report.skipped
        );
    }
    print!("{}", summary(&report, args.format, &out));
    Ok(())
}

/// The end-of-run summary: one line per table, then totals and the output
/// path. Bytes per table are shown only for the per-table-file formats.
fn summary(report: &ExportReport, format: Format, out: &std::path::Path) -> String {
    use std::fmt::Write as _;

    let per_file = format.writes_directory();
    let name_w = report
        .tables
        .iter()
        .map(|t| t.table.as_str().len())
        .chain(std::iter::once("table".len()))
        .max()
        .unwrap_or(5);
    let rows_w = report
        .tables
        .iter()
        .map(|t| group_thousands(t.rows).len())
        .chain(std::iter::once("rows".len()))
        .max()
        .unwrap_or(4);
    let bytes_w = report
        .tables
        .iter()
        .map(|t| group_thousands(t.bytes.unwrap_or(0)).len())
        .chain(std::iter::once("bytes".len()))
        .max()
        .unwrap_or(5);

    let mut s = String::new();
    if per_file {
        let _ = writeln!(
            s,
            "{:<name_w$}  {:>rows_w$}  {:>bytes_w$}",
            "table", "rows", "bytes"
        );
        let _ = writeln!(
            s,
            "{}  {}  {}",
            "-".repeat(name_w),
            "-".repeat(rows_w),
            "-".repeat(bytes_w)
        );
    } else {
        let _ = writeln!(s, "{:<name_w$}  {:>rows_w$}", "table", "rows");
        let _ = writeln!(s, "{}  {}", "-".repeat(name_w), "-".repeat(rows_w));
    }
    for t in &report.tables {
        if per_file {
            let _ = writeln!(
                s,
                "{:<name_w$}  {:>rows_w$}  {:>bytes_w$}",
                t.table.as_str(),
                group_thousands(t.rows),
                group_thousands(t.bytes.unwrap_or(0))
            );
        } else {
            let _ = writeln!(
                s,
                "{:<name_w$}  {:>rows_w$}",
                t.table.as_str(),
                group_thousands(t.rows)
            );
        }
    }
    let _ = writeln!(
        s,
        "{} table(s), {} row(s), {} bytes -> {}{}",
        report.tables.len(),
        group_thousands(report.rows()),
        group_thousands(report.bytes),
        out.display(),
        if per_file { "/" } else { "" }
    );
    s
}

/// `1234567` -> `1,234,567`.
fn group_thousands(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use hardmoney::export::TableStats;

    #[test]
    fn thousands_are_grouped() {
        assert_eq!(group_thousands(0), "0");
        assert_eq!(group_thousands(999), "999");
        assert_eq!(group_thousands(1000), "1,000");
        assert_eq!(group_thousands(1234567), "1,234,567");
    }

    #[test]
    fn only_tokens_resolve_or_explain() {
        assert_eq!(parse_only_token("SA"), Ok(Table::SchA));
        assert_eq!(parse_only_token("schb"), Ok(Table::SchB));
        let err = parse_only_token("ScheA").unwrap_err();
        assert!(err.contains("'ScheA' is not a table name"), "{err}");
    }

    fn report() -> ExportReport {
        let mut r = ExportReport::default();
        r.tables = vec![
            TableStats::new(Table::F3X, 1, Some(1200)),
            TableStats::new(Table::SchA, 12345, Some(2_000_000)),
        ];
        r.bytes = 2_001_200;
        r
    }

    #[test]
    fn summary_shows_bytes_per_file_only_for_directory_formats() {
        let dir = summary(&report(), Format::Csv, std::path::Path::new("out.csv"));
        assert!(dir.starts_with("table  "), "{dir}");
        assert!(dir.contains("bytes"), "{dir}");
        assert!(dir.contains("SchA   12,345  2,000,000"), "{dir}");
        assert!(
            dir.ends_with("2 table(s), 12,346 row(s), 2,001,200 bytes -> out.csv/\n"),
            "{dir}"
        );

        let db = summary(
            &report(),
            Format::Sqlite,
            std::path::Path::new("out.sqlite"),
        );
        assert!(!db.lines().next().unwrap_or("").contains("bytes"), "{db}");
        assert!(db.ends_with("-> out.sqlite\n"), "{db}");
    }
}
