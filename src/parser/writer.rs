//! `Filing::to_fec`: the inverse of parsing.
//!
//! A [`Filing`] can be serialised back to the `.fec` wire format. Because
//! every [`ParsedLine`] remembers the [`Layout`](crate::parser::Layout) it
//! was parsed with, each field is written back at exactly the column the
//! spec version assigns it, and `parse(to_fec(parse(f)))` yields the same
//! header, cover line, and body lines as `parse(f)` for every filing the
//! parser accepts (this is tested over every real fixture and with
//! property tests over arbitrary field values).
//!
//! # What is and is not preserved
//!
//! The output is *canonical*, not byte-identical:
//!
//! * Line endings are always `CRLF` (the FEC's convention).
//! * Spec 6.0+ filings use the ASCII-28 delimiter; 3.x-5.x filings are
//!   comma-delimited with CSV quoting applied only where needed.
//! * Every record is emitted with the full column count of its layout;
//!   trailing empty columns the original omitted (or padded) are
//!   normalised.
//! * Surrounding whitespace and one pair of wrapping quotes per field --
//!   both removed by the parser as wire conventions -- are not re-emitted.
//! * A Form 99's free text is emitted as a `[BEGINTEXT]`/`[ENDTEXT]` block
//!   after the cover line, with the cover's `text` column left blank, which
//!   is how the FEC's own software files it.
//! * Blank lines are dropped.
//!
//! # Encoding
//!
//! The FEC's character set is single-byte: ASCII 32-126 plus Latin-1
//! 128-168 and 173. [`Filing::to_fec`] therefore encodes as Windows-1252
//! when every character is representable, and as UTF-8 otherwise -- the
//! mirror image of the parser's "UTF-8 first, Windows-1252 fallback"
//! decoding, so a round trip always reproduces the same characters.
//! [`Filing::to_fec_string`] gives the text before encoding.

use std::io::{self, Write};

use crate::parser::filing::{Filing, NEW_DELIMITER, ParsedLine};
use crate::parser::tables::Table;

/// CRLF, the FEC's record terminator.
const CRLF: &str = "\r\n";

impl Filing {
    /// Serialises the filing to `.fec` text (see the module docs for what
    /// is canonicalised). Never fails: every line carries its layout.
    #[must_use]
    pub fn to_fec_string(&self) -> String {
        let mut out = String::new();
        let mut w = LineWriter::for_version(self.version.uses_fs_delimiter());
        w.write_record(&mut out, &self.header.to_fields());
        write_cover(&mut w, &mut out, &self.summary);
        for line in &self.lines {
            w.write_record(&mut out, &line.to_cells());
        }
        out
    }

    /// Serialises the filing to `.fec` bytes, Windows-1252 when every
    /// character is representable in it (the FEC's character set), else
    /// UTF-8. See the module docs.
    #[must_use]
    pub fn to_fec(&self) -> Vec<u8> {
        encode(&self.to_fec_string())
    }

    /// Writes [`Filing::to_fec`] to `writer`.
    pub fn write_fec<W: Write>(&self, mut writer: W) -> io::Result<()> {
        writer.write_all(&self.to_fec())
    }
}

/// Encodes canonical text as Windows-1252 if it can represent every
/// character *and* the result is not itself valid UTF-8 that would decode
/// differently (so the parser's UTF-8-first decoding cannot misread it);
/// otherwise UTF-8.
fn encode(text: &str) -> Vec<u8> {
    let (cow, _, had_unmappable) = encoding_rs::WINDOWS_1252.encode(text);
    if had_unmappable {
        return text.as_bytes().to_vec();
    }
    if text.is_ascii() {
        // Identical in both encodings.
        return cow.into_owned();
    }
    match std::str::from_utf8(&cow) {
        // Non-ASCII Windows-1252 bytes that also happen to be valid UTF-8
        // would decode to different characters; fall back to UTF-8.
        Ok(decoded) if decoded != text => text.as_bytes().to_vec(),
        _ => cow.into_owned(),
    }
}

/// Writes the cover line, hoisting a Form 99's free text into a
/// `[BEGINTEXT]` block.
fn write_cover(w: &mut LineWriter, out: &mut String, cover: &ParsedLine) {
    let free_text = (cover.table() == Table::F99)
        .then(|| cover.get_non_empty("text"))
        .flatten();
    match free_text {
        Some(text) => {
            let mut cells = cover.to_cells();
            if let Some(idx) = cover.layout().field("text").map(|f| usize::from(f.column))
                && let Some(slot) = cells.get_mut(idx)
            {
                *slot = "";
            }
            w.write_record(out, &cells);
            out.push_str("[BEGINTEXT]");
            out.push_str(CRLF);
            for line in text.split('\n') {
                out.push_str(line.trim_end_matches('\r'));
                out.push_str(CRLF);
            }
            out.push_str("[ENDTEXT]");
            out.push_str(CRLF);
        }
        None => w.write_record(out, &cover.to_cells()),
    }
}

/// Delimiter-specific record formatting.
enum LineWriter {
    /// ASCII-28 delimited (spec 6.0+). Fields are written as-is.
    FileSeparator,
    /// Comma delimited with CSV quoting (spec 3.x-5.x).
    Csv,
}

impl LineWriter {
    fn for_version(uses_fs: bool) -> Self {
        if uses_fs {
            Self::FileSeparator
        } else {
            Self::Csv
        }
    }

    fn write_record(&mut self, out: &mut String, cells: &[&str]) {
        match self {
            Self::FileSeparator => {
                for (i, cell) in cells.iter().enumerate() {
                    if i > 0 {
                        out.push(NEW_DELIMITER);
                    }
                    // A delimiter or line break inside a value would corrupt
                    // the record structure; they cannot survive a parse
                    // (the parser splits on them), so replace with a space.
                    push_sanitised(out, cell, &[NEW_DELIMITER, '\r', '\n']);
                }
            }
            Self::Csv => {
                for (i, cell) in cells.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    if cell.contains([',', '"', '\r', '\n']) {
                        out.push('"');
                        for ch in cell.chars() {
                            match ch {
                                '"' => out.push_str("\"\""),
                                '\r' | '\n' => out.push(' '),
                                c => out.push(c),
                            }
                        }
                        out.push('"');
                    } else {
                        out.push_str(cell);
                    }
                }
            }
        }
        out.push_str(CRLF);
    }
}

fn push_sanitised(out: &mut String, cell: &str, forbidden: &[char]) {
    if cell.contains(forbidden) {
        out.extend(
            cell.chars()
                .map(|c| if forbidden.contains(&c) { ' ' } else { c }),
        );
    } else {
        out.push_str(cell);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::SpecVersion;
    use proptest::prelude::*;

    fn same_records(a: &ParsedLine, b: &ParsedLine) -> bool {
        a.raw_form_type == b.raw_form_type && a.table() == b.table() && a.iter().eq(b.iter())
    }

    fn assert_round_trip(filing: &Filing) {
        let bytes = filing.to_fec();
        let again = Filing::parse_bytes(&bytes).unwrap_or_else(|e| panic!("re-parse: {e}"));
        assert_eq!(filing.header, again.header);
        assert_eq!(filing.version, again.version);
        assert_eq!(filing.raw_form_type, again.raw_form_type);
        assert!(
            same_records(&filing.summary, &again.summary),
            "cover differs"
        );
        assert_eq!(filing.lines.len(), again.lines.len());
        for (a, b) in filing.lines.iter().zip(&again.lines) {
            assert!(
                same_records(a, b),
                "line {} differs:\n{a:?}\n{b:?}",
                a.line_no
            );
        }
    }

    #[test]
    fn fs_delimited_filing_round_trips_and_is_canonical() {
        let text = [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}\u{1c}\u{1c}",
            "F3XN\u{1c}C00123456\u{1c}  \"Friends of AT&T\"  \u{1c}",
            "",
            "sb21b\u{1c}C00123456\u{1c}T1\u{1c}\u{1c}\u{1c}ORG\u{1c}Vendor, Inc.",
        ]
        .join("\n");
        let filing = Filing::parse(&text).unwrap();
        let out = filing.to_fec_string();
        // CRLF, full width, trimmed/unquoted values, upper-cased token kept as filed.
        assert!(out.ends_with(CRLF));
        let lines: Vec<&str> = out.split(CRLF).filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 3);
        assert!(lines[0].starts_with("HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile"));
        assert!(lines[1].starts_with("F3XN\u{1c}C00123456\u{1c}Friends of AT&T\u{1c}"));
        let width = Table::F3X
            .layout(SpecVersion::electronic(8, 5))
            .unwrap()
            .width;
        assert_eq!(lines[1].split(NEW_DELIMITER).count(), usize::from(width));
        assert!(lines[2].starts_with("sb21b\u{1c}C00123456\u{1c}T1\u{1c}"));
        assert_round_trip(&filing);
        // Idempotent: writing the re-parsed filing gives identical bytes.
        assert_eq!(Filing::parse(&out).unwrap().to_fec_string(), out);
    }

    #[test]
    fn f99_free_text_is_written_as_a_block() {
        let text = [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}\u{1c}",
            "F99\u{1c}C00944124\u{1c}REVIVE OREGON\u{1c}PO BOX 26141\u{1c}\u{1c}ALEXANDRIA\u{1c}VA\u{1c}22313\u{1c}MARSTON\u{1c}CHRIS\u{1c}\u{1c}\u{1c}\u{1c}20260914\u{1c}MST\u{1c}\u{1c}",
            "[BEGINTEXT]",
            "First paragraph, with mixed Case.",
            "",
            "Second paragraph.",
            "[ENDTEXT]",
        ]
        .join("\n");
        let filing = Filing::parse(&text).unwrap();
        let out = filing.to_fec_string();
        assert!(out.contains("[BEGINTEXT]\r\nFirst paragraph, with mixed Case.\r\n\r\nSecond paragraph.\r\n[ENDTEXT]\r\n"));
        // The cover's own text column is blank on the wire.
        let cover_line = out.split(CRLF).nth(1).unwrap();
        assert!(!cover_line.contains("First paragraph"));
        assert_round_trip(&filing);
    }

    #[test]
    fn comma_delimited_filing_round_trips_with_quoting() {
        let text = "HDR,FEC,5.3,SoftCo,1.0,^,FEC-24088,1\nF3XA,C00123456,\"Merck PAC, The Political Action Committee of Merck & Co., Inc.\"\nSA11AI,C00123456,,,,IND,\"O'Brien \"\"Bob\"\", Jr.\"\n";
        let filing = Filing::parse(text).unwrap();
        let out = filing.to_fec_string();
        assert!(
            out.contains(",\"Merck PAC, The Political Action Committee of Merck & Co., Inc.\",")
        );
        assert!(!out.contains(NEW_DELIMITER));
        assert_round_trip(&filing);
        assert_eq!(filing.amends_filing, Some(24088));
    }

    #[test]
    fn encoding_is_windows_1252_when_representable_else_utf8() {
        assert_eq!(encode("plain ascii"), b"plain ascii");
        // é is 0xE9 in Windows-1252; a single byte, invalid as UTF-8.
        assert_eq!(encode("caf\u{e9}"), b"caf\xe9");
        // Curly apostrophe is 0x92 in Windows-1252.
        assert_eq!(encode("O\u{2019}Neil"), b"O\x92Neil");
        // A character outside Windows-1252 forces UTF-8.
        assert_eq!(encode("\u{4e2d}"), "\u{4e2d}".as_bytes());
        // Round trip through the parser's decoder.
        for s in [
            "caf\u{e9}",
            "O\u{2019}Neil",
            "\u{4e2d}\u{6587}",
            "mixed \u{e9} \u{4e2d}",
        ] {
            assert_eq!(crate::parser::filing::decode(&encode(s)), s, "{s:?}");
        }
    }

    #[test]
    fn delimiter_and_line_breaks_inside_values_cannot_corrupt_structure() {
        let mut filing =
            Filing::parse("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456\u{1c}NAME")
                .unwrap();
        filing
            .summary
            .set("committee_name", "bad\u{1c}name\nwith break")
            .unwrap();
        let again = Filing::parse(&filing.to_fec_string()).unwrap();
        assert_eq!(
            again.summary.get("committee_name"),
            Some("bad name with break")
        );
        assert_eq!(again.lines.len(), 0);
    }

    /// Values that survive the parser's normalisation unchanged: no
    /// delimiter/CR/LF (structural), no leading/trailing whitespace, not
    /// wrapped in a quote pair (wire conventions the parser strips), and
    /// no characters the FEC forbids.
    fn wire_safe_value() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[ -~\u{a0}-\u{a8}\u{ad}]{0,40}")
            .expect("valid regex")
            .prop_map(|s| s.trim().to_string())
            .prop_filter("not quote-wrapped", |s| {
                !(s.len() >= 2 && s.starts_with('"') && s.ends_with('"'))
            })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        #[test]
        fn arbitrary_schedule_a_values_round_trip(
            last in wire_safe_value(),
            org in wire_safe_value(),
            employer in wire_safe_value(),
            memo_text in wire_safe_value(),
            amount in -99_999_999i64..99_999_999i64,
        ) {
            let amount_s = format!("{}.{:02}", amount / 100, (amount % 100).abs());
            let line = ParsedLine::from_pairs(
                Table::SchA,
                SpecVersion::electronic(8, 5),
                3,
                [
                    ("form_type", "SA11AI"),
                    ("filer_committee_id_number", "C00123456"),
                    ("contributor_last_name", last.as_str()),
                    ("contributor_organization_name", org.as_str()),
                    ("contributor_employer", employer.as_str()),
                    ("memo_text_description", memo_text.as_str()),
                    ("contribution_amount", amount_s.as_str()),
                ],
            )
            .unwrap();
            let mut filing = Filing::parse(
                "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456\u{1c}NAME",
            )
            .unwrap();
            filing.lines.push(line);
            let again = Filing::parse_bytes(&filing.to_fec()).unwrap();
            prop_assert_eq!(again.lines.len(), 1);
            prop_assert!(same_records(&filing.lines[0], &again.lines[0]));
        }

        #[test]
        fn arbitrary_values_round_trip_through_csv_quoting(
            name in wire_safe_value(),
            street in wire_safe_value(),
        ) {
            let mut filing = Filing::parse("HDR,FEC,5.3,X,1\nF3XN,C00123456,NAME\n").unwrap();
            filing.summary.set("committee_name", &name).unwrap();
            filing.summary.set("street_1", &street).unwrap();
            let again = Filing::parse_bytes(&filing.to_fec()).unwrap();
            prop_assert!(same_records(&filing.summary, &again.summary));
        }
    }
}
