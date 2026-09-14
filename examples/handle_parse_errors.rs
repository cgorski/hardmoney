//! Less common, but very useful: `hardmoney`'s error type is designed to
//! be matched on and handled, not just unwrapped. This demonstrates two
//! scenarios you'll actually hit against real-world FEC data:
//!
//! 1. A malformed/unrecognized filing (garbage input, or a form type this
//!    crate doesn't know about yet) -- `Filing::parse_bytes` returns a
//!    `Result`, never panics.
//! 2. The collision guard: if a format table (whether bundled or your
//!    own) assigns the same canonical field name to two different column
//!    positions in one version bucket, `Line::from_csv_str` returns
//!    `FecError::DuplicateCanonicalField` instead of silently letting the
//!    second field clobber the first -- this is the exact class of bug
//!    that was found and fixed in six of the bundled format tables (see
//!    the README's "Parser correctness" section and `NOTICE`).
//!
//! Run with:
//!
//!     cargo run --example handle_parse_errors

use hardmoney::parser::line::Line;
use hardmoney::{FecError, Filing};

fn main() {
    // --- Scenario 1: graceful handling of bad input ---
    match Filing::parse_bytes(b"not a real filing") {
        Ok(_) => unreachable!("garbage input should never parse successfully"),
        Err(e) => println!("expected parse failure: {e}"),
    }

    // --- Scenario 2: the duplicate-canonical-field guard ---
    // A minimal synthetic format table with one version bucket ("^1") that
    // (incorrectly) maps both column 1 and column 2 to the canonical name
    // "amount" -- exactly the class of mistake six of the real bundled
    // tables had before it was fixed.
    let colliding_table = "canonical,^1\namount,1\namount,2\n";

    match Line::from_csv_str("DEMO", colliding_table) {
        Ok(_) => unreachable!("a same-bucket collision must be rejected"),
        Err(FecError::DuplicateCanonicalField {
            form,
            version_bucket,
            canonical,
            first_position,
            second_position,
        }) => {
            println!(
                "caught the collision guard as expected: form={form} bucket={version_bucket} \
                 field={canonical:?} positions {first_position} and {second_position} collide"
            );
        }
        Err(other) => panic!("expected DuplicateCanonicalField, got: {other}"),
    }

    // The same canonical name repeated at the *same* position across rows
    // is fine (that's just redundant documentation in the source table,
    // not a real conflict) -- only a genuine position mismatch errors.
    let redundant_but_fine = "canonical,^1\namount,1\namount,1\n";
    Line::from_csv_str("DEMO", redundant_but_fine)
        .expect("identical repeated positions are not a collision");
    println!("repeated-but-identical positions parsed fine, as expected");
}
