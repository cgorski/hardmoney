//! Most common use case: parse a `.fec` file and read its header/summary.
//!
//! Run with a bundled real fixture (no arguments needed):
//!
//!     cargo run --example parse_filing
//!
//! Or point it at any other `.fec` file on disk:
//!
//!     cargo run --example parse_filing -- path/to/filing.fec

use hardmoney::Filing;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tests/fixtures/F3XN_2011834.fec".to_string());

    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    // `parse_bytes` decodes UTF-8 (falling back to Windows-1252, which some
    // older filer-entered free text uses) before parsing -- prefer it over
    // `Filing::parse` unless you already have a `&str`.
    let filing = Filing::parse_bytes(&bytes).unwrap_or_else(|e| panic!("parsing {path}: {e}"));

    println!(
        "form:          {} (base: {})",
        filing.raw_form_type, filing.base_form_type
    );
    println!("spec version:  {}", filing.version);
    println!("is amendment:  {}", filing.is_amendment);
    if let Some(original) = &filing.amends_filing {
        println!("amends filing: {original}");
    }
    println!("body lines:    {}", filing.lines.len());

    // `summary` is the report's top-level cover/totals line -- a
    // `ParsedLine` whose `fields` are keyed by canonical field name.
    for field in [
        "committee_name",
        "col_a_total_receipts",
        "col_a_total_disbursements",
    ] {
        println!("summary.{field:<28} {:?}", filing.summary.get(field));
    }

    // Tally how many lines dispatched to each schedule/sub-form table.
    let mut by_table: std::collections::BTreeMap<hardmoney::Table, usize> =
        std::collections::BTreeMap::new();
    for line in &filing.lines {
        *by_table.entry(line.table).or_default() += 1;
    }
    println!("lines by table: {by_table:?}");
}
