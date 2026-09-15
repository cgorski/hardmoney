//! The static description of the `.fec` format: spec versions, per-version
//! column layouts, and the FEC's per-field specifications.
//!
//! Everything here is data, not behaviour. The values are generated at
//! build time (see `build.rs`) from `data/fec-csv-sources/` (column
//! positions across every spec version since 2001) and
//! `data/fec-spec/spec-*.json` (the FEC's own field types, lengths,
//! required levels, and rule text for the current version), and exposed
//! through [`Table`] and the per-table modules in [`crate::parser::tables`].
//!
//! # Compile-time-checked field access
//!
//! Every table has a zero-sized marker type (e.g. [`markers::F3X`]) and one
//! [`Field<Marker>`] constant per canonical field
//! (e.g. `tables::f3x::COL_A_TOTAL_RECEIPTS`). A [`Typed<'_, F3X>`] view
//! only accepts `Field<F3X>` keys, so asking an F3X cover page for a
//! Schedule A field is a **compile error** rather than a silent `None`:
//!
//! ```
//! use hardmoney::parser::tables::{f3x, markers::F3X};
//! # use hardmoney::Filing;
//! # let text = "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456";
//! let filing = Filing::parse(text).unwrap();
//! let cover = filing.summary.typed::<F3X>().unwrap();
//! let receipts = cover.money(f3x::COL_A_TOTAL_RECEIPTS); // Option<Decimal>
//! # let _ = receipts;
//! ```
//!
//! [`markers::F3X`]: crate::parser::tables::markers::F3X

use std::fmt;
use std::marker::PhantomData;
use std::str::FromStr;

use chrono::NaiveDate;
use rust_decimal::Decimal;

use crate::parser::filing::ParsedLine;
use crate::parser::tables::Table;
use crate::parser::typed::{TypedViewError, parse_fec_date, parse_money};

// ---------------------------------------------------------------------------
// SpecVersion
// ---------------------------------------------------------------------------

/// An FEC electronic-filing format version, e.g. `8.5`, `3.00`, or the
/// paper-conversion variants `P3.4`.
///
/// Versions are compared numerically (`8.5 > 8.4 > 7.0 > 6.4`), with all
/// paper versions ordering after all electronic ones. The wire spelling is
/// not preserved (`3.00` and `3.0` are the same version); keep the raw
/// header string if you need to reproduce it byte-for-byte.
///
/// Only the first digit of the minor component is significant, which is
/// how the FEC has always numbered releases (`8.5.0.1` is a build of `8.5`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(into = "String", try_from = "String"))]
pub struct SpecVersion {
    paper: bool,
    major: u8,
    minor: u8,
}

/// Why a string is not a [`SpecVersion`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("'{0}' is not an FEC spec version (expected e.g. '8.5', '3.00', or 'P3.4')")]
pub struct InvalidSpecVersion(pub String);

impl SpecVersion {
    /// An electronic version, e.g. `SpecVersion::electronic(8, 5)` for 8.5.
    #[must_use]
    pub const fn electronic(major: u8, minor: u8) -> Self {
        Self {
            paper: false,
            major,
            minor,
        }
    }

    /// A paper-conversion version, e.g. `SpecVersion::paper(3, 4)` for P3.4.
    #[must_use]
    pub const fn paper(major: u8, minor: u8) -> Self {
        Self {
            paper: true,
            major,
            minor,
        }
    }

    /// True for `P`-prefixed paper-conversion versions.
    #[must_use]
    pub const fn is_paper(self) -> bool {
        self.paper
    }

    /// The major component (`8` for 8.5, `3` for `3.00`).
    #[must_use]
    pub const fn major(self) -> u8 {
        self.major
    }

    /// The first digit of the minor component (`5` for 8.5, `0` for
    /// `3.00`, `5` for `8.5.0.1`).
    #[must_use]
    pub const fn minor(self) -> u8 {
        self.minor
    }

    /// Electronic versions 6.0 and later delimit fields with ASCII 28; 3.x
    /// through 5.x used a comma with CSV quoting.
    #[must_use]
    pub const fn uses_fs_delimiter(self) -> bool {
        !self.paper && self.major >= 6
    }

    /// Electronic versions 3.x-5.x carried a `name_delim` header column.
    #[must_use]
    pub const fn has_name_delim_header(self) -> bool {
        !self.paper && self.major <= 5
    }
}

impl FromStr for SpecVersion {
    type Err = InvalidSpecVersion;

    /// Accepts `M`, `M.m`, `M.mm…`, `M.m.x.y` and the same with a leading
    /// `P`/`p`; surrounding whitespace is ignored. Only the first minor
    /// digit is kept (`3.00` -> 3.0, `8.5.0.1` -> 8.5).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = || InvalidSpecVersion(s.to_string());
        let t = s.trim();
        let (paper, rest) = match t.strip_prefix(['P', 'p']) {
            Some(r) => (true, r),
            None => (false, t),
        };
        let mut parts = rest.split('.');
        let major_s = parts.next().ok_or_else(err)?;
        if major_s.is_empty() || !major_s.bytes().all(|b| b.is_ascii_digit()) {
            return Err(err());
        }
        let major: u8 = major_s.parse().map_err(|_| err())?;
        let minor = match parts.next() {
            None => 0,
            Some(m) => {
                if m.is_empty() || !m.bytes().all(|b| b.is_ascii_digit()) {
                    return Err(err());
                }
                // Every byte is an ASCII digit, so the first one is in
                // b'0'..=b'9' and the subtraction cannot underflow; the
                // `checked_sub` merely makes that visible to the reader.
                m.bytes()
                    .next()
                    .and_then(|b| b.checked_sub(b'0'))
                    .ok_or_else(err)?
            }
        };
        // Anything after the minor component is a build number; it must be
        // numeric but is otherwise ignored.
        for extra in parts {
            if extra.is_empty() || !extra.bytes().all(|b| b.is_ascii_digit()) {
                return Err(err());
            }
        }
        Ok(Self {
            paper,
            major,
            minor,
        })
    }
}

impl fmt::Display for SpecVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.paper {
            f.write_str("P")?;
        }
        write!(f, "{}.{}", self.major, self.minor)
    }
}

impl From<SpecVersion> for String {
    fn from(v: SpecVersion) -> Self {
        v.to_string()
    }
}

impl TryFrom<String> for SpecVersion {
    type Error = InvalidSpecVersion;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        s.parse()
    }
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// One canonical field's position within a [`Layout`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDef {
    /// Canonical (lower_snake_case) field name, stable across versions.
    pub name: &'static str,
    /// 0-based column in the delimited record.
    pub column: u16,
}

/// The column layout of one [`Table`] for a set of spec versions.
///
/// `fields` is in table order (the order the FEC lists them); `by_name`
/// indexes `fields` sorted by name so [`Layout::index_of`] is a binary
/// search. `width` is one more than the highest column used, i.e. the
/// number of delimited cells a writer must emit.
#[derive(Debug)]
pub struct Layout {
    /// The table this layout belongs to.
    pub table: Table,
    /// Every spec version this layout applies to (never empty; disjoint
    /// from every other layout of the same table).
    pub versions: &'static [SpecVersion],
    /// The fields present at these versions, in table order, each with its
    /// 0-based column. Columns are distinct; not every column below
    /// `width` need be named.
    pub fields: &'static [FieldDef],
    /// One more than the highest column: the number of cells a writer
    /// emits for a record of this layout.
    pub width: u16,
    #[doc(hidden)]
    pub by_name: &'static [u16],
}

impl Layout {
    /// Whether this layout applies to `version`.
    #[must_use]
    pub fn supports(&self, version: SpecVersion) -> bool {
        self.versions.contains(&version)
    }

    /// Index into [`Layout::fields`] of the field called `name`.
    #[must_use]
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.by_name
            .binary_search_by(|&i| {
                self.fields
                    .get(usize::from(i))
                    .map_or(std::cmp::Ordering::Less, |f| f.name.cmp(name))
            })
            .ok()
            .and_then(|k| self.by_name.get(k))
            .map(|&i| usize::from(i))
    }

    /// The field definition for `name`, if this layout has it.
    #[must_use]
    pub fn field(&self, name: &str) -> Option<&'static FieldDef> {
        self.index_of(name).and_then(|i| self.fields.get(i))
    }

    /// The name of the field at column 0 -- the record's dispatch token:
    /// `form_type` on every table except `TEXT`, whose token field is
    /// `rec_type`. `None` only if a layout had no column-0 field, which the
    /// bundled data never does.
    #[must_use]
    pub fn token_field(&self) -> Option<&'static str> {
        self.fields.iter().find(|f| f.column == 0).map(|f| f.name)
    }

    /// The FEC specification for `name` at the bundled spec version, if the
    /// field still exists there.
    #[must_use]
    pub fn spec(&self, name: &str) -> Option<&'static FieldSpec> {
        self.table
            .specs()
            .iter()
            .find(|s| s.canonical == Some(name))
    }
}

impl Table {
    /// The column layout for this table at `version`, if the bundled data
    /// covers it.
    #[must_use]
    pub fn layout(self, version: SpecVersion) -> Option<&'static Layout> {
        self.layouts().iter().find(|l| l.supports(version))
    }

    /// Whether the bundled data has a layout for this table at `version`.
    #[must_use]
    pub fn supports_version(self, version: SpecVersion) -> bool {
        self.layout(version).is_some()
    }

    /// The FEC specification for a field of this table at the bundled spec
    /// version.
    #[must_use]
    pub fn spec(self, name: &str) -> Option<&'static FieldSpec> {
        self.specs().iter().find(|s| s.canonical == Some(name))
    }
}

// ---------------------------------------------------------------------------
// FieldSpec
// ---------------------------------------------------------------------------

/// The data type of a field as the FEC's spec declares it (`TYPE` column).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display, strum::EnumString)]
#[strum(serialize_all = "snake_case")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
#[non_exhaustive]
pub enum FieldKind {
    /// `A-n`: letters only (state codes, single-letter flags).
    Alpha,
    /// `A/N-n`: any printable text up to `n` characters.
    AlphaNumeric,
    /// `NUM-n` / `N-n`: digits only. Eight-digit numerics are `YYYYMMDD` dates.
    Numeric,
    /// `AMT-12`: a dollar amount, optionally signed, at most two decimals.
    Amount,
    /// The spec row had no parseable type.
    Unknown,
}

/// Whether the FEC requires a field, and how hard it fails when missing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(
    feature = "serde",
    serde(rename_all = "snake_case", tag = "level", content = "condition")
)]
#[non_exhaustive]
pub enum Requirement {
    /// Optional.
    None,
    /// `X (error)`: a blank value makes the filing unacceptable.
    Error,
    /// `X (warning)`: a blank value is accepted with a warning.
    Warning,
    /// Required only under the quoted condition from the spec, e.g.
    /// `"X (warn if REPORT CODE=[12?|30?])"`.
    Conditional(&'static str),
}

impl Requirement {
    /// True for [`Requirement::Error`] and [`Requirement::Warning`] -- the
    /// unconditional requirements a validator can check without context.
    #[must_use]
    pub const fn is_unconditional(self) -> bool {
        matches!(self, Requirement::Error | Requirement::Warning)
    }
}

/// The FEC's specification of one column of one table, from the
/// *Electronic Filing Specification Requirements, Part II* workbook.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct FieldSpec {
    /// 0-based column at the bundled spec version.
    pub column: u16,
    /// The canonical field name at that column, if the layout tables name
    /// it (a few spec columns are unnamed "dummy" positions).
    pub canonical: Option<&'static str>,
    /// The FEC's field description, e.g. `"CONTRIBUTOR ORGANIZATION NAME"`.
    pub description: &'static str,
    /// The declared data type (`A/N-200` -> [`FieldKind::AlphaNumeric`]).
    pub kind: FieldKind,
    /// Maximum length in characters, from the type (`A/N-200` -> 200).
    pub max_len: Option<u16>,
    /// Whether, and how hard, the FEC requires the field.
    pub required: Requirement,
    /// The spec's sample value, e.g. `"C00123456"`.
    pub sample: Option<&'static str>,
    /// Prose describing allowed values, e.g. `"[IND|ORG|COM]"`.
    pub value_reference: Option<&'static str>,
    /// Rule text, e.g. `"= 11ai + 11aii"` on a cover-page total.
    pub rule: Option<&'static str>,
    /// Parent forms this schedule column applies to (`FIELD-FORM ASSOCIATION`).
    pub forms: &'static [&'static str],
    /// Machine-readable allowed values, where the FEC publishes them.
    pub allowed_values: &'static [&'static str],
    /// A regex the value must match, where the FEC publishes one (e.g. the
    /// committee/candidate ID format).
    pub pattern: Option<&'static str>,
}

// ---------------------------------------------------------------------------
// Field<T> and Typed<'a, T>
// ---------------------------------------------------------------------------

/// Implemented by the generated zero-sized marker types in
/// [`crate::parser::tables::markers`]; ties a [`Field`] to its [`Table`].
pub trait TableMarker: Copy + fmt::Debug + 'static {
    const TABLE: Table;
}

/// A canonical field name statically tied to the table it belongs to.
///
/// Constants of this type are generated for every field of every table
/// (`tables::sch_a::CONTRIBUTION_AMOUNT: Field<SchA>`). Use them with
/// [`Typed`] for compile-time-checked access, or call [`Field::name`] to get
/// the plain string for the untyped [`ParsedLine::get`].
pub struct Field<T: TableMarker> {
    name: &'static str,
    _table: PhantomData<fn() -> T>,
}

impl<T: TableMarker> Field<T> {
    /// Names a field of `T`'s table. Prefer the generated constants; this
    /// exists so they can be `const`.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name,
            _table: PhantomData,
        }
    }

    /// The canonical field name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.name
    }

    /// The table this field belongs to.
    #[must_use]
    pub const fn table(self) -> Table {
        T::TABLE
    }

    /// The FEC's specification for this field at the bundled spec version.
    #[must_use]
    pub fn spec(self) -> Option<&'static FieldSpec> {
        T::TABLE.spec(self.name)
    }
}

impl<T: TableMarker> Clone for Field<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: TableMarker> Copy for Field<T> {}
impl<T: TableMarker> PartialEq for Field<T> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl<T: TableMarker> Eq for Field<T> {}
impl<T: TableMarker> std::hash::Hash for Field<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.name.hash(state);
    }
}
impl<T: TableMarker> fmt::Debug for Field<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", T::TABLE, self.name)
    }
}
impl<T: TableMarker> fmt::Display for Field<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name)
    }
}

/// A [`ParsedLine`] known (checked at construction) to belong to table `T`,
/// so that field access is checked against `T` at compile time.
///
/// Obtain one with [`ParsedLine::typed`]. All accessors return `None` for
/// blank fields and for fields this filing's spec version does not have.
#[derive(Clone, Copy)]
pub struct Typed<'a, T: TableMarker> {
    line: &'a ParsedLine,
    _table: PhantomData<fn() -> T>,
}

impl<'a, T: TableMarker> Typed<'a, T> {
    pub(crate) fn new(line: &'a ParsedLine) -> Result<Self, TypedViewError> {
        if line.table() != T::TABLE {
            return Err(TypedViewError::WrongTable {
                expected: T::TABLE,
                found: line.table(),
                line_no: line.line_no,
            });
        }
        Ok(Self {
            line,
            _table: PhantomData,
        })
    }

    /// The underlying line.
    #[must_use]
    pub const fn line(self) -> &'a ParsedLine {
        self.line
    }

    /// The raw (trimmed) value, `None` if blank or absent in this version.
    #[must_use]
    pub fn get(self, field: Field<T>) -> Option<&'a str> {
        self.line.get(field.name).filter(|v| !v.is_empty())
    }

    /// The value parsed as an exact dollar amount.
    #[must_use]
    pub fn money(self, field: Field<T>) -> Option<Decimal> {
        self.get(field).and_then(parse_money)
    }

    /// The value parsed as a `YYYYMMDD` date.
    #[must_use]
    pub fn date(self, field: Field<T>) -> Option<NaiveDate> {
        self.get(field).and_then(parse_fec_date)
    }

    /// The value as an owned `String`, `None` if blank.
    #[must_use]
    pub fn string(self, field: Field<T>) -> Option<String> {
        self.get(field).map(str::to_string)
    }
}

impl<T: TableMarker> fmt::Debug for Typed<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Typed")
            .field("table", &T::TABLE)
            .field("line_no", &self.line.line_no)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_version_parses_real_world_spellings() {
        for (raw, expected) in [
            ("8.5", SpecVersion::electronic(8, 5)),
            (" 8.5 ", SpecVersion::electronic(8, 5)),
            ("3.00", SpecVersion::electronic(3, 0)),
            ("3", SpecVersion::electronic(3, 0)),
            ("5.00", SpecVersion::electronic(5, 0)),
            ("8.5.0.1", SpecVersion::electronic(8, 5)),
            ("P3.4", SpecVersion::paper(3, 4)),
            ("p2.6", SpecVersion::paper(2, 6)),
            ("P1", SpecVersion::paper(1, 0)),
        ] {
            assert_eq!(raw.parse::<SpecVersion>().unwrap(), expected, "{raw:?}");
        }
        for bad in ["", "x", "8.", "1800.5", "8.a", "..", "P", "8.5.x", "-8.5"] {
            assert!(bad.parse::<SpecVersion>().is_err(), "{bad:?}");
        }
        // Well-formed but unknown to the FEC: parses here, rejected by
        // `Header::from_fields` (seen in a real corpus file).
        assert_eq!("180.5".parse::<SpecVersion>().unwrap().major(), 180);
    }

    #[test]
    fn spec_version_orders_numerically_with_paper_last() {
        let mut v = [
            SpecVersion::paper(2, 6),
            SpecVersion::electronic(8, 5),
            SpecVersion::electronic(3, 0),
            SpecVersion::electronic(6, 4),
            SpecVersion::electronic(7, 0),
        ];
        v.sort();
        assert_eq!(
            v.iter().map(ToString::to_string).collect::<Vec<_>>(),
            ["3.0", "6.4", "7.0", "8.5", "P2.6"]
        );
        assert!(SpecVersion::electronic(8, 5).uses_fs_delimiter());
        assert!(!SpecVersion::electronic(5, 3).uses_fs_delimiter());
        assert!(SpecVersion::electronic(5, 3).has_name_delim_header());
    }

    #[test]
    fn every_table_has_a_layout_and_lookups_agree() {
        for &t in Table::ALL {
            let layouts = t.layouts();
            assert!(!layouts.is_empty(), "{t} has no layouts");
            for l in layouts {
                assert_eq!(l.table, t);
                assert!(!l.versions.is_empty(), "{t}: empty version set");
                assert_eq!(l.by_name.len(), l.fields.len());
                for (i, f) in l.fields.iter().enumerate() {
                    assert_eq!(l.index_of(f.name), Some(i), "{t}: {}", f.name);
                    assert!(f.column < l.width);
                }
                assert_eq!(l.index_of("no_such_field"), None);
            }
        }
    }

    #[test]
    fn current_layout_columns_match_spec_columns() {
        // Every spec row that names a canonical field must agree with the
        // 8.5 layout on where that field lives.
        let v85 = SpecVersion::electronic(8, 5);
        for &t in Table::ALL {
            let Some(layout) = t.layout(v85) else {
                continue;
            };
            for s in t.specs() {
                if let Some(name) = s.canonical {
                    let def = layout.field(name).unwrap_or_else(|| panic!("{t}.{name}"));
                    assert_eq!(def.column, s.column, "{t}.{name}");
                }
            }
        }
    }

    #[test]
    fn field_constants_name_real_fields() {
        use crate::parser::tables::{f3x, sch_a};
        let l = Table::SchA.layout(SpecVersion::electronic(8, 5)).unwrap();
        assert!(l.field(sch_a::CONTRIBUTION_AMOUNT.name()).is_some());
        assert_eq!(sch_a::CONTRIBUTION_AMOUNT.table(), Table::SchA);
        assert_eq!(
            format!("{:?}", f3x::COL_A_TOTAL_RECEIPTS),
            "F3X.col_a_total_receipts"
        );
        let spec = f3x::COL_A_TOTAL_RECEIPTS.spec().expect("spec row");
        assert_eq!(spec.kind, FieldKind::Amount);
        assert_eq!(spec.rule, Some("= 19"));
    }
}
