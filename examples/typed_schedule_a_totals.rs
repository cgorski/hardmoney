//! Common, very useful pattern: use the ergonomic typed views instead of
//! hand-parsing raw string fields. `Filing::views::<ScheduleA>()`
//! yields only genuine Schedule A lines (the table is checked, so a Schedule
//! B line can never masquerade as a contribution) and each `ScheduleA` gives
//! you a real `NaiveDate` and an exact `rust_decimal::Decimal` (never
//! `f64`, which cannot represent every decimal currency amount exactly)
//! for every itemized contribution in a filing -- and a usable
//! `contributor_name` regardless of whether the filing is old-format (a
//! single combined name field) or current-format (split into
//! organization/last/first/middle instead).
//!
//! Run with:
//!
//!     cargo run --example typed_schedule_a_totals -- path/to/filing.fec
//!
//! (defaults to a bundled real fixture if no path is given).

use hardmoney::{Filing, ScheduleA};
use rust_decimal::Decimal;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tests/fixtures/F3A_2011812.fec".to_string());

    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    let filing = Filing::parse_bytes(&bytes).unwrap_or_else(|e| panic!("parsing {path}: {e}"));

    let mut total = Decimal::ZERO;
    let mut count = 0usize;
    let mut largest: Option<ScheduleA> = None;

    // `views::<ScheduleA>()` filters to `Table::SchA` lines and converts
    // each one. A line is only dropped if a genuinely required field (the
    // filer committee id) is blank -- everything else degrades to `None`,
    // since real-world FEC data is routinely incomplete on optional
    // fields. To see *why* a line was dropped, use `line.view::<ScheduleA>()`
    // on `filing.lines_for(Table::SchA)` instead.
    for sched_a in filing.views::<ScheduleA>() {
        if let Some(amount) = sched_a.contribution_amount {
            // Plain `Decimal` addition -- exact, with no accumulated
            // rounding drift no matter how many lines this loop sums.
            total += amount;
            count += 1;
            let current_max = largest
                .as_ref()
                .and_then(|l| l.contribution_amount)
                .unwrap_or(Decimal::ZERO);
            if amount > current_max {
                largest = Some(sched_a);
            }
        }
    }

    println!("{count} itemized Schedule A contributions");
    // `Decimal`'s `Display` already prints exactly two decimal places for
    // a value built from cents, so no `{:.2}` formatting is needed --
    // unlike `f64`, there's no precision to lose or round away here.
    println!("total: ${total}");
    if let Some(l) = largest {
        println!(
            "largest: ${} from {} on {:?}",
            l.contribution_amount.unwrap_or(Decimal::ZERO),
            l.contributor_name.as_deref().unwrap_or("(no name)"),
            l.contribution_date,
        );
    }
}
