//! Direct port of the entry-cleaning helpers from nyt-pyfec's `pyfec/utils.py`.
//!
//! `clean_entry` is applied to *every* field of *every* line, mirroring the
//! upstream comment "NOTE THIS IS RUN ON EVERY SINGLE ENTRY -- Optimize
//! whenever possible."

/// Port of `pyfec.utils.clean_entry`:
/// `entry.strip().replace("^"," ").replace('"', "").upper().strip()`
pub fn clean_entry(entry: &str) -> String {
    entry
        .trim()
        .replace('^', " ")
        .replace('"', "")
        .to_uppercase()
        .trim()
        .to_string()
}

/// Port of `pyfec.utils.utf8_clean`.
///
/// The original strips a fixed set of Latin-1/Windows-1252 control and
/// punctuation bytes (interpreted here as the equivalent Unicode code points
/// once decoded), and translates several characters that would break simple
/// delimited/CSV ingestion (newlines, curly quotes, em dash, pipe, backslash)
/// into plain ASCII equivalents.
pub fn utf8_clean(raw: &str) -> String {
    const TO_REMOVE: &[char] = &[
        '\u{A5}', '\u{A0}', '\u{22}', '\u{26}', '\u{3C}', '\u{3E}', '\u{A1}', '\u{A2}', '\u{A3}',
        '\u{A4}', '\u{A6}', '\u{A7}', '\u{A8}', '\u{A9}', '\u{AA}', '\u{AB}', '\u{AC}', '\u{AD}',
        '\u{AE}', '\u{AF}', '\u{B0}', '\u{B1}', '\u{B2}', '\u{B3}', '\u{B4}', '\u{B5}', '\u{B6}',
        '\u{B7}', '\u{B8}', '\u{B9}', '\u{BA}', '\u{BB}', '\u{BC}', '\u{BD}', '\u{BE}', '\u{BF}',
        '\u{D7}', '\u{F7}', '\u{95}', '\u{96}', '\u{98}', '\u{99}', '\t',
    ];

    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        if TO_REMOVE.contains(&ch) {
            continue;
        }
        let translated = match ch {
            '\n' => ' ',
            '\u{85}' => '.',
            '\u{91}' => '\'',
            '\u{92}' => '\'',
            '\u{93}' => '"',
            '\u{94}' => '"',
            '\u{97}' => '-',
            '|' => ',',
            '\\' => ' ',
            other => other,
        };
        out.push(translated);
    }
    out
}

/// Port of `pyfec.utils.get_cycle`: pads a year to the next even number and
/// returns it as a string (election cycles are even years). Returns `None`
/// for unparsable input or a year outside a sane range, matching the
/// Python `ValueError` -> `None` path without the possibility of overflow.
pub fn get_cycle(year: &str) -> Option<String> {
    let this_year: i64 = year.trim().parse().ok()?;
    let padded = if this_year.rem_euclid(2) != 0 {
        this_year.checked_add(1)?
    } else {
        this_year
    };
    Some(padded.to_string())
}

/// Port of `pyfec.utils.get_four_digit_year`: expands a two-digit year,
/// treating anything up to ten years past `current_year` as 20xx and the
/// rest as 19xx. Returns `None` for unparsable input or values that are
/// not actually two-digit.
pub fn get_four_digit_year(two_digit: &str, current_year: i32) -> Option<String> {
    let two_digit_year: i32 = two_digit.trim().parse().ok()?;
    if !(0..=99).contains(&two_digit_year) {
        return None;
    }
    let this_year_2digit = current_year.rem_euclid(100);
    let four_digit = if two_digit_year <= this_year_2digit.checked_add(10)? {
        2000_i32.checked_add(two_digit_year)?
    } else {
        1900_i32.checked_add(two_digit_year)?
    };
    Some(four_digit.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_entry_matches_pyfec() {
        assert_eq!(clean_entry("  hello^world\"  "), "HELLO WORLD");
        assert_eq!(clean_entry("F3XA"), "F3XA");
        assert_eq!(clean_entry(""), "");
    }

    #[test]
    fn utf8_clean_strips_and_translates() {
        assert_eq!(utf8_clean("a\nb"), "a b");
        assert_eq!(utf8_clean("a\tb"), "ab");
        assert_eq!(utf8_clean("a|b"), "a,b");
        assert_eq!(utf8_clean("a\\b"), "a b");
    }

    #[test]
    fn get_cycle_pads_odd_years() {
        assert_eq!(get_cycle("2023"), Some("2024".to_string()));
        assert_eq!(get_cycle("2024"), Some("2024".to_string()));
        assert_eq!(get_cycle("abc"), None);
        // Extreme values must not panic (they used to overflow in debug).
        assert_eq!(get_cycle("9223372036854775807"), None);
        assert_eq!(get_cycle("-1"), Some("0".to_string()));
    }

    #[test]
    fn get_four_digit_year_expands_without_overflow() {
        assert_eq!(get_four_digit_year("24", 2026), Some("2024".to_string()));
        assert_eq!(get_four_digit_year("36", 2026), Some("2036".to_string()));
        assert_eq!(get_four_digit_year("37", 2026), Some("1937".to_string()));
        assert_eq!(get_four_digit_year("99", 2026), Some("1999".to_string()));
        assert_eq!(get_four_digit_year("100", 2026), None);
        assert_eq!(get_four_digit_year("2147483647", 2026), None);
        assert_eq!(get_four_digit_year("x", 2026), None);
    }
}
