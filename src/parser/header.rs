//! The first record of a filing (`HDR`), which identifies the format
//! version used by the rest of the file.
//!
//! Layout per the FEC's *Electronic Filing Specification, Part II* (sheet
//! `HDR`): `HDR`, `FEC`, version, software name, software version, report
//! id (the original filing's `FEC-<n>` -- amendments only), report number
//! (the sequential amendment number), and a free-text comment. Spec 3.x-5.x
//! carried an extra `name_delim` column between the software version and
//! the report id.

use crate::parser::error::{FecError, Result};
use crate::parser::schema::SpecVersion;
use crate::parser::utils::normalize_field;

/// The parsed `HDR` record of an electronic filing.
///
/// String fields are normalised like every other field (surrounding ASCII
/// whitespace and one pair of wrapping quotes removed, see
/// [`crate::parser::utils`]) but otherwise as filed; columns the physical
/// line did not carry are empty (older software omitted the trailing
/// report-id/number/comment columns entirely). `fec_version_raw` keeps the
/// wire spelling (`"3.00"`) while [`Header::version`] is the parsed value.
/// Nothing but the version is validated here: `record_type` and `ef_type`
/// are kept even when they are not `HDR`/`FEC` (`hardmoney validate`
/// reports that).
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct Header {
    /// Always `"HDR"` on a well-formed filing (kept as filed).
    pub record_type: String,
    /// Always `"FEC"` on a well-formed filing (kept as filed).
    pub ef_type: String,
    /// The version exactly as filed, e.g. `"8.5"` or `"3.00"`.
    pub fec_version_raw: String,
    /// The parsed version.
    pub version: SpecVersion,
    /// Filing software name, e.g. `"FECfile"`.
    pub soft_name: String,
    /// Filing software version, e.g. `"8.5.1.0(f34)"`.
    pub soft_ver: String,
    /// Spec 3.x-5.x only: the delimiter used inside name fields.
    pub name_delim: Option<String>,
    /// `FEC-<original filing number>` on amendments; otherwise blank.
    pub report_id: String,
    /// Sequential amendment number as filed (`"1"`, `"2"`, ...), blank on
    /// an original report.
    pub report_number: String,
    /// Free-text comment (`HDRcomment`, spec 6.x+); blank when absent.
    pub comment: String,
}

impl Header {
    /// Parses a header row that has already been split into raw fields
    /// (`["HDR", "FEC", "8.5", software, version, ...]`).
    ///
    /// Fails with [`FecError::UnknownElectronicHeaderVersion`] -- carrying
    /// the raw third field -- if that field is not a well-formed version
    /// (see [`SpecVersion`]) or names a paper-conversion (`P…`) or
    /// out-of-range (outside 3.x-8.x) version; an empty or too-short row
    /// fails the same way with an empty string. Every other column is
    /// optional and blank when absent, so a two-column row `["HDR",
    /// "FEC"]` fails only because its version is blank.
    pub fn from_fields(fields: &[&str]) -> Result<Self> {
        let field = |i: usize| {
            fields
                .get(i)
                .map(|s| normalize_field(s).to_string())
                .unwrap_or_default()
        };
        let fec_version_raw = field(2);
        let version: SpecVersion = fec_version_raw
            .parse()
            .map_err(|_| FecError::UnknownElectronicHeaderVersion(fec_version_raw.clone()))?;
        if version.is_paper() || !(3..=8).contains(&version.major()) {
            return Err(FecError::UnknownElectronicHeaderVersion(fec_version_raw));
        }
        // 3.x-5.x: HDR, FEC, ver, soft_name, soft_ver, name_delim, report_id, report_number, comment
        // 6.x-8.x: HDR, FEC, ver, soft_name, soft_ver, report_id, report_number, comment
        let (name_delim, tail) = if version.has_name_delim_header() {
            (Some(field(5)), 6)
        } else {
            (None, 5)
        };
        Ok(Self {
            record_type: field(0),
            ef_type: field(1),
            fec_version_raw,
            version,
            soft_name: field(3),
            soft_ver: field(4),
            name_delim,
            report_id: field(tail),
            report_number: field(tail + 1),
            comment: field(tail + 2),
        })
    }

    /// The header's columns in wire order for this version, as a writer
    /// would emit them.
    #[must_use]
    pub fn to_fields(&self) -> Vec<&str> {
        let mut out = vec![
            self.record_type.as_str(),
            self.ef_type.as_str(),
            self.fec_version_raw.as_str(),
            self.soft_name.as_str(),
            self.soft_ver.as_str(),
        ];
        if self.version.has_name_delim_header() {
            out.push(self.name_delim.as_deref().unwrap_or(""));
        }
        out.push(self.report_id.as_str());
        out.push(self.report_number.as_str());
        out.push(self.comment.as_str());
        out
    }

    /// The original filing number referenced by `report_id`
    /// (`"FEC-1234567"` -> `1234567`), if present and well-formed.
    #[must_use]
    pub fn original_filing_id(&self) -> Option<u64> {
        let rest = self
            .report_id
            .trim()
            .get(..3)
            .filter(|p| p.eq_ignore_ascii_case("fec"))
            .and_then(|_| self.report_id.trim().get(3..))?;
        let digits = rest.trim_start().strip_prefix('-')?.trim();
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok()
    }

    /// The sequential amendment number, if `report_number` is a number.
    #[must_use]
    pub fn amendment_number(&self) -> Option<u32> {
        self.report_number.trim().parse().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_new_electronic_header() {
        let h =
            Header::from_fields(&["HDR", "FEC", "8.5", "FECfile", "8.5.1.0(f34)", "", ""]).unwrap();
        assert_eq!(h.version, SpecVersion::electronic(8, 5));
        assert_eq!(h.fec_version_raw, "8.5");
        assert_eq!(h.soft_name, "FECfile");
        assert_eq!(h.name_delim, None);
        assert_eq!(h.comment, "");
        assert_eq!(h.original_filing_id(), None);
        assert_eq!(h.amendment_number(), None);
    }

    #[test]
    fn parses_header_comment_and_amendment_columns() {
        let h = Header::from_fields(&[
            "HDR",
            "FEC",
            "8.5",
            "Vendor",
            "1.0",
            "FEC-1234567",
            "2",
            "Filed via API",
        ])
        .unwrap();
        assert_eq!(h.report_id, "FEC-1234567");
        assert_eq!(h.original_filing_id(), Some(1_234_567));
        assert_eq!(h.amendment_number(), Some(2));
        assert_eq!(h.comment, "Filed via API");
        assert_eq!(
            h.to_fields(),
            [
                "HDR",
                "FEC",
                "8.5",
                "Vendor",
                "1.0",
                "FEC-1234567",
                "2",
                "Filed via API"
            ]
        );
    }

    #[test]
    fn parses_old_electronic_header_with_name_delim() {
        let h = Header::from_fields(&["HDR", "FEC", "5.3", "SoftCo", "1.0", "^", "FEC-1", "1"])
            .unwrap();
        assert_eq!(h.name_delim.as_deref(), Some("^"));
        assert_eq!(h.report_id, "FEC-1");
        assert_eq!(h.original_filing_id(), Some(1));
        assert_eq!(h.to_fields().len(), 9);
    }

    #[test]
    fn trims_trailing_whitespace_in_version() {
        // Observed in a real 2026 F3X filing: "8.5 ".
        let h = Header::from_fields(&["HDR", "FEC", "8.5 ", "Some Vendor", "3.1"]).unwrap();
        assert_eq!(h.fec_version_raw, "8.5");
        assert_eq!(h.version, SpecVersion::electronic(8, 5));
    }

    #[test]
    fn original_filing_id_tolerates_spacing_and_case() {
        let mk = |id: &str| Header::from_fields(&["HDR", "FEC", "8.5", "", "", id, ""]).unwrap();
        assert_eq!(mk("FEC - 42").original_filing_id(), Some(42));
        assert_eq!(mk("fec-7").original_filing_id(), Some(7));
        assert_eq!(mk("FEC-").original_filing_id(), None);
        assert_eq!(mk("FEC-12a").original_filing_id(), None);
        assert_eq!(mk("").original_filing_id(), None);
    }

    #[test]
    fn unknown_or_paper_versions_error() {
        let cases: [&[&str]; 5] = [
            &["HDR", "FEC", "9.9", "X", "1"],
            &["HDR", "FEC", "180.5", "X", "1"],
            &["HDR", "FEC", "P3.4", "X", "1"],
            &["HDR", "FEC"],
            &[],
        ];
        for bad in cases {
            assert!(
                matches!(
                    Header::from_fields(bad),
                    Err(FecError::UnknownElectronicHeaderVersion(_))
                ),
                "{bad:?}"
            );
        }
    }
}
