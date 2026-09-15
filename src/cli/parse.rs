//! `hardmoney parse`: parse one `.fec` file and print a JSON summary.

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Args;
use hardmoney::{Filing, ParseOptions};

#[derive(Args, Debug)]
pub struct ParseArgs {
    /// Path to a `.fec` file.
    pub path: PathBuf,

    /// Skip body lines that cannot be parsed (unknown form type, or no
    /// column layout for this spec version) instead of failing. Skipped
    /// lines are listed on stderr and counted in the JSON output.
    #[arg(long)]
    pub lenient: bool,

    /// Include every parsed body line (all fields) in the output. Large.
    #[arg(long)]
    pub lines: bool,
}

pub fn run(args: ParseArgs) -> super::CliResult {
    let bytes = std::fs::read(&args.path)?;
    let options = if args.lenient {
        ParseOptions::LENIENT
    } else {
        ParseOptions::STRICT
    };
    let (filing, skipped) = Filing::parse_bytes_with(&bytes, &options)?.into_parts();

    if !skipped.is_empty() {
        eprintln!("warning: {} line(s) skipped:", skipped.len());
        for s in skipped.iter().take(10) {
            eprintln!("  {s}");
        }
        if skipped.len() > 10 {
            eprintln!("  ... and {} more", skipped.len() - 10);
        }
    }

    let mut by_table: BTreeMap<String, usize> = BTreeMap::new();
    for line in &filing.lines {
        *by_table.entry(line.table.to_string()).or_default() += 1;
    }

    let mut out = serde_json::json!({
        "form_type": filing.raw_form_type,
        "base_form_type": filing.base_form_type,
        "version": filing.version,
        "is_amendment": filing.is_amendment,
        "amends_filing": filing.amends_filing,
        "line_count": filing.lines.len(),
        "lines_by_table": by_table,
        "skipped_count": skipped.len(),
        "skipped": skipped,
        "header": filing.headers,
        "summary": filing.summary.fields,
    });
    if args.lines {
        out["lines"] = serde_json::to_value(&filing.lines)?;
    }
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}
