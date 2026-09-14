//! Port of `pyfec/header.py`: parses the first record of a filing, which
//! identifies the format version used by the rest of the file.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::parser::error::{FecError, Result};
use crate::parser::utils::clean_entry;

static OLD_EHEADERS: &[&str] = &[
    "record_type",
    "ef_type",
    "fec_version",
    "soft_name",
    "soft_ver",
    "name_delim",
    "report_id",
    "report_number",
];
static OLD_EHEADERS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[3|4|5]").unwrap());

static NEW_EHEADERS: &[&str] = &[
    "record_type",
    "ef_type",
    "fec_version",
    "soft_name",
    "soft_ver",
    "report_id",
    "report_number",
];
static NEW_EHEADERS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^[6|7|8]").unwrap());

static PAPER_HEADERS_V1: &[&str] = &["record_type", "fec_version", "vendor", "batch_number"];
static PAPER_HEADERS_V1_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^P1\.0").unwrap());

static PAPER_HEADERS_V2_2: &[&str] = &["record_type", "fec_version", "vendor", "batch_number"];
static PAPER_HEADERS_V2_2_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^P2\.2|^P2\.3|^P2\.4").unwrap());

static PAPER_HEADERS_V2_6: &[&str] = &[
    "record_type",
    "fec_version",
    "vendor",
    "batch_number",
    "report_id",
];
static PAPER_HEADERS_V2_6_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^P2\.6|^P3\.0|^P3\.1").unwrap());

/// Parsed header fields, keyed by canonical name (see the `*_HEADERS`
/// constants above for which keys are present for a given format).
pub type HeaderMap = indexmap::IndexMap<String, String>;

/// Parses a header row (already split into raw fields) into a canonical
/// field map. `is_paper` selects electronic vs. paper header layouts,
/// mirroring `header.parse(header_array, is_paper)`.
pub fn parse(header_array: &[String], is_paper: bool) -> Result<HeaderMap> {
    let (headers_list, _version): (&[&str], String) = if !is_paper {
        let version = clean_entry(header_array.get(2).map(|s| s.as_str()).unwrap_or(""));
        if OLD_EHEADERS_RE.is_match(&version) {
            (OLD_EHEADERS, version)
        } else if NEW_EHEADERS_RE.is_match(&version) {
            (NEW_EHEADERS, version)
        } else {
            return Err(FecError::UnknownElectronicHeaderVersion(version));
        }
    } else {
        let version = clean_entry(header_array.get(1).map(|s| s.as_str()).unwrap_or(""));
        if PAPER_HEADERS_V1_RE.is_match(&version) {
            (PAPER_HEADERS_V1, version)
        } else if PAPER_HEADERS_V2_2_RE.is_match(&version) {
            (PAPER_HEADERS_V2_2, version)
        } else if PAPER_HEADERS_V2_6_RE.is_match(&version) {
            (PAPER_HEADERS_V2_6, version)
        } else {
            return Err(FecError::UnknownPaperHeaderVersion(version));
        }
    };

    let mut headers = HeaderMap::new();
    for (i, key) in headers_list.iter().enumerate() {
        let value = header_array
            .get(i)
            .map(|s| clean_entry(s))
            .unwrap_or_default();
        headers.insert(key.to_string(), value);
    }
    Ok(headers)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_new_electronic_header() {
        let arr = v(&["HDR", "FEC", "8.5", "FECfile", "8.5.1.0(f34)", "", ""]);
        let h = parse(&arr, false).unwrap();
        assert_eq!(h.get("fec_version").unwrap(), "8.5");
        assert_eq!(h.get("soft_name").unwrap(), "FECFILE");
        assert!(!h.contains_key("name_delim"));
    }

    #[test]
    fn parses_old_electronic_header_with_name_delim() {
        let arr = v(&["HDR", "FEC", "5.3", "SoftCo", "1.0", ",", "FEC-1", "1"]);
        let h = parse(&arr, false).unwrap();
        assert_eq!(h.get("name_delim").unwrap(), ",");
    }

    #[test]
    fn trims_trailing_whitespace_in_version() {
        // Real-world filings sometimes have a stray trailing space on the
        // version field (observed in an actual 2026 F3X filing).
        let arr = v(&["HDR", "FEC", "8.5 ", "Some Vendor", "3.1"]);
        let h = parse(&arr, false).unwrap();
        assert_eq!(h.get("fec_version").unwrap(), "8.5");
    }

    #[test]
    fn unknown_version_errors() {
        let arr = v(&["HDR", "FEC", "9.9", "X", "1"]);
        assert!(parse(&arr, false).is_err());
    }
}
