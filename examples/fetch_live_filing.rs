//! Less common, but very useful: fetch a real filing directly from the
//! FEC's own document store by numeric filing id (the same id used in
//! fec.gov filing URLs), with no need to download a `.fec` file by hand
//! first. Requires the `fetch` feature (on by default).
//!
//! Run with:
//!
//!     cargo run --example fetch_live_filing -- 2011831
//!
//! (defaults to filing id 2011831, a real Form 3X, if none is given).
//!
//! Combines `Filing::fetch` with the `Form3XSummary` typed view to turn a
//! raw download straight into exact `Decimal` totals -- no `f64` anywhere
//! in the path from the FEC's own bytes to what's printed below.

use hardmoney::{Filing, Form3XSummary, Table};

fn main() {
    let filing_id: u64 = std::env::args()
        .nth(1)
        .map(|s| s.parse().expect("filing id must be a number"))
        .unwrap_or(2_011_831);

    println!("fetching filing {filing_id} from docquery.fec.gov ...");
    let filing = Filing::fetch(filing_id).unwrap_or_else(|e| panic!("fetching {filing_id}: {e}"));

    println!(
        "{} ({}), spec {}",
        filing.raw_form_type, filing.base_form_type, filing.version
    );

    if filing.summary.table == Table::F3X {
        match filing.summary.view::<Form3XSummary>() {
            Ok(summary) => {
                println!("committee:            {:?}", summary.committee_name);
                println!(
                    "receipts this period: ${}",
                    summary.total_receipts.unwrap_or_default()
                );
                println!(
                    "disbursements period: ${}",
                    summary.total_disbursements.unwrap_or_default()
                );
                println!(
                    "cash on hand (close): ${}",
                    summary.cash_on_hand_close_of_period.unwrap_or_default()
                );
            }
            Err(e) => eprintln!("could not build a Form3XSummary: {e}"),
        }
    } else {
        println!(
            "(not a Form 3X -- Form3XSummary only applies to that form; \
             every filing's raw fields are always available via `filing.summary`)"
        );
    }
}
