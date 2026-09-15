//! Dispatch from a raw line's `form_type` token (column 0, e.g. `"SA11AI"`,
//! `"F3XN"`, `"SC/10"`) to the [`Table`] whose format table parses it.
//!
//! Two independent lookups are involved (do not confuse them):
//!   1. Here: `form_type` token -> [`Table`] (e.g. "SA11AI" -> `Table::SchA`).
//!   2. [`Table::layout`]: spec version -> column layout within that table.
//!
//! Order is load-bearing: several patterns are prefixes of each other, and
//! the first match wins. The entries cover every bundled table, including
//! Form 1/1M/1S/2/2S (registrations), Form 3Z/3Z1/3Z2 and 3P31/3PZ1/3PZ2
//! (consolidated candidate-committee sub-forms), Form 8/8II/8III and
//! 10/10.5 (historical), and Schedule I (Levin funds). Before the
//! registration forms were added, every Form 1/2 filing on the FEC's live
//! feed (~26% of daily volume) failed to parse.

use std::sync::LazyLock;

use regex::Regex;

use crate::parser::tables::Table;

/// Top-level filing form types this crate processes end-to-end. These are
/// the forms that can appear on a filing's *summary* (second) line; body
/// lines (schedules, sub-forms, `TEXT` records) are dispatched separately.
///
pub static ALLOWED_TOP_LEVEL_FORMS: &[&str] = &[
    "F3", "F3X", "F3P", "F9", "F5", "F24", "F6", "F7", "F4", "F3L", "F13", "F99", "F1", "F1M",
    "F2", "F8", "F10",
];

#[must_use]
pub fn is_allowed_top_level_form(base_form: &str) -> bool {
    ALLOWED_TOP_LEVEL_FORMS.contains(&base_form.to_uppercase().as_str())
}

/// `(form_type_regex, table)` pairs, tried in order; the first match wins.
///
/// Every pattern is compiled as `(?i)^(?:{pattern})` -- case-insensitive
/// (real filings contain e.g. `SB21b`) and start-anchored, with
/// alternations safely grouped.
static DISPATCH_ORDER: &[(&str, Table)] = &[
    (r"SA3L", Table::SchA3L),
    (r"SA", Table::SchA),
    (r"SB", Table::SchB),
    (r"SC1", Table::SchC1),
    (r"SC2", Table::SchC2),
    (r"SC", Table::SchC),
    (r"SD", Table::SchD),
    (r"SE", Table::SchE),
    (r"SF", Table::SchF),
    // Schedule I (Levin-fund account summary), spec 3.x-8.4; dropped in 8.5.
    (r"SI", Table::SchI),
    (r"F3X[ANT]", Table::F3X),
    // F3P sub-forms. None of these match `F3P[ANT]` ('3'/'Z' are not in the
    // class) so position is not load-bearing; kept adjacent for readability.
    (r"F3P31", Table::F3P31),
    (r"F3PZ1", Table::F3PZ1),
    (r"F3PZ2", Table::F3PZ2),
    (r"F3P[ANT]", Table::F3P),
    (r"F3S", Table::F3S),
    // F3Z family: real filings carry `F3Z` (one per authorized committee)
    // and `F3ZT` (the consolidated total); F3Z1/F3Z2 existed in 8.2-8.4 only.
    (r"F3Z1", Table::F3Z1),
    (r"F3Z2", Table::F3Z2),
    (r"F3ZT?$", Table::F3Z),
    (r"F3[ANT]$", Table::F3),
    (r"F91", Table::F91),
    (r"F92", Table::F92),
    (r"F93", Table::F93),
    (r"F94", Table::F94),
    (r"F99", Table::F99),
    (r"F9", Table::F9),
    (r"F6[AN]*$", Table::F6),
    (r"F65", Table::F65),
    (r"F57", Table::F57),
    (r"F56", Table::F56),
    (r"F5", Table::F5),
    (r"TEXT", Table::Text),
    (r"F24", Table::F24),
    (r"H1", Table::H1),
    (r"H2", Table::H2),
    (r"H3", Table::H3),
    (r"H4", Table::H4),
    (r"H5", Table::H5),
    (r"H6", Table::H6),
    (r"SL", Table::SchL),
    (r"F3PS", Table::F3PS),
    (r"F76$", Table::F76),
    (r"F7[AN]$", Table::F7),
    (r"F4[ANT]", Table::F4),
    (r"F3L[AN]", Table::F3L),
    (r"F13[AN]$", Table::F13),
    (r"F132", Table::F132),
    (r"F133", Table::F133),
    // Form 1 family. `F1[AN]$` is end-anchored so it cannot shadow F13x/F10x.
    (r"F1S", Table::F1S),
    (r"F1M[AN]?$", Table::F1M),
    (r"F1[AN]$", Table::F1),
    // Form 2 family. `F2[AN]?$` cannot match F24x or F2S. F2S has its own
    // table (authored locally; fech-sources has none, and fecfile's `^f2[^4]`
    // mis-parses F2S lines with the F2 layout).
    (r"F2S$", Table::F2S),
    (r"F2[AN]?$", Table::F2),
    // Form 8 (debt settlement plan, spec 5.x/6.x). The fech-sources table
    // names are F82/F83 but the on-the-wire tokens are F8II/F8III (per the
    // FEC spec and fecfile, whose field lists are identical). Accept both.
    (r"F8II$|F82$", Table::F82),
    (r"F8III$|F83$", Table::F83),
    (r"F8[AN]?$", Table::F8),
    // Form 10 / 10.5 (candidate personal-funds notices, abolished 2008).
    (r"F105$", Table::F105),
    (r"F10[AN]?$", Table::F10),
];

struct CompiledDispatch {
    regex: Regex,
    table: Table,
}

/// Compiled dispatch table. A pattern that fails to compile is skipped here
/// (so `Filing::parse` can never panic on it) and caught by the
/// `every_dispatch_pattern_compiles` test instead.
static DISPATCH: LazyLock<Vec<CompiledDispatch>> = LazyLock::new(|| {
    DISPATCH_ORDER
        .iter()
        .filter_map(|(pattern, table)| {
            Regex::new(&format!("(?i)^(?:{pattern})"))
                .ok()
                .map(|regex| CompiledDispatch {
                    regex,
                    table: *table,
                })
        })
        .collect()
});

/// Finds which [`Table`] a raw `form_type` token (column 0 of a line, e.g.
/// `"SA11AI"`) should be parsed with.
#[must_use]
pub fn table_for_form_type(form_type: &str) -> Option<Table> {
    DISPATCH
        .iter()
        .find(|d| d.regex.is_match(form_type.trim()))
        .map(|d| d.table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_dispatch_pattern_compiles() {
        assert_eq!(DISPATCH.len(), DISPATCH_ORDER.len());
    }

    #[test]
    fn every_table_except_hdr_is_reachable_from_dispatch() {
        // HDR is the header record, parsed by `header.rs`, never by dispatch.
        let unreachable: Vec<_> = Table::ALL
            .iter()
            .filter(|&&t| t != Table::Hdr)
            .filter(|&&t| !DISPATCH_ORDER.iter().any(|(_, target)| *target == t))
            .collect();
        assert!(
            unreachable.is_empty(),
            "format tables with no dispatch entry: {unreachable:?}"
        );
    }

    #[test]
    fn dispatch_order_prefers_specific_over_general() {
        let cases = [
            ("SA3L", Table::SchA3L),
            ("SA11AI", Table::SchA),
            ("sb21b", Table::SchB),
            ("SC/10", Table::SchC),
            ("SC1", Table::SchC1),
            ("SC2", Table::SchC2),
            ("SI", Table::SchI),
            ("SL", Table::SchL),
            ("F3XN", Table::F3X),
            ("F3N", Table::F3),
            ("F3S", Table::F3S),
            ("F3PS", Table::F3PS),
            ("F3P31", Table::F3P31),
            ("F3PZ1", Table::F3PZ1),
            ("F3Z", Table::F3Z),
            ("F3ZT", Table::F3Z),
            ("F3Z1", Table::F3Z1),
            ("F76", Table::F76),
            ("F7N", Table::F7),
            ("F6", Table::F6),
            ("F6N", Table::F6),
            ("F65", Table::F65),
            ("TEXT", Table::Text),
        ];
        for (token, expected) in cases {
            assert_eq!(table_for_form_type(token), Some(expected), "{token}");
        }
    }

    #[test]
    fn registration_and_historical_forms_dispatch() {
        let cases = [
            ("F1N", Table::F1),
            ("F1A", Table::F1),
            ("F1S", Table::F1S),
            ("F1MN", Table::F1M),
            ("F1MA", Table::F1M),
            ("F1M", Table::F1M),
            ("F2N", Table::F2),
            ("F2A", Table::F2),
            ("F2", Table::F2),
            ("F2S", Table::F2S),
            ("F24N", Table::F24),
            ("F24A", Table::F24),
            ("F8", Table::F8),
            ("F8N", Table::F8),
            ("F8II", Table::F82),
            ("F82", Table::F82),
            ("F8III", Table::F83),
            ("F83", Table::F83),
            ("F10", Table::F10),
            ("F105", Table::F105),
            ("F13N", Table::F13),
            ("F132", Table::F132),
            ("F133", Table::F133),
        ];
        for (token, expected) in cases {
            assert_eq!(table_for_form_type(token), Some(expected), "{token}");
        }
    }

    #[test]
    fn unknown_form_type_is_none() {
        assert_eq!(table_for_form_type("ZZZ"), None);
        assert_eq!(table_for_form_type(""), None);
    }

    #[test]
    fn allowed_top_level_forms_include_registrations() {
        for f in [
            "F3X", "f3x", "F3", "F3P", "F99", "F1", "F1M", "F2", "F8", "F10",
        ] {
            assert!(is_allowed_top_level_form(f), "{f}");
        }
        for f in ["F1S", "F2S", "SchA", "SA", "F3Z", "ZZZ"] {
            assert!(!is_allowed_top_level_form(f), "{f}");
        }
    }
}
