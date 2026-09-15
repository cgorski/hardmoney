//! `hardmoney reconcile`: recompute every cover-page line of a report from
//! its schedules and formulas, and show where the filing disagrees with
//! itself. With `--with-prior`, also check Column B against the schedules
//! of the committee's earlier reports and carry cash on hand forward.

use std::path::{Path, PathBuf};

use clap::Args;
use hardmoney::parser::reconcile::{Column, LineCheck, ReportChain};
use hardmoney::{Filing, ParseOptions};
use rust_decimal::Decimal;

#[derive(Args, Debug)]
pub struct ReconcileArgs {
    /// Path to a `.fec` file (Form 3X, 3, or 3P).
    pub path: PathBuf,

    /// The same committee's earlier reports on the same form, in any
    /// order. Adds Column B checks that sum each schedule line over the
    /// chain (the calendar year for Form 3X, the election cycle for Forms 3
    /// and 3P) and cash-on-hand carry-forward checks. Give every report of
    /// the period; a gap is reported on stderr. An original and its
    /// amendment cannot both be given.
    #[arg(long, value_name = "FILE", num_args = 1..)]
    pub with_prior: Vec<PathBuf>,

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

/// Reads and parses one file with the chosen strictness, warning on stderr
/// about skipped lines (they are excluded from every schedule sum).
fn load(path: &Path, options: &ParseOptions) -> super::CliResult<Filing> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let (filing, skipped) = Filing::parse_bytes_with(&bytes, options)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .into_parts();
    if !skipped.is_empty() {
        eprintln!(
            "warning: {}: {} line(s) skipped and excluded from schedule sums",
            path.display(),
            skipped.len()
        );
    }
    Ok(filing)
}

pub fn run(args: ReconcileArgs) -> super::CliResult {
    let options = if args.lenient {
        ParseOptions::LENIENT
    } else {
        ParseOptions::STRICT
    };
    if args.tolerance.is_sign_negative() {
        return Err("--tolerance must be >= 0".into());
    }
    let filing = load(&args.path, &options)?;
    let priors: Vec<Filing> = args
        .with_prior
        .iter()
        .map(|p| load(p, &options))
        .collect::<Result<_, _>>()?;

    let reconciliation = filing.reconcile()?;
    let mut checks: Vec<LineCheck> = reconciliation.checks.clone();

    // The chain adds Column B schedule sums and the cash carry-forward.
    let chain = if args.with_prior.is_empty() {
        None
    } else {
        let chain = ReportChain::new(&filing, priors.iter())?;
        for gap in chain.gaps() {
            eprintln!(
                "warning: no report in the chain covers {gap}; Column B sums are short by that \
                 period's activity"
            );
        }
        checks.extend(chain.reconcile().checks);
        Some(chain)
    };

    let wanted: Vec<&LineCheck> = checks
        .iter()
        .filter(|c| args.column.is_none_or(|col| c.column == Column::from(col)))
        .collect();
    let disagreeing: Vec<&LineCheck> = wanted
        .iter()
        .copied()
        .filter(|c| c.violation() > args.tolerance)
        .collect();

    if args.json {
        let mut out = serde_json::json!({
            "form": reconciliation.form,
            "file": args.path,
            "tolerance": args.tolerance,
            "checks": wanted.len(),
            "disagreeing": disagreeing.len(),
            "lines": if args.all { &wanted } else { &disagreeing },
        });
        if let (Some(chain), serde_json::Value::Object(map)) = (&chain, &mut out) {
            map.insert(
                "chain".to_string(),
                serde_json::json!({
                    "reports": chain.prior().len() + 1,
                    "reports_summed": chain.period_reports().len(),
                    "basis": chain.basis(),
                    "gaps": chain.gaps(),
                }),
            );
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        for c in if args.all { &wanted } else { &disagreeing } {
            println!("{c}");
        }
        let chain_note = chain.as_ref().map_or_else(String::new, |chain| {
            format!(
                "; chain of {} report(s), {} summed ({})",
                chain.prior().len() + 1,
                chain.period_reports().len(),
                chain.basis()
            )
        });
        println!(
            "{} {}: {} of {} line(s) disagree (tolerance {}){chain_note}",
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
