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
//!   comma-delimited with CSV quoting applied only where needed. The
//!   version decides: the parser accepts a file whose header line is
//!   ASCII-28-delimited but declares a pre-6.0 version (it sniffs the
//!   delimiter from the header line), and such a file is written back in
//!   its version's comma format. A `[BEGINTEXT]` block in that
//!   inconsistent file is the one thing the comma format cannot carry:
//!   its line breaks become spaces.
//! * Every record is emitted with the full column count of its layout;
//!   trailing empty columns the original omitted (or padded) are
//!   normalised.
//! * Surrounding whitespace and one pair of wrapping quotes per field --
//!   both removed by the parser as wire conventions -- are not re-emitted.
//!   A value that itself begins and ends with `"` (the parser hands back
//!   `"-"` for the wire cell `""-""`) is written with one extra pair, so
//!   the re-parse strips exactly the pair the writer added.
//! * A Form 99's free text is emitted as a `[BEGINTEXT]`/`[ENDTEXT]` block
//!   after the cover line, with the cover's `text` column left blank, which
//!   is how the FEC's own software files it. Any other record whose `text`
//!   contains a line break or an ASCII-28 -- what the parser yields for a
//!   block that followed a body line, since block lines are not split on
//!   the delimiter -- is written the same way, so both survive; text
//!   without either stays inline. Spec 3.x-5.x filings predate the block
//!   convention (their parser reads `[BEGINTEXT]` as a record), so there
//!   text is always inline.
//! * Blank lines are dropped.
//! * A value can only contain what the parser can hand back. Anything
//!   structural for the file's own format is replaced with a space rather
//!   than corrupting the record: in an ASCII-28 file, an ASCII-28 or a
//!   line feed inside a value (the parser splits on both, so they can
//!   only arrive through [`ParsedLine::set`]); in a comma-delimited file,
//!   a line feed (records are one physical line). Everything else the
//!   parser hands back is written back verbatim so the round trip stays
//!   exact -- a bare carriage return in either format, and in a
//!   comma-delimited file an ASCII-28 inside a quoted value (the
//!   delimiter is decided by the header line alone, so a body value may
//!   contain one). The FEC's validator flags both as illegal characters
//!   either way.
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
        let blocks = self.version.uses_fs_delimiter();
        let mut w = LineWriter::for_version(blocks);
        w.write_header(&mut out, &self.header.to_fields());
        // The FEC's own software files a Form 99's text as a block even
        // when it is one line; every other record hoists only what inline
        // cannot carry.
        let f99_cover = self.summary.table() == Table::F99;
        write_record_with_text(&mut w, &mut out, &self.summary, blocks, f99_cover);
        for line in &self.lines {
            write_record_with_text(&mut w, &mut out, line, blocks, false);
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

/// Writes one record, hoisting its `text` into a `[BEGINTEXT]`/`[ENDTEXT]`
/// block after it when the format allows blocks (spec 6.0+) and either
/// `always` is set (the Form 99 cover) or the text has a line break or an
/// ASCII-28, which an inline column cannot carry (the parser splits
/// records on both but block lines on neither). The parser splices a
/// block back into the record it follows, so this is the exact inverse of
/// how the text was read.
fn write_record_with_text(
    w: &mut LineWriter,
    out: &mut String,
    record: &ParsedLine,
    blocks_allowed: bool,
    always: bool,
) {
    let hoisted = blocks_allowed
        .then(|| record.get_non_empty("text"))
        .flatten()
        .filter(|text| always || text.contains(['\n', NEW_DELIMITER]));
    match hoisted {
        Some(text) => {
            let mut cells = record.to_cells();
            if let Some(idx) = record.layout().field("text").map(|f| usize::from(f.column))
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
        None => w.write_record(out, &record.to_cells()),
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

    /// The first record of the file. The parser looks at the raw first
    /// bytes before any CSV parsing: a UTF-8 byte-order mark is stripped,
    /// and a file starting with `/*` (the pre-3.0 comment header) is
    /// rejected outright. A comma-delimited filing whose header cell was
    /// `"/*..."` or `"<BOM>..."` on the wire parses (the quotes hide the
    /// marker) and hands back a `record_type` beginning with it; written
    /// bare at byte 0 it would be rejected, or lose its first character,
    /// so it is written quoted, as the original was. (Both found by the
    /// `roundtrip` fuzz target.)
    fn write_header(&mut self, out: &mut String, cells: &[&str]) {
        let hides_file_marker = matches!(self, Self::Csv)
            && cells
                .first()
                .is_some_and(|c| c.starts_with("/*") || c.starts_with('\u{feff}'));
        self.write_cells(out, cells, hides_file_marker);
    }

    fn write_record(&mut self, out: &mut String, cells: &[&str]) {
        self.write_cells(out, cells, false);
    }

    /// One record. `force_quote_first` quotes the first cell even when
    /// CSV quoting would not require it (see [`LineWriter::write_header`]);
    /// it has no effect on the ASCII-28 format, which has no quoting.
    fn write_cells(&mut self, out: &mut String, cells: &[&str], force_quote_first: bool) {
        match self {
            Self::FileSeparator => {
                for (i, cell) in cells.iter().enumerate() {
                    if i > 0 {
                        out.push(NEW_DELIMITER);
                    }
                    let cell = wire_cell(cell);
                    // A delimiter or line feed inside a value would corrupt
                    // the record structure; neither can survive a parse
                    // (the parser splits on them), so replace with a space.
                    // A bare `\r` is data to the parser and stays.
                    push_sanitised(out, &cell, &[NEW_DELIMITER, '\n']);
                }
            }
            Self::Csv => {
                for (i, cell) in cells.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    push_csv_cell(out, cell, force_quote_first && i == 0);
                }
            }
        }
        out.push_str(CRLF);
    }
}

/// One comma-delimited cell, quoted when needed (or when `force_quote`).
/// `\r` and ASCII-28 are quoted (so a CSV consumer sees them as field
/// content) but kept: the per-line reader treats neither as structure, so
/// both survive a parse and must survive the write. Only a line feed
/// cannot be carried (one record per physical line) and could not have
/// been parsed.
fn push_csv_cell(out: &mut String, cell: &str, force_quote: bool) {
    let cell = wire_cell(cell);
    if force_quote || cell.contains([',', '"', '\r', '\n', NEW_DELIMITER]) {
        out.push('"');
        for ch in cell.chars() {
            match ch {
                '"' => out.push_str("\"\""),
                '\n' => out.push(' '),
                c => out.push(c),
            }
        }
        out.push('"');
    } else {
        out.push_str(&cell);
    }
}

/// The parser strips one pair of wrapping quotes from every cell as a
/// wire convention (`normalize_field`), so a value that begins and ends
/// with `"` must go out wrapped in one more pair, or the re-parse hands
/// back the value with its own quotes gone. The exact inverse: the parser
/// strips only a genuine pair (two or more characters), so that is the
/// only case wrapped here.
fn wire_cell(cell: &str) -> std::borrow::Cow<'_, str> {
    if cell.len() >= 2 && cell.starts_with('"') && cell.ends_with('"') {
        std::borrow::Cow::Owned(format!("\"{cell}\""))
    } else {
        std::borrow::Cow::Borrowed(cell)
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

    /// A `[BEGINTEXT]` block can follow a body line as well as the cover;
    /// the parser splices it into that line's `text` with the line breaks
    /// kept. The writer must put it back as a block, or the breaks become
    /// spaces and the round trip is lossy.
    #[test]
    fn text_block_after_a_body_line_is_written_back_as_a_block() {
        let text = [
            "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}\u{1c}",
            "F99\u{1c}C00944124\u{1c}REVIVE OREGON\u{1c}PO BOX 26141\u{1c}\u{1c}ALEXANDRIA\u{1c}VA\u{1c}22313\u{1c}MARSTON\u{1c}CHRIS\u{1c}\u{1c}\u{1c}\u{1c}20260914\u{1c}MST\u{1c}\u{1c}",
            "TEXT\u{1c}C00944124\u{1c}T1\u{1c}\u{1c}\u{1c}",
            "[BEGINTEXT]",
            "Line one of the note.",
            "Line two.",
            "[ENDTEXT]",
            "TEXT\u{1c}C00944124\u{1c}T2\u{1c}\u{1c}\u{1c}inline text stays inline",
        ]
        .join("\n");
        let filing = Filing::parse(&text).unwrap();
        assert_eq!(
            filing.lines[0].get("text"),
            Some("Line one of the note.\nLine two.")
        );

        // Block lines are not split on the delimiter, so a block can carry
        // an ASCII-28 that an inline column cannot; a one-line block with
        // one must stay a block. (Found by the `roundtrip` fuzz target.)
        let with_fs = "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nTEXT\u{1c}C00944124\u{1c}T1\n[BEGINTEXT]\nsplit\u{1c}here\n[ENDTEXT]\n";
        let f = Filing::parse(with_fs).unwrap();
        assert_eq!(f.summary.get("text"), Some("split\u{1c}here"));
        assert!(
            f.to_fec_string()
                .contains("[BEGINTEXT]\r\nsplit\u{1c}here\r\n[ENDTEXT]")
        );
        assert_round_trip(&f);
        let out = filing.to_fec_string();
        assert!(
            out.contains(
                "\r\n[BEGINTEXT]\r\nLine one of the note.\r\nLine two.\r\n[ENDTEXT]\r\nTEXT\u{1c}"
            ),
            "the block must follow its record: {out:?}"
        );
        // The record's own text column is blank on the wire, and a
        // single-line text stays inline.
        let records: Vec<&str> = out.split(CRLF).collect();
        assert!(records[2].starts_with("TEXT\u{1c}C00944124\u{1c}T1\u{1c}"));
        assert!(!records[2].contains("Line one"));
        assert!(
            records
                .iter()
                .any(|r| r.ends_with("inline text stays inline"))
        );
        assert_round_trip(&filing);
        assert_eq!(Filing::parse(&out).unwrap().to_fec_string(), out);
    }

    /// Pre-6.0 filings predate the block convention: the parser reads a
    /// `[BEGINTEXT]` line as a (bad) record there, so the writer must keep
    /// a 5.x Form 99's text inline.
    #[test]
    fn comma_delimited_f99_text_stays_inline() {
        let text = "HDR,FEC,5.3,SoftCo,1.0\nF99,C00123456,SOME PAC,1 MAIN ST,,TOWN,VA,22313,SMITH,20260914,MST,The whole explanation on one line.\n";
        let filing = Filing::parse(text).unwrap();
        assert_eq!(
            filing.summary.get("text"),
            Some("The whole explanation on one line.")
        );
        let out = filing.to_fec_string();
        assert!(!out.contains("[BEGINTEXT]"), "{out:?}");
        assert!(
            out.contains(",The whole explanation on one line."),
            "{out:?}"
        );
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
        // The reverse ambiguity: `Ã©` is two Windows-1252 characters whose
        // bytes (C3 A9) are also valid UTF-8 for `é`. The parser decodes
        // UTF-8 first, so those bytes would come back as `é`; the encoder
        // notices and emits UTF-8 instead.
        assert_eq!(encode("\u{c3}\u{a9}"), "\u{c3}\u{a9}".as_bytes());
        // Round trip through the parser's decoder.
        for s in [
            "caf\u{e9}",
            "O\u{2019}Neil",
            "\u{4e2d}\u{6587}",
            "mixed \u{e9} \u{4e2d}",
            "\u{c3}\u{a9}",
            "\u{c2}\u{a0}",
        ] {
            assert_eq!(crate::parser::filing::decode(&encode(s)), s, "{s:?}");
        }
        // A UTF-8 BOM is not part of the header token.
        assert_eq!(
            crate::parser::filing::decode(b"\xEF\xBB\xBFHDR\x1cFEC"),
            "HDR\u{1c}FEC"
        );
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

    /// In a comma-delimited filing the delimiter is decided by the header
    /// line alone, so an ASCII-28 inside a body value is data the parser
    /// hands back; the writer must hand it back too. (Found by the
    /// `roundtrip` fuzz target on a mutated 3.00 fixture.)
    #[test]
    fn ascii_28_inside_a_comma_delimited_value_round_trips() {
        let text = "HDR,FEC,5.3,X,1\r\nF3XN,C00123456,\"odd\u{1c}name\"\r\nSA11AI,C00123456,,,,IND,\"Bank\u{1c}of Amory\"\r\n";
        let filing = Filing::parse(text).unwrap();
        assert_eq!(filing.summary.get("committee_name"), Some("odd\u{1c}name"));
        let out = filing.to_fec_string();
        assert!(
            out.contains("\"odd\u{1c}name\""),
            "the ASCII-28 must be written back, quoted: {out:?}"
        );
        let again = Filing::parse(&out).unwrap();
        assert_eq!(again.summary.get("committee_name"), Some("odd\u{1c}name"));
        assert_eq!(again.version, filing.version, "still comma-delimited");
        assert_round_trip(&filing);
    }

    /// The parser rejects a file whose first bytes are `/*` (the pre-3.0
    /// comment header) before any CSV parsing, so a header cell that was
    /// `"/*..."` on the wire -- which parses, the quotes hiding the marker
    /// -- must be written quoted again. (Found by the `roundtrip` fuzz
    /// target on CI's first real run.)
    #[test]
    fn a_quoted_header_cell_hiding_a_file_marker_stays_quoted() {
        let filing = Filing::parse("\"/*R\",FEC,5.3,X,1\nF3XN,C00123456,NAME\n").unwrap();
        assert_eq!(filing.header.record_type, "/*R");
        let out = filing.to_fec_string();
        assert!(out.starts_with("\"/*R\",FEC,5.3"), "{out:?}");
        assert_round_trip(&filing);

        // A byte-order mark inside the quotes is data; bare at byte 0 the
        // parser would strip it.
        let bom = Filing::parse("\"\u{feff}HDR\",FEC,5.3,X,1\nF3XN,C00123456,NAME\n").unwrap();
        assert_eq!(bom.header.record_type, "\u{feff}HDR");
        assert!(
            bom.to_fec_string().starts_with("\"\u{feff}HDR\",FEC"),
            "{:?}",
            bom.to_fec_string()
        );
        assert_round_trip(&bom);

        // An ordinary header is untouched.
        let plain = Filing::parse("HDR,FEC,5.3,X,1\nF3XN,C00123456,NAME\n").unwrap();
        assert!(plain.to_fec_string().starts_with("HDR,FEC,5.3"));
    }

    /// A bare carriage return inside a value is what the parser yields for
    /// one on the wire (only `\n` ends a record), so the writer must hand
    /// it back unchanged on both delimiter paths.
    #[test]
    fn bare_carriage_return_inside_a_value_round_trips() {
        for (text, field) in [
            (
                "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\r\nF3XN\u{1c}C00123456\u{1c}odd\rname\r\n",
                "committee_name",
            ),
            (
                "HDR,FEC,5.3,X,1\r\nF3XN,C00123456,\"odd\rname\"\r\n",
                "committee_name",
            ),
        ] {
            let filing = Filing::parse(text).unwrap();
            assert_eq!(filing.summary.get(field), Some("odd\rname"), "{text:?}");
            let again = Filing::parse_bytes(&filing.to_fec()).unwrap();
            assert_eq!(again.summary.get(field), Some("odd\rname"), "{text:?}");
            assert_round_trip(&filing);
        }
    }

    /// A value the parser hands back as itself once its own wrapping pair
    /// (if any) is written and stripped: quote-wrapped values are what the
    /// wire cell `""x""` parses to, so the round trip must carry them.
    #[test]
    fn quote_wrapped_values_round_trip_in_both_formats() {
        // Found by the `roundtrip` fuzz target: a Schedule B purpose of
        // `"-"` (with its quotes) came back as `-`. `wire` is the cell as
        // an ASCII-28 file carries it; `csv` the same cell CSV-quoted.
        for (wire, csv, value) in [
            ("\"\"-\"\"", "\"\"\"\"\"-\"\"\"\"\"", "\"-\""),
            ("\"\"\"\"", "\"\"\"\"\"\"\"\"\"\"", "\"\""),
            // Starts with a quote but does not end with one: no pair to
            // strip, so the value is the wire cell itself.
            ("\"Bob\" Smith", "\"\"\"Bob\"\" Smith\"", "\"Bob\" Smith"),
            ("\"", "\"\"\"\"", "\""),
            ("abc\"", "\"abc\"\"\"", "abc\""),
        ] {
            let fs =
                format!("HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\nF3XN\u{1c}C00123456\u{1c}{wire}\n");
            let filing = Filing::parse(&fs).unwrap();
            assert_eq!(
                filing.summary.get("committee_name"),
                Some(value),
                "{wire:?}"
            );
            assert_round_trip(&filing);

            let old = Filing::parse(&format!("HDR,FEC,5.3,X,1\nF3XN,C00123456,{csv}\n")).unwrap();
            assert_eq!(old.summary.get("committee_name"), Some(value), "{csv:?}");
            assert_round_trip(&old);
        }
    }

    /// Values that survive the parser's normalisation unchanged: no
    /// delimiter/CR/LF (structural), no leading/trailing whitespace, and
    /// no characters the FEC forbids. Quote-wrapped values are included:
    /// the writer re-wraps them so the parser's one-pair strip lands on
    /// the pair the writer added.
    fn wire_safe_value() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[ -~\u{a0}-\u{a8}\u{ad}]{0,40}")
            .expect("valid regex")
            .prop_map(|s| s.trim().to_string())
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
                    ("memo_text", memo_text.as_str()),
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
