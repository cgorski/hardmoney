//! Everything downstream of a lenient parse must be panic-free on any
//! input: the validator, the reconciler (an error for forms without
//! rules), the reviewer, and their `Display`/`Debug` output.
#![no_main]

use hardmoney::{Filing, ParseOptions};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Ok(lenient) = Filing::parse_bytes_with(data, &ParseOptions::LENIENT) else {
        return;
    };
    // Validation of the lenient wrapper reports skipped lines too.
    let _ = format!("{:?}", lenient.validate());
    let (filing, _skipped) = lenient.into_parts();
    let validation = filing.validate();
    let _ = validation.is_acceptable();
    for finding in &validation.findings {
        let _ = finding.to_string();
    }
    match filing.reconcile() {
        Ok(r) => {
            let _ = r.balances();
            let _ = r.mismatches().count();
            let _ = format!("{r:?}");
        }
        Err(e) => {
            let _ = e.to_string();
        }
    }
    let review = filing.review();
    let _ = format!("{review:?}");
});
