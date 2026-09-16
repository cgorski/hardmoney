//! `hardmoney validate`: check one `.fec` file against the FEC's
//! acceptance rules and print the findings, WebCheck-style.
//!
//! Exit status is 1 if the filing has any error-severity finding (the FEC
//! would reject it) -- or, with `--strict-warnings`, any finding at all --
//! and 0 otherwise. Findings go to stdout; parse failures go to stderr.
//!
//! With `--oracle webcheck` the file is also submitted to the FEC's own
//! WebCheck validator and its findings are printed after ours, followed
//! by the diff (`N matched, M only ours, K only theirs`). The exit status
//! is unchanged by the oracle unless `--strict-oracle`, which exits 1 on
//! any disagreement. Never the default: it sends the file to the FEC.

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::{Args, ValueEnum};
use hardmoney::parser::webcheck::{Credentials, OracleDiff, OracleReport, WebCheck, diff};
use hardmoney::parser::{Severity, Validation};
use hardmoney::{Filing, ParseOptions};

use super::shared::EndpointArgs;

/// An external validator to compare our findings against.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Oracle {
    /// The FEC's WebCheck (<https://efoservices.fec.gov/webcheck/>). The
    /// public upload channel needs no credentials; with
    /// `--webcheck-api-key` the vendor SOAP service is used instead.
    Webcheck,
}

#[derive(Args, Debug)]
pub struct ValidateArgs {
    /// Path to a `.fec` file.
    pub path: PathBuf,

    /// Print the findings as a JSON document instead of one line each.
    #[arg(long)]
    pub json: bool,

    /// Exit 1 on warnings too, not only on errors.
    #[arg(long)]
    pub strict_warnings: bool,

    /// Also submit the file to an external validator and diff its
    /// findings against ours. Sends the file over the network.
    #[arg(long, value_enum)]
    pub oracle: Option<Oracle>,

    /// With `--oracle`, exit 1 if the oracle and hardmoney disagree on
    /// any finding.
    #[arg(long, requires = "oracle")]
    pub strict_oracle: bool,

    /// FEC vendor API key for WebCheck's SOAP service. Without it the
    /// credential-free upload channel (what the WebCheck web page uses)
    /// is taken. Ignored without `--oracle webcheck`.
    #[arg(long, env = "WEBCHECK_API_KEY", hide_env_values = true)]
    pub webcheck_api_key: Option<String>,

    /// Contact e-mail to pass WebCheck's SOAP service (it e-mails results
    /// for files over 20 MB). Ignored without `--webcheck-api-key`.
    #[arg(long, env = "WEBCHECK_EMAIL", hide_env_values = true)]
    pub webcheck_email: Option<String>,

    // Only `--webcheck-endpoint` is used here; the other endpoint flags
    // are accepted for uniformity across commands.
    #[command(flatten)]
    pub endpoints: EndpointArgs,
}

/// Non-zero exit without an "error:" chain from `main` -- the findings
/// themselves are the explanation and have already been printed.
#[derive(Debug)]
struct NotAcceptable {
    errors: usize,
    warnings: usize,
}

impl std::fmt::Display for NotAcceptable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "validation failed with {} error(s) and {} warning(s)",
            self.errors, self.warnings
        )
    }
}

impl std::error::Error for NotAcceptable {}

/// `--strict-oracle`: the oracle and hardmoney disagree.
#[derive(Debug)]
struct OracleDisagrees {
    only_ours: usize,
    only_theirs: usize,
}

impl std::fmt::Display for OracleDisagrees {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "oracle disagrees: {} finding(s) only ours, {} only theirs",
            self.only_ours, self.only_theirs
        )
    }
}

impl std::error::Error for OracleDisagrees {}

pub fn run(args: ValidateArgs) -> super::CliResult {
    let bytes = std::fs::read(&args.path)?;
    // Lenient: a line the FEC would ignore (#18) should be reported as a
    // finding, not abort the whole check.
    let lenient = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)?;
    let validation = lenient.validate();
    let filing = lenient.value();

    let errors = validation.error_count();
    let warnings = validation.warning_count();

    let oracle = match args.oracle {
        Some(Oracle::Webcheck) => Some(ask_webcheck(&args, &bytes, &validation)?),
        None => None,
    };

    if args.json {
        let mut by_rule: BTreeMap<String, usize> = BTreeMap::new();
        for f in &validation {
            *by_rule.entry(f.rule.to_string()).or_default() += 1;
        }
        let mut out = serde_json::json!({
            "file": args.path.display().to_string(),
            "form_type": filing.raw_form_type,
            "version": filing.version,
            "line_count": filing.lines.len(),
            "acceptable": validation.is_acceptable(),
            "errors": errors,
            "warnings": warnings,
            "findings_by_rule": by_rule,
            "findings": validation.findings,
        });
        if let Some((report, d)) = &oracle {
            out["oracle"] = serde_json::json!({
                "name": "webcheck",
                "result": report.result,
                "acceptable": report.is_acceptable(),
                "errors": report.errors_reported.unwrap_or_else(|| report.error_count()),
                "warnings": report.warnings_reported.unwrap_or_else(|| report.warning_count()),
                "filing_type": report.filing_type,
                "committee_id": report.committee_id,
                "findings": report.findings,
            });
            out["oracle_diff"] = serde_json::json!({
                "matched": d.matched.len(),
                "only_ours": d.only_ours,
                "only_theirs": d.only_theirs,
            });
        }
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print_text(&validation, &filing.raw_form_type, filing.lines.len());
        if let Some((report, d)) = &oracle {
            println!();
            println!("--- WebCheck ({}) ---", args.path.display());
            print!("{report}");
            println!();
            println!("--- Diff: hardmoney vs. WebCheck ---");
            print!("{d}");
        }
    }

    let fail = errors > 0 || (args.strict_warnings && warnings > 0);
    if fail {
        return Err(Box::new(NotAcceptable { errors, warnings }));
    }
    if args.strict_oracle
        && let Some((_, d)) = &oracle
        && !d.is_empty()
    {
        return Err(Box::new(OracleDisagrees {
            only_ours: d.only_ours.len(),
            only_theirs: d.only_theirs.len(),
        }));
    }
    Ok(())
}

/// Submits the file to WebCheck and diffs the report against `ours`. A
/// transport or service failure is the command's error: the oracle was
/// asked for, so silently skipping it would be misleading.
fn ask_webcheck(
    args: &ValidateArgs,
    bytes: &[u8],
    ours: &Validation,
) -> super::CliResult<(OracleReport, OracleDiff)> {
    let filename = args
        .path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "filing.fec".to_string());
    let credentials = args.webcheck_api_key.as_ref().map(|key| {
        let c = Credentials::new(key.clone());
        match &args.webcheck_email {
            Some(email) => c.with_email(email.clone()),
            None => c,
        }
    });
    let endpoints = args.endpoints.resolve()?;
    let report =
        WebCheck::with_endpoints(&endpoints).submit(&filename, bytes, credentials.as_ref())?;
    let d = diff(ours, &report);
    Ok((report, d))
}

fn print_text(validation: &Validation, form_type: &str, line_count: usize) {
    for f in validation {
        println!("{f}");
    }
    let verdict = if validation.is_acceptable() {
        "ACCEPTABLE"
    } else {
        "NOT ACCEPTABLE"
    };
    println!(
        "{verdict}: {form_type}, {line_count} body line(s), {} error(s), {} warning(s)",
        validation.error_count(),
        validation.warning_count()
    );
    if !validation.is_empty() {
        let mut by_rule: BTreeMap<(Severity, String), usize> = BTreeMap::new();
        for f in validation {
            *by_rule.entry((f.severity, f.rule.to_string())).or_default() += 1;
        }
        for ((severity, rule), n) in by_rule.iter().rev() {
            println!("  {n:>6} {severity:<7} {rule}");
        }
    }
}
