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
//! raw download straight into cents-accurate totals.

use hardmoney::{Filing, Form3XSummary};

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

    if filing.base_form_type == "F3X" {
        match Form3XSummary::try_from(&filing.summary) {
            Ok(summary) => {
                println!("committee:            {:?}", summary.committee_name);
                println!(
                    "receipts this period: {:?} cents",
                    summary.total_receipts_cents
                );
                println!(
                    "disbursements period: {:?} cents",
                    summary.total_disbursements_cents
                );
                println!(
                    "cash on hand (close): {:?} cents",
                    summary.cash_on_hand_close_of_period_cents
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
