//! Compile-time embedding of the fec-csv-sources format tables, plus the
//! [`Table`] enum that names them.
//!
//! Source and provenance: these tables originate from the community-
//! maintained fech-sources project (<https://github.com/dwillis/fech-sources>,
//! itself derived from the `sources/` directory shipped with the NYT's Fech
//! Ruby gem, the same lineage nyt-pyfec's own bundled tables come from),
//! current through FEC electronic filing spec 8.5.0.1, with the corrections
//! listed in `NOTICE`. `F2S.csv` is authored locally (fech-sources has no
//! table for it). See `README.md` for full sourcing notes and
//! `tests/real_filings.rs` for validation against real filings.
//!
//! GENERATED FILE -- do not edit by hand. Regenerate with
//! `python3 scripts/generate_format_data.py` after adding, removing, or
//! renaming a CSV in `data/fec-csv-sources/`.

/// One FEC format table: a form, schedule, or record type whose column
/// layout (per spec version) is described by a bundled CSV.
///
/// This is the type-level identity of a table. [`crate::parser::ParsedLine::table`]
/// carries it so callers can match on a closed set instead of comparing
/// strings, and typed views use it to refuse lines from the wrong table.
///
/// `#[non_exhaustive]` because the FEC adds record types over time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum Table {
    /// `F1.csv`
    F1,
    /// `F10.csv`
    F10,
    /// `F105.csv`
    F105,
    /// `F13.csv`
    F13,
    /// `F132.csv`
    F132,
    /// `F133.csv`
    F133,
    /// `F1M.csv`
    F1M,
    /// `F1S.csv`
    F1S,
    /// `F2.csv`
    F2,
    /// `F24.csv`
    F24,
    /// `F2S.csv`
    F2S,
    /// `F3.csv`
    F3,
    /// `F3L.csv`
    F3L,
    /// `F3P.csv`
    F3P,
    /// `F3P31.csv`
    F3P31,
    /// `F3PS.csv`
    F3PS,
    /// `F3PZ1.csv`
    F3PZ1,
    /// `F3PZ2.csv`
    F3PZ2,
    /// `F3S.csv`
    F3S,
    /// `F3X.csv`
    F3X,
    /// `F3Z.csv`
    F3Z,
    /// `F3Z1.csv`
    F3Z1,
    /// `F3Z2.csv`
    F3Z2,
    /// `F4.csv`
    F4,
    /// `F5.csv`
    F5,
    /// `F56.csv`
    F56,
    /// `F57.csv`
    F57,
    /// `F6.csv`
    F6,
    /// `F65.csv`
    F65,
    /// `F7.csv`
    F7,
    /// `F76.csv`
    F76,
    /// `F8.csv`
    F8,
    /// `F82.csv`
    F82,
    /// `F83.csv`
    F83,
    /// `F9.csv`
    F9,
    /// `F91.csv`
    F91,
    /// `F92.csv`
    F92,
    /// `F93.csv`
    F93,
    /// `F94.csv`
    F94,
    /// `F99.csv`
    F99,
    /// `H1.csv`
    H1,
    /// `H2.csv`
    H2,
    /// `H3.csv`
    H3,
    /// `H4.csv`
    H4,
    /// `H5.csv`
    H5,
    /// `H6.csv`
    H6,
    /// `HDR.csv`
    Hdr,
    /// `SchA.csv`
    SchA,
    /// `SchA3L.csv`
    SchA3L,
    /// `SchB.csv`
    SchB,
    /// `SchC.csv`
    SchC,
    /// `SchC1.csv`
    SchC1,
    /// `SchC2.csv`
    SchC2,
    /// `SchD.csv`
    SchD,
    /// `SchE.csv`
    SchE,
    /// `SchF.csv`
    SchF,
    /// `SchI.csv`
    SchI,
    /// `SchL.csv`
    SchL,
    /// `TEXT.csv`
    Text,
}

impl Table {
    /// Every table, in the same order as [`FORM_CSV_DATA`].
    pub const ALL: &[Table] = &[
        Table::F1,
        Table::F10,
        Table::F105,
        Table::F13,
        Table::F132,
        Table::F133,
        Table::F1M,
        Table::F1S,
        Table::F2,
        Table::F24,
        Table::F2S,
        Table::F3,
        Table::F3L,
        Table::F3P,
        Table::F3P31,
        Table::F3PS,
        Table::F3PZ1,
        Table::F3PZ2,
        Table::F3S,
        Table::F3X,
        Table::F3Z,
        Table::F3Z1,
        Table::F3Z2,
        Table::F4,
        Table::F5,
        Table::F56,
        Table::F57,
        Table::F6,
        Table::F65,
        Table::F7,
        Table::F76,
        Table::F8,
        Table::F82,
        Table::F83,
        Table::F9,
        Table::F91,
        Table::F92,
        Table::F93,
        Table::F94,
        Table::F99,
        Table::H1,
        Table::H2,
        Table::H3,
        Table::H4,
        Table::H5,
        Table::H6,
        Table::Hdr,
        Table::SchA,
        Table::SchA3L,
        Table::SchB,
        Table::SchC,
        Table::SchC1,
        Table::SchC2,
        Table::SchD,
        Table::SchE,
        Table::SchF,
        Table::SchI,
        Table::SchL,
        Table::Text,
    ];

    /// Position of this variant in [`Table::ALL`] (its declaration order).
    pub(crate) const fn index(self) -> usize {
        self as usize
    }

    /// The table's name as used in the CSV filename and in older
    /// string-based APIs, e.g. `"SchA"`, `"F3X"`.
    pub const fn as_str(self) -> &'static str {
        match self {
            Table::F1 => "F1",
            Table::F10 => "F10",
            Table::F105 => "F105",
            Table::F13 => "F13",
            Table::F132 => "F132",
            Table::F133 => "F133",
            Table::F1M => "F1M",
            Table::F1S => "F1S",
            Table::F2 => "F2",
            Table::F24 => "F24",
            Table::F2S => "F2S",
            Table::F3 => "F3",
            Table::F3L => "F3L",
            Table::F3P => "F3P",
            Table::F3P31 => "F3P31",
            Table::F3PS => "F3PS",
            Table::F3PZ1 => "F3PZ1",
            Table::F3PZ2 => "F3PZ2",
            Table::F3S => "F3S",
            Table::F3X => "F3X",
            Table::F3Z => "F3Z",
            Table::F3Z1 => "F3Z1",
            Table::F3Z2 => "F3Z2",
            Table::F4 => "F4",
            Table::F5 => "F5",
            Table::F56 => "F56",
            Table::F57 => "F57",
            Table::F6 => "F6",
            Table::F65 => "F65",
            Table::F7 => "F7",
            Table::F76 => "F76",
            Table::F8 => "F8",
            Table::F82 => "F82",
            Table::F83 => "F83",
            Table::F9 => "F9",
            Table::F91 => "F91",
            Table::F92 => "F92",
            Table::F93 => "F93",
            Table::F94 => "F94",
            Table::F99 => "F99",
            Table::H1 => "H1",
            Table::H2 => "H2",
            Table::H3 => "H3",
            Table::H4 => "H4",
            Table::H5 => "H5",
            Table::H6 => "H6",
            Table::Hdr => "HDR",
            Table::SchA => "SchA",
            Table::SchA3L => "SchA3L",
            Table::SchB => "SchB",
            Table::SchC => "SchC",
            Table::SchC1 => "SchC1",
            Table::SchC2 => "SchC2",
            Table::SchD => "SchD",
            Table::SchE => "SchE",
            Table::SchF => "SchF",
            Table::SchI => "SchI",
            Table::SchL => "SchL",
            Table::Text => "TEXT",
        }
    }

    /// Inverse of [`Table::as_str`] (exact, case-sensitive).
    pub fn from_name(name: &str) -> Option<Table> {
        match name {
            "F1" => Some(Table::F1),
            "F10" => Some(Table::F10),
            "F105" => Some(Table::F105),
            "F13" => Some(Table::F13),
            "F132" => Some(Table::F132),
            "F133" => Some(Table::F133),
            "F1M" => Some(Table::F1M),
            "F1S" => Some(Table::F1S),
            "F2" => Some(Table::F2),
            "F24" => Some(Table::F24),
            "F2S" => Some(Table::F2S),
            "F3" => Some(Table::F3),
            "F3L" => Some(Table::F3L),
            "F3P" => Some(Table::F3P),
            "F3P31" => Some(Table::F3P31),
            "F3PS" => Some(Table::F3PS),
            "F3PZ1" => Some(Table::F3PZ1),
            "F3PZ2" => Some(Table::F3PZ2),
            "F3S" => Some(Table::F3S),
            "F3X" => Some(Table::F3X),
            "F3Z" => Some(Table::F3Z),
            "F3Z1" => Some(Table::F3Z1),
            "F3Z2" => Some(Table::F3Z2),
            "F4" => Some(Table::F4),
            "F5" => Some(Table::F5),
            "F56" => Some(Table::F56),
            "F57" => Some(Table::F57),
            "F6" => Some(Table::F6),
            "F65" => Some(Table::F65),
            "F7" => Some(Table::F7),
            "F76" => Some(Table::F76),
            "F8" => Some(Table::F8),
            "F82" => Some(Table::F82),
            "F83" => Some(Table::F83),
            "F9" => Some(Table::F9),
            "F91" => Some(Table::F91),
            "F92" => Some(Table::F92),
            "F93" => Some(Table::F93),
            "F94" => Some(Table::F94),
            "F99" => Some(Table::F99),
            "H1" => Some(Table::H1),
            "H2" => Some(Table::H2),
            "H3" => Some(Table::H3),
            "H4" => Some(Table::H4),
            "H5" => Some(Table::H5),
            "H6" => Some(Table::H6),
            "HDR" => Some(Table::Hdr),
            "SchA" => Some(Table::SchA),
            "SchA3L" => Some(Table::SchA3L),
            "SchB" => Some(Table::SchB),
            "SchC" => Some(Table::SchC),
            "SchC1" => Some(Table::SchC1),
            "SchC2" => Some(Table::SchC2),
            "SchD" => Some(Table::SchD),
            "SchE" => Some(Table::SchE),
            "SchF" => Some(Table::SchF),
            "SchI" => Some(Table::SchI),
            "SchL" => Some(Table::SchL),
            "TEXT" => Some(Table::Text),
            _ => None,
        }
    }

    /// The raw CSV format table for this variant.
    pub const fn csv(self) -> &'static str {
        match self {
            Table::F1 => include_str!("../../data/fec-csv-sources/F1.csv"),
            Table::F10 => include_str!("../../data/fec-csv-sources/F10.csv"),
            Table::F105 => include_str!("../../data/fec-csv-sources/F105.csv"),
            Table::F13 => include_str!("../../data/fec-csv-sources/F13.csv"),
            Table::F132 => include_str!("../../data/fec-csv-sources/F132.csv"),
            Table::F133 => include_str!("../../data/fec-csv-sources/F133.csv"),
            Table::F1M => include_str!("../../data/fec-csv-sources/F1M.csv"),
            Table::F1S => include_str!("../../data/fec-csv-sources/F1S.csv"),
            Table::F2 => include_str!("../../data/fec-csv-sources/F2.csv"),
            Table::F24 => include_str!("../../data/fec-csv-sources/F24.csv"),
            Table::F2S => include_str!("../../data/fec-csv-sources/F2S.csv"),
            Table::F3 => include_str!("../../data/fec-csv-sources/F3.csv"),
            Table::F3L => include_str!("../../data/fec-csv-sources/F3L.csv"),
            Table::F3P => include_str!("../../data/fec-csv-sources/F3P.csv"),
            Table::F3P31 => include_str!("../../data/fec-csv-sources/F3P31.csv"),
            Table::F3PS => include_str!("../../data/fec-csv-sources/F3PS.csv"),
            Table::F3PZ1 => include_str!("../../data/fec-csv-sources/F3PZ1.csv"),
            Table::F3PZ2 => include_str!("../../data/fec-csv-sources/F3PZ2.csv"),
            Table::F3S => include_str!("../../data/fec-csv-sources/F3S.csv"),
            Table::F3X => include_str!("../../data/fec-csv-sources/F3X.csv"),
            Table::F3Z => include_str!("../../data/fec-csv-sources/F3Z.csv"),
            Table::F3Z1 => include_str!("../../data/fec-csv-sources/F3Z1.csv"),
            Table::F3Z2 => include_str!("../../data/fec-csv-sources/F3Z2.csv"),
            Table::F4 => include_str!("../../data/fec-csv-sources/F4.csv"),
            Table::F5 => include_str!("../../data/fec-csv-sources/F5.csv"),
            Table::F56 => include_str!("../../data/fec-csv-sources/F56.csv"),
            Table::F57 => include_str!("../../data/fec-csv-sources/F57.csv"),
            Table::F6 => include_str!("../../data/fec-csv-sources/F6.csv"),
            Table::F65 => include_str!("../../data/fec-csv-sources/F65.csv"),
            Table::F7 => include_str!("../../data/fec-csv-sources/F7.csv"),
            Table::F76 => include_str!("../../data/fec-csv-sources/F76.csv"),
            Table::F8 => include_str!("../../data/fec-csv-sources/F8.csv"),
            Table::F82 => include_str!("../../data/fec-csv-sources/F82.csv"),
            Table::F83 => include_str!("../../data/fec-csv-sources/F83.csv"),
            Table::F9 => include_str!("../../data/fec-csv-sources/F9.csv"),
            Table::F91 => include_str!("../../data/fec-csv-sources/F91.csv"),
            Table::F92 => include_str!("../../data/fec-csv-sources/F92.csv"),
            Table::F93 => include_str!("../../data/fec-csv-sources/F93.csv"),
            Table::F94 => include_str!("../../data/fec-csv-sources/F94.csv"),
            Table::F99 => include_str!("../../data/fec-csv-sources/F99.csv"),
            Table::H1 => include_str!("../../data/fec-csv-sources/H1.csv"),
            Table::H2 => include_str!("../../data/fec-csv-sources/H2.csv"),
            Table::H3 => include_str!("../../data/fec-csv-sources/H3.csv"),
            Table::H4 => include_str!("../../data/fec-csv-sources/H4.csv"),
            Table::H5 => include_str!("../../data/fec-csv-sources/H5.csv"),
            Table::H6 => include_str!("../../data/fec-csv-sources/H6.csv"),
            Table::Hdr => include_str!("../../data/fec-csv-sources/HDR.csv"),
            Table::SchA => include_str!("../../data/fec-csv-sources/SchA.csv"),
            Table::SchA3L => include_str!("../../data/fec-csv-sources/SchA3L.csv"),
            Table::SchB => include_str!("../../data/fec-csv-sources/SchB.csv"),
            Table::SchC => include_str!("../../data/fec-csv-sources/SchC.csv"),
            Table::SchC1 => include_str!("../../data/fec-csv-sources/SchC1.csv"),
            Table::SchC2 => include_str!("../../data/fec-csv-sources/SchC2.csv"),
            Table::SchD => include_str!("../../data/fec-csv-sources/SchD.csv"),
            Table::SchE => include_str!("../../data/fec-csv-sources/SchE.csv"),
            Table::SchF => include_str!("../../data/fec-csv-sources/SchF.csv"),
            Table::SchI => include_str!("../../data/fec-csv-sources/SchI.csv"),
            Table::SchL => include_str!("../../data/fec-csv-sources/SchL.csv"),
            Table::Text => include_str!("../../data/fec-csv-sources/TEXT.csv"),
        }
    }
}

impl std::fmt::Display for Table {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Table {
    type Err = UnknownTable;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Table::from_name(s).ok_or_else(|| UnknownTable(s.to_string()))
    }
}

/// Error from parsing a `Table` from a string: the name is not a bundled format table.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("unknown format table '{0}'")]
pub struct UnknownTable(pub String);

/// `(table name, embedded CSV)` for every bundled format table, in the
/// same order as [`Table::ALL`]. Prefer [`Table::csv`] in new code.
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
    ("F2S", include_str!("../../data/fec-csv-sources/F2S.csv")),
    ("F3", include_str!("../../data/fec-csv-sources/F3.csv")),
    ("F3L", include_str!("../../data/fec-csv-sources/F3L.csv")),
    ("F3P", include_str!("../../data/fec-csv-sources/F3P.csv")),
    (
        "F3P31",
        include_str!("../../data/fec-csv-sources/F3P31.csv"),
    ),
    ("F3PS", include_str!("../../data/fec-csv-sources/F3PS.csv")),
    (
        "F3PZ1",
        include_str!("../../data/fec-csv-sources/F3PZ1.csv"),
    ),
    (
        "F3PZ2",
        include_str!("../../data/fec-csv-sources/F3PZ2.csv"),
    ),
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
    (
        "SchA3L",
        include_str!("../../data/fec-csv-sources/SchA3L.csv"),
    ),
    ("SchB", include_str!("../../data/fec-csv-sources/SchB.csv")),
    ("SchC", include_str!("../../data/fec-csv-sources/SchC.csv")),
    (
        "SchC1",
        include_str!("../../data/fec-csv-sources/SchC1.csv"),
    ),
    (
        "SchC2",
        include_str!("../../data/fec-csv-sources/SchC2.csv"),
    ),
    ("SchD", include_str!("../../data/fec-csv-sources/SchD.csv")),
    ("SchE", include_str!("../../data/fec-csv-sources/SchE.csv")),
    ("SchF", include_str!("../../data/fec-csv-sources/SchF.csv")),
    ("SchI", include_str!("../../data/fec-csv-sources/SchI.csv")),
    ("SchL", include_str!("../../data/fec-csv-sources/SchL.csv")),
    ("TEXT", include_str!("../../data/fec-csv-sources/TEXT.csv")),
];

/// Compile-time guard: `Table::ALL` and `FORM_CSV_DATA` were generated
/// from the same directory listing and must have the same length.
const _: () = assert!(Table::ALL.len() == FORM_CSV_DATA.len());
