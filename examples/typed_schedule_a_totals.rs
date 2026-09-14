//! Common, very useful pattern: use the ergonomic typed views instead of
//! hand-parsing raw `IndexMap<String, String>` fields. `ScheduleA` gives
//! you a real `NaiveDate` and exact integer cents (never `f64`, which
//! cannot represent currency amounts exactly) for every itemized
//! contribution in a filing -- and a usable `contributor_name` regardless
//! of whether the filing is old-format (a single combined name field) or
//! current-format (split into organization/last/first/middle instead).
//!
//! Run with:
//!
//!     cargo run --example typed_schedule_a_totals -- path/to/filing.fec
//!
//! (defaults to a bundled real fixture if no path is given).

use hardmoney::{Filing, ScheduleA};

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tests/fixtures/F3A_2011812.fec".to_string());

    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("reading {path}: {e}"));
    let filing = Filing::parse_bytes(&bytes).unwrap_or_else(|e| panic!("parsing {path}: {e}"));

    let mut total_cents: i64 = 0;
    let mut count = 0usize;
    let mut largest: Option<ScheduleA> = None;

    for line in filing.lines.iter().filter(|l| l.table == "SchA") {
        // TryFrom<&ParsedLine> surfaces a TypedViewError instead of
        // panicking if a genuinely required field (the filer committee
        // id) is missing -- everything else degrades to `None` rather
        // than failing the whole line, since real-world FEC data is
        // routinely incomplete on optional fields.
        let sched_a: ScheduleA = match line.try_into() {
            Ok(row) => row,
            Err(e) => {
                eprintln!("skipping malformed Schedule A line: {e}");
                continue;
            }
        };

        if let Some(cents) = sched_a.contribution_amount_cents {
            total_cents += cents;
            count += 1;
            let current_max = largest
                .as_ref()
                .and_then(|l| l.contribution_amount_cents)
                .unwrap_or(0);
            if cents > current_max {
                largest = Some(sched_a);
            }
        }
    }

    println!("{count} itemized Schedule A contributions");
    println!("total: ${:.2}", total_cents as f64 / 100.0);
    if let Some(l) = largest {
        println!(
            "largest: ${:.2} from {} on {:?}",
            l.contribution_amount_cents.unwrap_or(0) as f64 / 100.0,
            l.contributor_name.as_deref().unwrap_or("(no name)"),
            l.contribution_date,
        );
    }
}
