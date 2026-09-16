//! The writer is the inverse of the parser: anything that parses strictly
//! writes back to bytes that parse to the same header, cover, and body
//! lines (content, not physical line numbers: canonical output drops
//! blank lines), and the canonical form is a fixed point (writing it
//! again gives identical bytes). The streaming parser must read the
//! canonical bytes the same way, except on the documented encoding
//! divergence (see `common::decoders_agree`).
#![no_main]

#[path = "common.rs"]
mod common;

use std::io::Cursor;

use hardmoney::Filing;
use hardmoney::parser::stream::FilingReader;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(filing) = Filing::parse_bytes(data) else {
        return;
    };
    if !common::delimiter_matches_version(data, &filing) {
        return;
    }
    let bytes = filing.to_fec();
    let again = match Filing::parse_bytes(&bytes) {
        Ok(f) => f,
        Err(e) => panic!("canonical output does not parse: {e}"),
    };
    assert!(
        common::same_filing_content(&filing, &again),
        "round trip changed the filing: {}",
        common::first_difference(&filing, &again)
    );
    assert_eq!(again.to_fec(), bytes, "canonical form is not a fixed point");
    let streamed = FilingReader::new(Cursor::new(&bytes))
        .and_then(|r| r.into_filing())
        .unwrap_or_else(|e| panic!("streaming parser rejects canonical output: {e}"));
    let (streamed, skipped) = streamed.into_parts();
    assert!(skipped.is_empty());
    if common::decoders_agree(&bytes) {
        assert!(
            common::same_filing_content(&filing, &streamed),
            "streaming parser reads canonical output differently: {}",
            common::first_difference(&filing, &streamed)
        );
    }
});
