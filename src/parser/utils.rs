//! Field-level normalisation shared by the header, line, and filing parsers.
//!
//! # Fidelity policy (changed in 2.0)
//!
//! hardmoney 1.x inherited nyt-pyfec's `clean_entry`/`utf8_clean`, which
//! upper-cased every field and silently deleted `&`, `<`, `>`, `"`, `\` and
//! turned `|` into `,`. That made `AT&T` come out as `ATT` and made a
//! byte-faithful writer impossible. Since 2.0 the parser preserves every
//! field **verbatim except for two wire-format conventions**:
//!
//! * surrounding ASCII whitespace is trimmed (the FEC's own ingestion trims
//!   it, and several historical vendors padded fields to fixed widths);
//! * one pair of surrounding double quotes is removed (`"BRABANT"` ->
//!   `BRABANT`, `""` -> empty). Some vendors -- CMDI Crimson Filer among
//!   them -- quote every text field even in ASCII-28-delimited files, and
//!   the FEC accepts and strips them (its validator's message #26 is
//!   "Invalid double-quote surround text field", #30 "Embedded double-quotes
//!   not allowed"). A lone or unbalanced quote is data and is kept.
//!
//! Anything that needs to *interpret* a value (entity-type codes, memo
//! flags, form-type tokens) does so case-insensitively at the point of
//! interpretation instead.

/// Applies the wire-format normalisation described in the module docs:
/// trims leading/trailing ASCII whitespace (space, tab, CR, LF, FF, VT),
/// then removes one pair of surrounding double quotes if present, then
/// trims again. Everything else is returned untouched.
///
/// Only ASCII whitespace is trimmed: a non-breaking space or other Unicode
/// whitespace inside filer-entered text is data, not padding.
#[must_use]
pub fn normalize_field(entry: &str) -> &str {
    let trimmed = entry.trim_matches(|c: char| c.is_ascii_whitespace());
    match trimmed
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        // `"` alone would strip to itself via prefix then fail the suffix,
        // so only genuinely paired quotes reach here.
        Some(inner) => inner.trim_matches(|c: char| c.is_ascii_whitespace()),
        None => trimmed,
    }
}

/// A raw form-type token, upper-cased for stable comparison, e.g. `"sb21b"`
/// -> `"SB21B"`. The spec defines these tokens as case-insensitive and real
/// filings contain both cases.
#[must_use]
pub fn normalize_form_type(token: &str) -> String {
    normalize_field(token).to_ascii_uppercase()
}

/// Whether a `memo_code` field marks the line as a memo entry.
///
/// Memo entries (`memo_code == "X"`) are informational and **excluded** from
/// every cover-page total; forgetting this is the #1 reason recomputed
/// totals disagree with a filing's cover page. Case-insensitive, trimmed.
#[must_use]
pub fn is_memo_code(value: &str) -> bool {
    normalize_field(value).eq_ignore_ascii_case("X")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_field_trims_only_ascii_whitespace() {
        assert_eq!(normalize_field("  hello world  "), "hello world");
        assert_eq!(normalize_field("\tAT&T\r"), "AT&T");
        assert_eq!(
            normalize_field("O'Brien \"Bob\" <b>"),
            "O'Brien \"Bob\" <b>"
        );
        assert_eq!(normalize_field("a|b\\c^d"), "a|b\\c^d");
        assert_eq!(normalize_field("\u{a0}x\u{a0}"), "\u{a0}x\u{a0}");
        assert_eq!(normalize_field(""), "");
        assert_eq!(normalize_field("   "), "");
    }

    #[test]
    fn normalize_field_preserves_case() {
        assert_eq!(normalize_field("FECfile"), "FECfile");
        assert_eq!(normalize_field("Smith, Jane"), "Smith, Jane");
    }

    #[test]
    fn normalize_field_strips_exactly_one_pair_of_surrounding_quotes() {
        assert_eq!(normalize_field("\"BRABANT\""), "BRABANT");
        assert_eq!(normalize_field("\"\""), "");
        assert_eq!(normalize_field(" \"SD12\" "), "SD12");
        assert_eq!(normalize_field("\" padded \""), "padded");
        // Only one pair; nested quoting is data.
        assert_eq!(normalize_field("\"\"x\"\""), "\"x\"");
        // Lone or unbalanced quotes are data.
        assert_eq!(normalize_field("\""), "\"");
        assert_eq!(normalize_field("\"abc"), "\"abc");
        assert_eq!(normalize_field("abc\""), "abc\"");
        assert_eq!(normalize_field("say \"hi\" now"), "say \"hi\" now");
    }

    #[test]
    fn normalize_form_type_uppercases() {
        assert_eq!(normalize_form_type(" sb21b "), "SB21B");
        assert_eq!(normalize_form_type("F3XA"), "F3XA");
        assert_eq!(normalize_form_type("sc/10"), "SC/10");
    }

    #[test]
    fn memo_code_detection() {
        assert!(is_memo_code("X"));
        assert!(is_memo_code("x"));
        assert!(is_memo_code(" X "));
        assert!(!is_memo_code(""));
        assert!(!is_memo_code("Y"));
        assert!(!is_memo_code("XX"));
    }
}
