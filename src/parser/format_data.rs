//! Compile-time embedding of the fec-csv-sources format tables.
//!
//! Source and provenance: these tables originate from the community-
//! maintained fech-sources project (https://github.com/dwillis/fech-sources,
//! itself derived from the `sources/` directory shipped with the NYT's Fech
//! Ruby gem, the same lineage nyt-pyfec's own bundled tables come from),
//! current through FEC electronic filing spec 8.5.0.1. See
//! ../README.md for full sourcing notes, and
//! ../tests/real_filing_validation.rs for validation against live 2026
//! sample filings.
//!
//! Regenerate this file with `python3 ../../scripts/generate_format_data.py`
//! after adding, removing, or renaming a CSV in data/fec-csv-sources/.

pub static FORM_CSV_DATA: &[(&str, &str)] = &[
    ("F1", include_str!("../../data/fec-csv-sources/F1.csv")),
    ("F10", include_str!("../../data/fec-csv-sources/F10.csv")),
    ("F105", include_str!("../../data/fec-csv-sources/F105.csv")),
    ("F13", include_str!("../../data/fec-csv-sources/F13.csv")),
    ("F132", include_str!("../../data/fec-csv-sources/F132.csv")),
    ("F133", include_str!("../../data/fec-csv-sources/F133.csv")),
    ("F1M", include_str!("../../data/fec-csv-sources/F1M.csv")),
    ("F1S", include_str!("../../data/fec-csv-sources/F1S.csv")),
    ("F2", include_str!("../../data/fec-csv-sources/F2.csv")),
    ("F24", include_str!("../../data/fec-csv-sources/F24.csv")),
    ("F3", include_str!("../../data/fec-csv-sources/F3.csv")),
    ("F3L", include_str!("../../data/fec-csv-sources/F3L.csv")),
    ("F3P", include_str!("../../data/fec-csv-sources/F3P.csv")),
    ("F3P31", include_str!("../../data/fec-csv-sources/F3P31.csv")),
    ("F3PS", include_str!("../../data/fec-csv-sources/F3PS.csv")),
    ("F3PZ1", include_str!("../../data/fec-csv-sources/F3PZ1.csv")),
    ("F3PZ2", include_str!("../../data/fec-csv-sources/F3PZ2.csv")),
    ("F3S", include_str!("../../data/fec-csv-sources/F3S.csv")),
    ("F3X", include_str!("../../data/fec-csv-sources/F3X.csv")),
    ("F3Z", include_str!("../../data/fec-csv-sources/F3Z.csv")),
    ("F3Z1", include_str!("../../data/fec-csv-sources/F3Z1.csv")),
    ("F3Z2", include_str!("../../data/fec-csv-sources/F3Z2.csv")),
    ("F4", include_str!("../../data/fec-csv-sources/F4.csv")),
    ("F5", include_str!("../../data/fec-csv-sources/F5.csv")),
    ("F56", include_str!("../../data/fec-csv-sources/F56.csv")),
    ("F57", include_str!("../../data/fec-csv-sources/F57.csv")),
    ("F6", include_str!("../../data/fec-csv-sources/F6.csv")),
    ("F65", include_str!("../../data/fec-csv-sources/F65.csv")),
    ("F7", include_str!("../../data/fec-csv-sources/F7.csv")),
    ("F76", include_str!("../../data/fec-csv-sources/F76.csv")),
    ("F8", include_str!("../../data/fec-csv-sources/F8.csv")),
    ("F82", include_str!("../../data/fec-csv-sources/F82.csv")),
    ("F83", include_str!("../../data/fec-csv-sources/F83.csv")),
    ("F9", include_str!("../../data/fec-csv-sources/F9.csv")),
    ("F91", include_str!("../../data/fec-csv-sources/F91.csv")),
    ("F92", include_str!("../../data/fec-csv-sources/F92.csv")),
    ("F93", include_str!("../../data/fec-csv-sources/F93.csv")),
    ("F94", include_str!("../../data/fec-csv-sources/F94.csv")),
    ("F99", include_str!("../../data/fec-csv-sources/F99.csv")),
    ("H1", include_str!("../../data/fec-csv-sources/H1.csv")),
    ("H2", include_str!("../../data/fec-csv-sources/H2.csv")),
    ("H3", include_str!("../../data/fec-csv-sources/H3.csv")),
    ("H4", include_str!("../../data/fec-csv-sources/H4.csv")),
    ("H5", include_str!("../../data/fec-csv-sources/H5.csv")),
    ("H6", include_str!("../../data/fec-csv-sources/H6.csv")),
    ("HDR", include_str!("../../data/fec-csv-sources/HDR.csv")),
    ("SchA", include_str!("../../data/fec-csv-sources/SchA.csv")),
    ("SchA3L", include_str!("../../data/fec-csv-sources/SchA3L.csv")),
    ("SchB", include_str!("../../data/fec-csv-sources/SchB.csv")),
    ("SchC", include_str!("../../data/fec-csv-sources/SchC.csv")),
    ("SchC1", include_str!("../../data/fec-csv-sources/SchC1.csv")),
    ("SchC2", include_str!("../../data/fec-csv-sources/SchC2.csv")),
    ("SchD", include_str!("../../data/fec-csv-sources/SchD.csv")),
    ("SchE", include_str!("../../data/fec-csv-sources/SchE.csv")),
    ("SchF", include_str!("../../data/fec-csv-sources/SchF.csv")),
    ("SchI", include_str!("../../data/fec-csv-sources/SchI.csv")),
    ("SchL", include_str!("../../data/fec-csv-sources/SchL.csv")),
    ("TEXT", include_str!("../../data/fec-csv-sources/TEXT.csv")),
];
