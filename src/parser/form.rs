//! Port of `pyfec/form.py`: dispatches a raw line's `form_type` string (e.g.
//! `"SA11AI"`, `"F3XN"`, `"SC/10"`) to the right format table (`Line`), and
//! tracks which *top-level* filing form types this port actually handles.
//!
//! Two independent regex layers are involved (do not confuse them):
//!   1. Here: `form_type` string -> format table name (e.g. "SA11AI" -> "SchA").
//!   2. In `line.rs`: `fec_version` string -> column-position bucket within
//!      that table.

use once_cell::sync::Lazy;
use regex::Regex;
use std::collections::HashMap;

use crate::parser::error::{FecError, Result};
use crate::parser::line::Line;

/// Top-level filing form types this port processes end-to-end (i.e. for
/// which [`crate::parser::filing::Filing::summary`] knows how to build a
/// [`crate::parser::typed::Form3XSummary`]). This exactly matches nyt-pyfec's
/// `Form.allowed_forms` -- notably it does NOT include Form 1/1S/2/2S
/// (committee/candidate registration), which pyfec never supported either.
pub static ALLOWED_TOP_LEVEL_FORMS: &[&str] = &[
    "F3", "F3X", "F3P", "F9", "F5", "F24", "F6", "F7", "F4", "F3L", "F13", "F99",
];

pub fn is_allowed_top_level_form(base_form: &str) -> bool {
    ALLOWED_TOP_LEVEL_FORMS.contains(&base_form.to_uppercase().as_str())
}

/// `(form_type_regex, format_table_name)` pairs, in the EXACT order
/// nyt-pyfec's `regex_tuple` checked them. Order is load-bearing: several
/// regexes are prefixes of each other (e.g. `^SA3L` vs `^SA`, `^F3X...` vs
/// `^F3...$`), so the more specific pattern must be tried first.
///
/// Note on `[A|N|T]`: in the original Python this is a *character class*,
/// where `|` is a literal character, not alternation -- so it matches one
/// of the four characters `A`, `|`, `N`, `T`. Since a literal pipe never
/// appears in a real form_type, this is equivalent in practice to `[ANT]`,
/// which is what we use here.
static DISPATCH_ORDER: &[(&str, &str)] = &[
    (r"^SA3L", "SchA3L"),
    (r"^SA", "SchA"),
    (r"^SB", "SchB"),
    (r"^SC1", "SchC1"),
    (r"^SC2", "SchC2"),
    (r"^SC", "SchC"),
    (r"^SD", "SchD"),
    (r"^SE", "SchE"),
    (r"^SF", "SchF"),
    (r"^F3X[ANT]", "F3X"),
    (r"^F3P[ANT]", "F3P"),
    (r"^F3S", "F3S"),
    (r"^F3[ANT]$", "F3"),
    (r"^F91", "F91"),
    (r"^F92", "F92"),
    (r"^F93", "F93"),
    (r"^F94", "F94"),
    (r"^F99", "F99"),
    (r"^F9", "F9"),
    (r"^F6[AN]*$", "F6"),
    (r"^F65", "F65"),
    (r"^F57", "F57"),
    (r"^F56", "F56"),
    (r"^F5", "F5"),
    (r"^TEXT", "TEXT"),
    (r"^F24", "F24"),
    (r"^H1", "H1"),
    (r"^H2", "H2"),
    (r"^H3", "H3"),
    (r"^H4", "H4"),
    (r"^H5", "H5"),
    (r"^H6", "H6"),
    (r"^SL", "SchL"),
    (r"^F3PS", "F3PS"),
    (r"^F76$", "F76"),
    (r"^F7[AN]$", "F7"),
    (r"^F4[ANT]", "F4"),
    (r"^F3L[AN]", "F3L"),
    (r"^F13[AN]$", "F13"),
    (r"^F132", "F132"),
    (r"^F133", "F133"),
];

struct CompiledDispatch {
    regex: Regex,
    table: &'static str,
}

static DISPATCH: Lazy<Vec<CompiledDispatch>> = Lazy::new(|| {
    DISPATCH_ORDER
        .iter()
        .map(|(pattern, table)| {
            // re.I (case-insensitive) in the original; `(?i)` reproduces it.
            let anchored = format!("(?i)^(?:{})", pattern);
            CompiledDispatch {
                regex: Regex::new(&anchored).unwrap_or_else(|e| {
                    panic!("invalid built-in dispatch regex '{pattern}': {e}")
                }),
                table,
            }
        })
        .collect()
});

/// Lazily-parsed registry of every format table this port knows about,
/// built once from the embedded fec-csv-sources CSVs.
static TABLES: Lazy<HashMap<&'static str, Line>> = Lazy::new(|| {
    let mut map = HashMap::new();
    for (name, csv) in crate::parser::format_data::FORM_CSV_DATA {
        match Line::from_csv_str(name, csv) {
            Ok(line) => {
                map.insert(*name, line);
            }
            Err(e) => panic!("failed to parse built-in format table '{name}': {e}"),
        }
    }
    map
});

/// Finds which format table a raw `form_type` string (column 0 of a line,
/// e.g. `"SA11AI"`) should be parsed with.
pub fn table_name_for_form_type(form_type: &str) -> Option<&'static str> {
    DISPATCH
        .iter()
        .find(|d| d.regex.is_match(form_type))
        .map(|d| d.table)
}

/// Returns the [`Line`] parser for a raw `form_type` string, mirroring
/// `Form.get_line_parser` + the `ParserMissingError` path of
/// `Form.parse_form_line`.
pub fn line_parser_for_form_type(form_type: &str) -> Result<&'static Line> {
    let table_name = table_name_for_form_type(form_type).ok_or_else(|| FecError::ParserMissing {
        form_type: form_type.to_string(),
        version: String::new(),
    })?;
    Ok(TABLES.get(table_name).expect("dispatch table always registered"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_order_prefers_specific_over_general() {
        assert_eq!(table_name_for_form_type("SA3L"), Some("SchA3L"));
        assert_eq!(table_name_for_form_type("SA11AI"), Some("SchA"));
        assert_eq!(table_name_for_form_type("SC/10"), Some("SchC"));
        assert_eq!(table_name_for_form_type("SC1"), Some("SchC1"));
        assert_eq!(table_name_for_form_type("SC2"), Some("SchC2"));
        assert_eq!(table_name_for_form_type("F3XN"), Some("F3X"));
        assert_eq!(table_name_for_form_type("F3N"), Some("F3"));
        assert_eq!(table_name_for_form_type("F3S"), Some("F3S"));
        assert_eq!(table_name_for_form_type("F76"), Some("F76"));
        assert_eq!(table_name_for_form_type("F7N"), Some("F7"));
    }

    #[test]
    fn all_dispatch_targets_resolve_to_a_real_table() {
        for (_, table) in DISPATCH_ORDER {
            assert!(TABLES.contains_key(table), "missing table for {table}");
        }
    }

    #[test]
    fn unknown_form_type_errors() {
        assert!(line_parser_for_form_type("ZZZ").is_err());
    }

    #[test]
    fn allowed_top_level_forms_matches_pyfec() {
        assert!(is_allowed_top_level_form("F3X"));
        assert!(is_allowed_top_level_form("f3x"));
        assert!(!is_allowed_top_level_form("F1"));
        assert!(!is_allowed_top_level_form("F2"));
    }
}
