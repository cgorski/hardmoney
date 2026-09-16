//! Shared by the fuzz targets (`#[path]`-included, so each binary stays a
//! single file for cargo-fuzz).

#![allow(dead_code)]

use hardmoney::{Filing, ParsedLine};

/// Field-for-field equality of two parses of the *same bytes*: the list
/// `tests/stream_fixtures.rs` and the streaming parser's own tests
/// compare, line numbers included. `Filing` itself is not `PartialEq`.
pub fn same_filing(a: &Filing, b: &Filing) -> bool {
    a.header == b.header
        && a.version == b.version
        && a.raw_form_type == b.raw_form_type
        && a.base_form_type == b.base_form_type
        && a.is_amendment == b.is_amendment
        && a.amends_filing == b.amends_filing
        && a.summary == b.summary
        && a.lines == b.lines
}

/// What a record *says*, independent of where it sat in the file: the
/// comparison the writer guarantees (`src/parser/writer.rs`, `same_records`
/// in its tests). Canonical output drops blank lines, so physical line
/// numbers legitimately shift.
pub fn same_record_content(a: &ParsedLine, b: &ParsedLine) -> bool {
    a.raw_form_type == b.raw_form_type && a.table() == b.table() && a.iter().eq(b.iter())
}

/// [`same_filing`] without line numbers: for a parse of the writer's
/// canonical output against the parse it came from.
pub fn same_filing_content(a: &Filing, b: &Filing) -> bool {
    a.header == b.header
        && a.version == b.version
        && a.raw_form_type == b.raw_form_type
        && a.base_form_type == b.base_form_type
        && a.is_amendment == b.is_amendment
        && a.amends_filing == b.amends_filing
        && same_record_content(&a.summary, &b.summary)
        && a.lines.len() == b.lines.len()
        && a.lines
            .iter()
            .zip(&b.lines)
            .all(|(x, y)| same_record_content(x, y))
}

/// Whether the eager and streaming parsers are documented to decode
/// `bytes` identically. The eager parser decodes the whole file (UTF-8,
/// else Windows-1252 for everything); the streaming reader, which never
/// sees the whole file, decides per line. They agree when the file is
/// valid UTF-8, or when every line that is valid UTF-8 on its own is pure
/// ASCII (so decoding it as Windows-1252 gives the same text). A file
/// that mixes non-ASCII UTF-8 lines with invalid bytes on other lines is
/// the one documented divergence (`src/parser/stream.rs`, module docs;
/// the book's "Encoding, per line"), not a bug the fuzzer should report.
pub fn decoders_agree(bytes: &[u8]) -> bool {
    std::str::from_utf8(bytes).is_ok()
        || bytes
            .split(|&b| b == b'\n')
            .all(|line| std::str::from_utf8(line).map_or(true, str::is_ascii))
}

/// Whether the file's delimiter agrees with its declared version. The
/// parser sniffs the delimiter from the header line; the writer follows
/// the version. A header line that is ASCII-28-delimited but declares a
/// pre-6.0 version (or the reverse) is accepted on the way in and
/// canonicalised to the version's format on the way out, which cannot
/// carry a `[BEGINTEXT]` block's line breaks -- documented in
/// `src/parser/writer.rs`, and not something a real filer produces.
pub fn delimiter_matches_version(bytes: &[u8], filing: &Filing) -> bool {
    let header_line = bytes.split(|&b| b == b'\n').next().unwrap_or(&[]);
    header_line.contains(&0x1c) == filing.version.uses_fs_delimiter()
}

/// The first line that differs, for a readable crash artifact.
pub fn first_difference(a: &Filing, b: &Filing) -> String {
    if a.header != b.header {
        return format!("header: {:?} vs {:?}", a.header, b.header);
    }
    if a.summary != b.summary {
        return format!(
            "cover: {:?} vs {:?}",
            a.summary.to_cells(),
            b.summary.to_cells()
        );
    }
    if a.lines.len() != b.lines.len() {
        return format!("{} vs {} body lines", a.lines.len(), b.lines.len());
    }
    for (x, y) in a.lines.iter().zip(&b.lines) {
        if x != y {
            return format!(
                "line {}: {:?} vs {:?}",
                x.line_no,
                x.to_cells(),
                y.to_cells()
            );
        }
    }
    "metadata (version/form type/amendment)".to_string()
}
