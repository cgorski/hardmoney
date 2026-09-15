//! Port of `pyfec/header.py`: parses the first record of a filing, which
//! identifies the format version used by the rest of the file.

use crate::parser::error::{FecError, Result};
use crate::parser::utils::clean_entry;

/// Spec 3.x-5.x electronic header: eight fields including `name_delim`.
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

/// Spec 6.x-8.x electronic header: seven fields (`name_delim` dropped).
static NEW_EHEADERS: &[&str] = &[
    "record_type",
    "ef_type",
    "fec_version",
    "soft_name",
    "soft_ver",
    "report_id",
    "report_number",
];

static PAPER_HEADERS_V1: &[&str] = &["record_type", "fec_version", "vendor", "batch_number"];
static PAPER_HEADERS_V2_2: &[&str] = &["record_type", "fec_version", "vendor", "batch_number"];
static PAPER_HEADERS_V2_6: &[&str] = &[
    "record_type",
    "fec_version",
    "vendor",
    "batch_number",
    "report_id",
];

/// The header layout for an electronic filing whose `fec_version` starts
/// with `version`. pyfec matched `^[3|4|5]` and `^[6|7|8]` (character
/// classes, so the pipes were literal and harmless); this is the same
/// test as a plain leading-character check.
fn electronic_layout(version: &str) -> Option<&'static [&'static str]> {
    match version.chars().next()? {
        '3' | '4' | '5' => Some(OLD_EHEADERS),
        '6' | '7' | '8' => Some(NEW_EHEADERS),
        _ => None,
    }
}

fn paper_layout(version: &str) -> Option<&'static [&'static str]> {
    if version.starts_with("P1.0") {
        Some(PAPER_HEADERS_V1)
    } else if ["P2.2", "P2.3", "P2.4"]
        .iter()
        .any(|p| version.starts_with(p))
    {
        Some(PAPER_HEADERS_V2_2)
    } else if ["P2.6", "P3.0", "P3.1"]
        .iter()
        .any(|p| version.starts_with(p))
    {
        Some(PAPER_HEADERS_V2_6)
    } else {
        None
    }
}

/// Parsed header fields, keyed by canonical name (see the `*_HEADERS`
/// constants above for which keys are present for a given format).
pub type HeaderMap = indexmap::IndexMap<String, String>;

/// Parses a header row (already split into raw fields) into a canonical
/// field map. `is_paper` selects electronic vs. paper header layouts,
/// mirroring `header.parse(header_array, is_paper)`.
pub fn parse(header_array: &[String], is_paper: bool) -> Result<HeaderMap> {
    let headers_list: &[&str] = if !is_paper {
        let version = clean_entry(header_array.get(2).map(|s| s.as_str()).unwrap_or(""));
        electronic_layout(&version).ok_or(FecError::UnknownElectronicHeaderVersion(version))?
    } else {
        let version = clean_entry(header_array.get(1).map(|s| s.as_str()).unwrap_or(""));
        paper_layout(&version).ok_or(FecError::UnknownPaperHeaderVersion(version))?
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
        assert!(parse(&v(&["HDR", "FEC"]), false).is_err());
        assert!(parse(&[], false).is_err());
        assert!(parse(&[], true).is_err());
    }

    #[test]
    fn paper_layouts_select_by_version_prefix() {
        let h = parse(&v(&["HDR", "P2.6", "VENDOR", "7", "FEC-1"]), true).unwrap();
        assert_eq!(h.get("report_id").unwrap(), "FEC-1");
        let h = parse(&v(&["HDR", "P1.0", "VENDOR", "7"]), true).unwrap();
        assert!(!h.contains_key("report_id"));
    }
}
