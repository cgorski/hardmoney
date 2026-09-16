//! The streaming parser and the eager parser must agree on arbitrary
//! bytes -- not only on well-formed filings -- in both modes: same
//! success/failure, same skipped-line count, same filing; on failure, the
//! same line number. Input on which the two are documented to decode
//! differently (see `common::decoders_agree`) is skipped: that is a
//! contract, not a finding.
#![no_main]

#[path = "common.rs"]
mod common;

use std::io::Cursor;

use hardmoney::parser::stream::FilingReader;
use hardmoney::{Filing, ParseOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if !common::decoders_agree(data) {
        return;
    }
    for (label, options) in [
        ("strict", ParseOptions::STRICT),
        ("lenient", ParseOptions::LENIENT),
    ] {
        let eager = Filing::parse_bytes_with(data, &options);
        let streamed =
            FilingReader::with_options(Cursor::new(data), options).and_then(|r| r.into_filing());
        match (eager, streamed) {
            (Ok(e), Ok(s)) => {
                let (ef, es) = e.into_parts();
                let (sf, ss) = s.into_parts();
                assert_eq!(
                    es.len(),
                    ss.len(),
                    "{label}: eager skipped {} line(s), streaming {}",
                    es.len(),
                    ss.len()
                );
                assert!(
                    common::same_filing(&ef, &sf),
                    "{label}: parsers disagree: {}",
                    common::first_difference(&ef, &sf)
                );
            }
            (Err(e), Err(s)) => {
                assert_eq!(
                    e.line_no(),
                    s.line_no(),
                    "{label}: eager failed at {:?} ({e}), streaming at {:?} ({s})",
                    e.line_no(),
                    s.line_no()
                );
            }
            (Ok(_), Err(s)) => panic!("{label}: eager parsed, streaming failed: {s}"),
            (Err(e), Ok(_)) => panic!("{label}: streaming parsed, eager failed: {e}"),
        }
    }

    // The iterator form, line by line, must not panic either.
    if let Ok(reader) = FilingReader::new(Cursor::new(data)) {
        for line in reader {
            if line.is_err() {
                break;
            }
        }
    }
});
