//! `hardmoney write`: parse a `.fec` file and write it back out in canonical
//! form (CRLF, full column width, normalised fields, F99 free text as a
//! `[BEGINTEXT]` block). Useful for normalising vendor output, for
//! generating golden fixtures, and for checking that a file round-trips.

use std::path::PathBuf;

use clap::Args;
use hardmoney::{Filing, ParseOptions};

#[derive(Args, Debug)]
pub struct WriteArgs {
    /// Path to a `.fec` file.
    pub path: PathBuf,

    /// Output path. Defaults to stdout.
    #[arg(long, short)]
    pub out: Option<PathBuf>,

    /// Skip body lines that cannot be parsed instead of failing (they are
    /// omitted from the output and listed on stderr).
    #[arg(long)]
    pub lenient: bool,

    /// Instead of writing, re-parse the written bytes and report whether
    /// every record round-trips; exit 1 if not.
    #[arg(long)]
    pub check: bool,
}

pub fn run(args: WriteArgs) -> super::CliResult {
    let bytes = std::fs::read(&args.path)?;
    let options = if args.lenient {
        ParseOptions::LENIENT
    } else {
        ParseOptions::STRICT
    };
    let (filing, skipped) = Filing::parse_bytes_with(&bytes, &options)?.into_parts();
    for s in &skipped {
        eprintln!("warning: omitted {s}");
    }

    let written = filing.to_fec();

    if args.check {
        let again = Filing::parse_bytes(&written)?;
        let mut problems = Vec::new();
        if filing.header != again.header {
            problems.push("header differs".to_string());
        }
        if !filing.summary.iter().eq(again.summary.iter()) {
            problems.push("cover line differs".to_string());
        }
        if filing.lines.len() != again.lines.len() {
            problems.push(format!(
                "line count differs: {} vs {}",
                filing.lines.len(),
                again.lines.len()
            ));
        }
        for (a, b) in filing.lines.iter().zip(&again.lines) {
            if a.table() != b.table() || !a.iter().eq(b.iter()) {
                problems.push(format!("line {} differs", a.line_no));
                if problems.len() > 10 {
                    break;
                }
            }
        }
        if problems.is_empty() {
            println!(
                "OK: {} round-trips ({} body lines, {} bytes in, {} bytes out)",
                args.path.display(),
                filing.lines.len(),
                bytes.len(),
                written.len()
            );
            return Ok(());
        }
        for p in &problems {
            eprintln!("MISMATCH: {p}");
        }
        return Err(format!(
            "{} problem(s) round-tripping {}",
            problems.len(),
            args.path.display()
        )
        .into());
    }

    match args.out {
        Some(out) => std::fs::write(&out, written)?,
        None => {
            use std::io::Write as _;
            std::io::stdout().lock().write_all(&written)?;
        }
    }
    Ok(())
}
