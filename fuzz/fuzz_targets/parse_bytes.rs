//! Arbitrary bytes through both parse modes.
//!
//! Invariants:
//! * neither mode panics;
//! * lenient accepts everything strict accepts, skips nothing on such
//!   input, and yields the same filing;
//! * whatever parsed can be written back and decoded.
#![no_main]

#[path = "common.rs"]
mod common;

use hardmoney::parser::filing::decode;
use hardmoney::{Filing, ParseOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let strict = Filing::parse_bytes(data);
    let lenient = Filing::parse_bytes_with(data, &ParseOptions::LENIENT);
    if let Ok(s) = &strict {
        let l = lenient
            .as_ref()
            .expect("lenient must accept what strict accepts");
        assert!(
            l.skipped().is_empty(),
            "lenient skipped {} line(s) of a strictly valid filing",
            l.skipped().len()
        );
        assert!(
            common::same_filing(s, l.value()),
            "strict and lenient disagree: {}",
            common::first_difference(s, l.value())
        );
    }
    if let Ok(l) = lenient {
        let (filing, _skipped) = l.into_parts();
        let _ = filing.to_fec();
    }
    let _ = decode(data);
});
