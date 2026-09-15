//! `hardmoney validate`: check one `.fec` file against the FEC's
//! acceptance rules and print the findings, WebCheck-style.
//!
//! Exit status is 1 if the filing has any error-severity finding (the FEC
//! would reject it) -- or, with `--strict-warnings`, any finding at all --
//! and 0 otherwise. Findings go to stdout; parse failures go to stderr.

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Args;
use hardmoney::parser::{Severity, Validation};
use hardmoney::{Filing, ParseOptions};

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

pub fn run(args: ValidateArgs) -> super::CliResult {
    let bytes = std::fs::read(&args.path)?;
    // Lenient: a line the FEC would ignore (#18) should be reported as a
    // finding, not abort the whole check.
    let lenient = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)?;
    let validation = lenient.validate();
    let filing = lenient.value();

    let errors = validation.error_count();
    let warnings = validation.warning_count();

    if args.json {
        let mut by_rule: BTreeMap<String, usize> = BTreeMap::new();
        for f in &validation {
            *by_rule.entry(f.rule.to_string()).or_default() += 1;
        }
        let out = serde_json::json!({
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
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        print_text(&validation, &filing.raw_form_type, filing.lines.len());
    }

    let fail = errors > 0 || (args.strict_warnings && warnings > 0);
    if fail {
        return Err(Box::new(NotAcceptable { errors, warnings }));
    }
    Ok(())
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
