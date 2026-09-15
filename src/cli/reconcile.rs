//! `hardmoney reconcile`: recompute every cover-page line of a report from
//! its schedules and formulas, and show where the filing disagrees with
//! itself.

use std::path::PathBuf;

use clap::Args;
use hardmoney::parser::reconcile::Column;
use hardmoney::{Filing, ParseOptions};
use rust_decimal::Decimal;

#[derive(Args, Debug)]
pub struct ReconcileArgs {
    /// Path to a `.fec` file (Form 3X, 3, or 3P).
    pub path: PathBuf,

    /// Emit the full check list as JSON.
    #[arg(long)]
    pub json: bool,

    /// Show every line, not just the ones that disagree.
    #[arg(long)]
    pub all: bool,

    /// Treat violations up to this amount as agreeing (e.g. 0.01 for a
    /// filer who rounds each line). A violation is the absolute delta for
    /// an `=` line, or the shortfall below the itemized sum for a `>=` line.
    #[arg(long, default_value = "0")]
    pub tolerance: Decimal,

    /// Only check this column (A = this period, B = year/cycle to date).
    #[arg(long, value_enum)]
    pub column: Option<ColumnArg>,

    /// Skip body lines that cannot be parsed instead of failing.
    #[arg(long)]
    pub lenient: bool,
}

#[derive(clap::ValueEnum, Clone, Copy, Debug)]
pub enum ColumnArg {
    A,
    B,
}

impl From<ColumnArg> for Column {
    fn from(c: ColumnArg) -> Self {
        match c {
            ColumnArg::A => Column::A,
            ColumnArg::B => Column::B,
        }
    }
}

pub fn run(args: ReconcileArgs) -> super::CliResult {
    let bytes = std::fs::read(&args.path)?;
    let options = if args.lenient {
        ParseOptions::LENIENT
    } else {
        ParseOptions::STRICT
    };
    let (filing, skipped) = Filing::parse_bytes_with(&bytes, &options)?.into_parts();
    if !skipped.is_empty() {
        eprintln!(
            "warning: {} line(s) skipped and excluded from schedule sums",
            skipped.len()
        );
    }
    if args.tolerance.is_sign_negative() {
        return Err("--tolerance must be >= 0".into());
    }

    let reconciliation = filing.reconcile()?;
    let wanted: Vec<_> = reconciliation
        .checks
        .iter()
        .filter(|c| args.column.is_none_or(|col| c.column == Column::from(col)))
        .collect();
    let disagreeing: Vec<_> = wanted
        .iter()
        .copied()
        .filter(|c| c.violation() > args.tolerance)
        .collect();

    if args.json {
        let out = serde_json::json!({
            "form": reconciliation.form,
            "file": args.path,
            "tolerance": args.tolerance,
            "checks": wanted.len(),
            "disagreeing": disagreeing.len(),
            "lines": if args.all { &wanted } else { &disagreeing },
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        for c in if args.all { &wanted } else { &disagreeing } {
            println!("{c}");
        }
        println!(
            "{} {}: {} of {} line(s) disagree (tolerance {})",
            reconciliation.form,
            filing
                .summary
                .get_non_empty("filer_committee_id_number")
                .unwrap_or("?"),
            disagreeing.len(),
            wanted.len(),
            args.tolerance
        );
    }
    if disagreeing.is_empty() {
        Ok(())
    } else {
        Err(format!("{} line(s) disagree", disagreeing.len()).into())
    }
}
