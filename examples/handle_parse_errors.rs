//! `hardmoney`'s error type is designed to be matched on and handled, not
//! just unwrapped. This demonstrates the scenarios you'll actually hit
//! against real-world FEC data:
//!
//! 1. A malformed/unrecognized filing (garbage input, or a form type this
//!    crate doesn't know about yet) -- `Filing::parse_bytes` returns a
//!    `Result`, never panics.
//! 2. A spec version the bundled tables do not cover for some table --
//!    `FecError::NoMatchingVersionBucket` names the table, version, and
//!    line.
//! 3. Strict vs. lenient body-line handling.
//! 4. Editing a line: `ParsedLine::set` refuses a field name the table
//!    does not have, so a typo cannot silently create a phantom field.
//!
//! Run with:
//!
//!     cargo run --example handle_parse_errors

use hardmoney::{FecError, Filing, ParseOptions, ParsedLine, SpecVersion, Table};

fn main() {
    // --- Scenario 1: graceful handling of bad input ---
    match Filing::parse_bytes(b"not a real filing") {
        Ok(_) => unreachable!("garbage input should never parse successfully"),
        Err(e) => println!("expected parse failure: {e}"),
    }

    // --- Scenario 2: a table with no layout for this version ---
    // Schedule I was dropped by the FEC in spec 8.5, so an SI line in an
    // 8.5 filing has no column layout to parse with.
    let with_sch_i = concat!(
        "HDR\x1cFEC\x1c8.5\x1cFECfile\x1c8.5.1.0\x1c\x1c\n",
        "F3XN\x1cC00123456\x1cCOMMITTEE NAME\n",
        "SI\x1cC00123456\x1cLEVIN ACCOUNT\n",
    );
    match Filing::parse(with_sch_i) {
        Err(FecError::NoMatchingVersionBucket {
            table,
            version,
            line_no,
        }) => {
            println!("no layout for {table} at spec {version} (line {line_no:?}), as expected");
        }
        other => panic!("expected NoMatchingVersionBucket, got {other:?}"),
    }

    // --- Scenario 3: strict vs. lenient body-line handling ---
    // A filing with one line whose form-type token matches no format
    // table. Strict parsing (the default) fails the whole filing and tells
    // you the line number; lenient parsing keeps the good lines and hands
    // back the skipped ones, which you *must* look at (or explicitly
    // discard) to get at the `Filing`.
    let with_junk = concat!(
        "HDR\x1cFEC\x1c8.5\x1cFECfile\x1c8.5.1.0\x1c\x1c\n",
        "F3XN\x1cC00123456\x1cCOMMITTEE NAME\n",
        "SA11AI\x1cC00123456\x1cIND\x1cSMITH, JANE\n",
        "ZZZ\x1cthis line type does not exist\n",
    );
    match Filing::parse(with_junk) {
        Err(FecError::ParserMissing {
            form_type, line_no, ..
        }) => {
            println!("strict: refused '{form_type}' at line {line_no:?}, as expected");
        }
        other => panic!("expected ParserMissing, got {other:?}"),
    }
    let (filing, skipped) = Filing::parse_with(with_junk, &ParseOptions::LENIENT)
        .expect("lenient parse of a well-formed header never fails")
        .into_parts();
    println!(
        "lenient: kept {} line(s), skipped {}: {}",
        filing.lines.len(),
        skipped.len(),
        skipped[0]
    );

    // --- Scenario 4: editing with unknown field names is an error ---
    let mut line = ParsedLine::from_pairs(
        Table::SchA,
        SpecVersion::electronic(8, 5),
        0,
        [("form_type", "SA11AI"), ("contribution_amount", "250.00")],
    )
    .expect("known fields");
    match line.set("contributon_amount", "1.00") {
        Err(FecError::UnknownField { table, field }) => {
            println!("refused to set '{field}' on {table}: no such field, as expected");
        }
        other => panic!("expected UnknownField, got {other:?}"),
    }
    assert_eq!(line.get("contribution_amount"), Some("250.00"));
}
