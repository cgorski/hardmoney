//! Streaming (constant-memory) parsing of large filings.
//!
//! # Streaming vs. eager
//!
//! [`Filing::parse_bytes`] is the right tool for almost every filing: it
//! decodes the whole file and hands back every line at once in
//! [`Filing::lines`], which is what an API, a validator, or a writer needs.
//! A few filings are not "almost every filing". A presidential committee's
//! post-general F3P runs to 135 MB and 700,000 Schedule A lines, and a
//! parsed [`ParsedLine`] is larger than its wire form, so a job that only
//! needs one pass (sum Schedule A, copy Schedule E into a database, count
//! itemised lines) should not have to hold all of it.
//!
//! [`FilingReader`] is that one pass. It reads the header and cover line
//! eagerly -- they are always needed, and they decide how every other line
//! is parsed -- and then yields body lines one at a time from any
//! [`BufRead`], in file order, holding at most one record (plus one
//! `[BEGINTEXT]` block) in memory. [`Filing::open`] is the eager
//! convenience built on it: it streams a file from disk and collects the
//! result into a [`Filing`] identical to what [`Filing::parse_bytes`] would
//! produce from the same bytes.
//!
//! Use the eager parser when you need random access to the lines, the
//! writer, validation, or reconciliation. Use the streaming reader when
//! the filing is large and one forward pass is enough, or when the input
//! is not a file (a network body, a decompressing reader).
//!
//! # The one-record lookahead
//!
//! Form 99 filings carry their free text in a `[BEGINTEXT]` … `[ENDTEXT]`
//! block that *follows* the record it belongs to (see
//! [`crate::parser::filing`]). A record therefore cannot be yielded the
//! moment it is parsed, because the next line might be a text block that
//! has to be spliced into its `text` field. The reader holds each parsed
//! record back until it has seen the following line, so what you receive
//! is always complete. Blocks that directly follow the cover line -- the
//! common Form 99 case -- are consumed by [`FilingReader::new`], so
//! [`Preamble::summary`] is complete before the first body line is read.
//!
//! # Encoding
//!
//! Filings are UTF-8, or Windows-1252 when older software wrote
//! filer-entered text as raw bytes. The eager parser decides once for the
//! whole file; the streaming reader, which never sees the whole file,
//! decides per line (per record on the comma-delimited path). The two agree
//! on every fixture and on every filing that is consistently one encoding.
//! They differ only on a file that mixes valid multi-byte UTF-8 on some
//! lines with invalid bytes on others, where the streaming reader keeps
//! the UTF-8 lines intact and the eager parser re-reads them as
//! Windows-1252.
//!
//! # Errors
//!
//! Errors that concern the filing as a whole (a bad header, no cover line,
//! a `/*`-style pre-electronic file) come from [`FilingReader::new`].
//! Errors about one body line come from the iterator as `Some(Err(..))`,
//! after which it is exhausted: under [`ParseOptions::STRICT`] the first
//! unparseable line ends the read, exactly as it fails [`Filing::parse`].
//! Under [`ParseOptions::LENIENT`] such lines are recorded in
//! [`FilingReader::skipped`] instead and the read continues. I/O errors and
//! an unterminated `[BEGINTEXT]` block always end the read.
//!
//! # Example
//!
//! Total the non-memo Schedule A receipts of a filing without loading it:
//!
//! ```no_run
//! use std::fs::File;
//! use std::io::BufReader;
//!
//! use hardmoney::parser::stream::FilingReader;
//! use hardmoney::{ScheduleA, Table};
//! use rust_decimal::Decimal;
//!
//! fn main() -> hardmoney::Result<()> {
//!     let file = BufReader::new(File::open("2010101.fec")?);
//!     let reader = FilingReader::new(file)?.filter_tables([Table::SchA]);
//!     let cover = reader.preamble();
//!     println!("{} filed at spec {}", cover.raw_form_type, cover.version);
//!
//!     let mut total = Decimal::ZERO;
//!     for line in reader {
//!         let line = line?;
//!         if line.is_memo() {
//!             continue;
//!         }
//!         if let Ok(a) = line.view::<ScheduleA>() {
//!             total += a.contribution_amount.unwrap_or_default();
//!         }
//!     }
//!     println!("Schedule A total: {total}");
//!     Ok(())
//! }
//! ```

use std::borrow::Cow;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Cursor, Read};
use std::iter::FusedIterator;
use std::path::Path;

use crate::parser::error::{FecError, Result};
use crate::parser::filing::{
    Filing, Lenient, NEW_DELIMITER, OnUnparseableLine, ParseOptions, ParsedLine, SkipReason,
    SkippedLine, split_new_delimited, strip_ant_suffix,
};
use crate::parser::form;
use crate::parser::header::Header;
use crate::parser::schema::SpecVersion;
use crate::parser::tables::Table;
use crate::parser::utils::normalize_form_type;

/// Read buffer for [`Filing::open`]. The default 8 KiB `BufReader` means
/// ~16,000 syscalls for a 135 MB filing; 64 KiB is a comfortable middle
/// ground that is still far smaller than any parsed record set.
const OPEN_BUFFER_BYTES: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Preamble
// ---------------------------------------------------------------------------

/// Everything a [`Filing`] carries apart from its body lines: the `HDR`
/// record, the cover line, and the form-type facts derived from them.
///
/// This is what [`FilingReader::new`] parses eagerly. The field
/// documentation on [`Filing`] applies to each field here; the cover line
/// is always [`ParsedLine::line_no`] 2, as in the eager parser.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct Preamble {
    /// The parsed `HDR` record.
    pub header: Header,
    /// The filing's spec version (same as `header.version`), which selects
    /// the column layout every body line is parsed with.
    pub version: SpecVersion,
    /// The top-level form type as filed, upper-cased, e.g. `"F3XA"`.
    pub raw_form_type: String,
    /// `raw_form_type` without its amendment/new/termination designator,
    /// e.g. `"F3X"`.
    pub base_form_type: String,
    /// True if `raw_form_type` designates an amendment (ends in `A`).
    pub is_amendment: bool,
    /// The filing number this filing amends, from the header's
    /// `report_id`; `None` when absent or malformed.
    pub amends_filing: Option<u64>,
    /// The cover/summary line. For a Form 99 this includes any
    /// `[BEGINTEXT]` block that directly follows it, spliced into `text`.
    pub summary: ParsedLine,
}

impl Preamble {
    /// Parses the header and cover line from their already-split fields.
    ///
    /// Fails with [`FecError::UnknownElectronicHeaderVersion`] if the
    /// header's version is not an electronic 3.x-8.x version,
    /// [`FecError::MissingFormLine`] if the cover line has no form-type
    /// token, [`FecError::ParserMissing`] if that token names no known
    /// table, and [`FecError::NoMatchingVersionBucket`] if the table has no
    /// layout at this version.
    pub fn from_fields(header_fields: &[&str], summary_fields: &[&str]) -> Result<Self> {
        let header = Header::from_fields(header_fields)?;
        let version = header.version;

        let raw_form_type = normalize_form_type(summary_fields.first().copied().unwrap_or(""));
        if raw_form_type.is_empty() {
            return Err(FecError::MissingFormLine);
        }
        let base_form_type = strip_ant_suffix(&raw_form_type);
        let is_amendment = raw_form_type.ends_with('A');
        let amends_filing = header.original_filing_id();

        let table =
            form::table_for_form_type(&raw_form_type).ok_or_else(|| FecError::ParserMissing {
                form_type: raw_form_type.clone(),
                version,
                line_no: Some(2),
            })?;
        let summary = ParsedLine::from_cells(table, version, 2, summary_fields)?;

        Ok(Self {
            header,
            version,
            raw_form_type,
            base_form_type,
            is_amendment,
            amends_filing,
            summary,
        })
    }

    /// Whether this filing's base form type is one this crate knows how to
    /// interpret end-to-end (same as [`Filing::is_allowed`]).
    #[must_use]
    pub fn is_allowed(&self) -> bool {
        form::is_allowed_top_level_form(&self.base_form_type)
    }

    /// Assembles a [`Filing`] from this preamble and its body `lines`.
    #[must_use]
    pub fn into_filing(self, lines: Vec<ParsedLine>) -> Filing {
        Filing {
            header: self.header,
            version: self.version,
            raw_form_type: self.raw_form_type,
            base_form_type: self.base_form_type,
            is_amendment: self.is_amendment,
            amends_filing: self.amends_filing,
            summary: self.summary,
            lines,
        }
    }
}

// ---------------------------------------------------------------------------
// FilingReader
// ---------------------------------------------------------------------------

/// A streaming parser over one filing: header and cover line up front,
/// body lines one at a time as an [`Iterator`] of
/// `Result<`[`ParsedLine`]`>`.
///
/// Memory use is independent of the filing's size: one physical line's
/// bytes, one held-back record (see the module docs), and -- under lenient
/// options only -- the list of [`SkippedLine`]s, which grows with the
/// number of lines skipped.
///
/// Construct with [`FilingReader::new`] (strict) or
/// [`FilingReader::with_options`]. The iterator is fused: once it has
/// returned `None`, or an `Err`, it stays exhausted.
pub struct FilingReader<R: BufRead> {
    source: Source<R>,
    preamble: Preamble,
    body: Body,
}

impl<R: BufRead> FilingReader<R> {
    /// Opens a filing for streaming under [`ParseOptions::STRICT`].
    ///
    /// Reads the header and cover line (and, for a Form 99, any
    /// `[BEGINTEXT]` block that directly follows the cover), choosing the
    /// ASCII-28 or comma-delimited path from the first line as
    /// [`Filing::parse`] does.
    ///
    /// Fails with [`FecError::Io`] if the reader fails,
    /// [`FecError::DeprecatedHeaderFormat`] if the file starts with `/*`,
    /// [`FecError::MissingFormLine`] if it has fewer than two lines or a
    /// blank cover line, [`FecError::Csv`] on a malformed comma-delimited
    /// header, [`FecError::UnterminatedTextBlock`] if a text block after
    /// the cover never closes, and with whatever
    /// [`Preamble::from_fields`] fails with.
    pub fn new(reader: R) -> Result<Self> {
        Self::with_options(reader, ParseOptions::STRICT)
    }

    /// Like [`FilingReader::new`] with explicit [`ParseOptions`].
    ///
    /// Under lenient options, body lines with an unknown form-type token
    /// or no layout for this version are recorded in
    /// [`skipped`](Self::skipped) rather than ending the read.
    pub fn with_options(reader: R, options: ParseOptions) -> Result<Self> {
        let mut source = LineSource::new(reader);
        let Some((_, header_raw)) = source.next_line()? else {
            return Err(FecError::MissingFormLine);
        };
        if header_raw.starts_with("/*") {
            return Err(FecError::DeprecatedHeaderFormat);
        }
        let is_delimited = header_raw.contains(NEW_DELIMITER);
        let header_line = header_raw.into_owned();

        if is_delimited {
            let Some((_, summary_raw)) = source.next_line()? else {
                return Err(FecError::MissingFormLine);
            };
            let mut preamble = Preamble::from_fields(
                &split_new_delimited(&header_line),
                &split_new_delimited(&summary_raw),
            )?;
            let mut body = Body::new(preamble.version, options);
            if !source.hold_first_body_line(&mut preamble.summary)? {
                body.done = true;
            }
            Ok(Self {
                source: Source::Delimited(source),
                preamble,
                body,
            })
        } else {
            // Spec 3.x-5.x: comma-delimited with CSV quoting, so the header
            // must be re-read through the CSV parser (its fields may be
            // quoted). Chain the raw first line back in front of the rest.
            let LineSource { reader, buf, .. } = source;
            let mut csv = CsvSource::new(Cursor::new(buf).chain(reader));
            if csv.next_record()?.is_none() {
                return Err(FecError::MissingFormLine);
            }
            let header_fields: Vec<String> = csv.fields().into_iter().map(str::to_owned).collect();
            if csv.next_record()?.is_none() {
                return Err(FecError::MissingFormLine);
            }
            let summary_fields = csv.fields();
            let preamble = Preamble::from_fields(
                &header_fields.iter().map(String::as_str).collect::<Vec<_>>(),
                &summary_fields,
            )?;
            let body = Body::new(preamble.version, options);
            Ok(Self {
                source: Source::Csv(csv),
                preamble,
                body,
            })
        }
    }

    /// The header, cover line, and form-type facts, parsed up front.
    #[must_use]
    pub fn preamble(&self) -> &Preamble {
        &self.preamble
    }

    /// Restricts the iterator to lines of the given tables.
    ///
    /// Lines of other tables are dropped silently -- they are *not*
    /// recorded as skipped -- but every line is still dispatched, so an
    /// unknown form-type token is still an error (or a [`SkippedLine`])
    /// per the [`ParseOptions`]. A `[BEGINTEXT]` block following a dropped
    /// line is dropped with it. An empty set yields nothing. Replaces any
    /// earlier filter.
    #[must_use]
    pub fn filter_tables(mut self, tables: impl IntoIterator<Item = Table>) -> Self {
        let mut set: Vec<Table> = tables.into_iter().collect();
        set.sort_unstable();
        set.dedup();
        self.body.filter = Some(set);
        self
    }

    /// Body lines skipped so far under lenient [`ParseOptions`]; always
    /// empty under [`ParseOptions::STRICT`].
    #[must_use]
    pub fn skipped(&self) -> &[SkippedLine] {
        &self.body.skipped
    }

    /// Physical lines consumed from the underlying reader so far,
    /// including the header and cover line and any line read ahead.
    ///
    /// Exact for ASCII-28 filings. For comma-delimited (spec 3.x-5.x)
    /// filings, where a quoted field may span lines, it is the last line
    /// of the most recently read record.
    #[must_use]
    pub fn lines_read(&self) -> u64 {
        match &self.source {
            Source::Delimited(s) => s.line_no,
            Source::Csv(s) => s.lines_read,
        }
    }

    /// Drains the reader into an eager [`Filing`], wrapped in a
    /// [`Lenient`] carrying every line skipped under lenient options
    /// (none under [`ParseOptions::STRICT`]) -- the same type
    /// [`Filing::parse_bytes_with`] returns.
    ///
    /// Fails with the first `Err` the iterator yields. With a
    /// [`filter_tables`](Self::filter_tables) in place, [`Filing::lines`]
    /// holds only the selected tables.
    pub fn into_filing(mut self) -> Result<Lenient<Filing>> {
        let lines = self.by_ref().collect::<Result<Vec<_>>>()?;
        let Self { preamble, body, .. } = self;
        Ok(Lenient::from_parts(
            preamble.into_filing(lines),
            body.skipped,
            body.first_error,
        ))
    }
}

impl<R: BufRead> Iterator for FilingReader<R> {
    type Item = Result<ParsedLine>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(err) = self.body.deferred.take() {
            return Some(Err(err));
        }
        if self.body.done {
            return None;
        }
        match &mut self.source {
            Source::Delimited(source) => next_delimited(source, &mut self.preamble, &mut self.body),
            Source::Csv(source) => next_csv(source, &mut self.body),
        }
    }
}

impl<R: BufRead> FusedIterator for FilingReader<R> {}

/// One iteration of the ASCII-28 path: reads lines until one produces a
/// record to yield (or an error, or the end of input).
fn next_delimited<R: BufRead>(
    source: &mut LineSource<R>,
    preamble: &mut Preamble,
    body: &mut Body,
) -> Option<Result<ParsedLine>> {
    loop {
        let (line_no, raw) = match source.next_line() {
            Ok(Some(line)) => line,
            Ok(None) => return body.finish(),
            Err(e) => return body.fail(e),
        };
        match classify(&raw) {
            LineKind::Blank => {}
            LineKind::BeginText => match source.read_text_block(line_no) {
                Ok(text) => body.attach_text(&mut preamble.summary, &text),
                Err(e) => return body.fail(e),
            },
            LineKind::Record => {
                let fields = split_new_delimited(&raw);
                if fields.iter().all(|f| f.trim().is_empty()) {
                    continue;
                }
                if let Some(item) = body.accept(&fields, line_no) {
                    return Some(item);
                }
            }
        }
    }
}

/// One iteration of the comma-delimited path. Pre-6.0 filings predate the
/// `[BEGINTEXT]` convention, so there is no text-block handling here (as in
/// the eager parser).
fn next_csv<R: Read>(source: &mut CsvSource<R>, body: &mut Body) -> Option<Result<ParsedLine>> {
    loop {
        let line_no = match source.next_record() {
            Ok(Some(line_no)) => line_no,
            Ok(None) => return body.finish(),
            Err(e) => return body.fail(e),
        };
        let fields = source.fields();
        if fields.iter().all(|f| f.trim().is_empty()) {
            continue;
        }
        if let Some(item) = body.accept(&fields, line_no) {
            return Some(item);
        }
    }
}

// ---------------------------------------------------------------------------
// Body state: dispatch, options, and the one-record lookahead
// ---------------------------------------------------------------------------

/// Where a `[BEGINTEXT]` block encountered now would be spliced.
enum Lookahead {
    /// No body record has been parsed yet: a block belongs to the cover.
    Cover,
    /// The most recent record, held back until the next line shows it is
    /// not followed by a text block.
    Record(ParsedLine),
    /// The most recent record was dropped by the table filter, so a block
    /// following it has nowhere to go.
    Discarded,
}

struct Body {
    version: SpecVersion,
    options: ParseOptions,
    filter: Option<Vec<Table>>,
    skipped: Vec<SkippedLine>,
    /// The error behind the first skip, for `Lenient::into_strict`.
    first_error: Option<Box<FecError>>,
    lookahead: Lookahead,
    /// An error to yield on the next call, once the held-back record that
    /// preceded it has been released.
    deferred: Option<FecError>,
    done: bool,
}

impl Body {
    fn new(version: SpecVersion, options: ParseOptions) -> Self {
        Self {
            version,
            options,
            filter: None,
            skipped: Vec::new(),
            first_error: None,
            lookahead: Lookahead::Cover,
            deferred: None,
            done: false,
        }
    }

    /// Dispatches one non-blank body line. `Some` is an item to yield now
    /// (a released record, or an error); `None` means keep reading.
    fn accept(&mut self, fields: &[&str], line_no: u64) -> Option<Result<ParsedLine>> {
        let form_type = normalize_form_type(fields.first().copied().unwrap_or(""));
        if form_type.is_empty() {
            // A line whose first column is blank carries no record type;
            // treat it like a blank line (as the eager parser does).
            return None;
        }

        let Some(table) = form::table_for_form_type(&form_type) else {
            let err = FecError::ParserMissing {
                form_type: form_type.clone(),
                version: self.version,
                line_no: Some(line_no),
            };
            return self.skip_or_fail(
                self.options.on_unknown_line,
                err,
                line_no,
                form_type,
                SkipReason::UnknownFormType,
            );
        };

        match ParsedLine::from_cells(table, self.version, line_no, fields) {
            Ok(line) => self.advance(line),
            Err(e @ FecError::NoMatchingVersionBucket { .. }) => self.skip_or_fail(
                self.options.on_missing_version,
                e,
                line_no,
                form_type,
                SkipReason::NoLayoutForVersion,
            ),
            Err(e) => self.fail(e),
        }
    }

    /// Slides the lookahead window: `line` becomes the held-back record
    /// (or is discarded by the filter) and whatever was held is released.
    fn advance(&mut self, line: ParsedLine) -> Option<Result<ParsedLine>> {
        let wanted = self
            .filter
            .as_ref()
            .is_none_or(|tables| tables.contains(&line.table()));
        let next = if wanted {
            Lookahead::Record(line)
        } else {
            Lookahead::Discarded
        };
        match std::mem::replace(&mut self.lookahead, next) {
            Lookahead::Record(previous) => Some(Ok(previous)),
            Lookahead::Cover | Lookahead::Discarded => None,
        }
    }

    fn skip_or_fail(
        &mut self,
        policy: OnUnparseableLine,
        err: FecError,
        line_no: u64,
        raw_form_type: String,
        reason: SkipReason,
    ) -> Option<Result<ParsedLine>> {
        match policy {
            OnUnparseableLine::Fail => self.fail(err),
            OnUnparseableLine::Skip => {
                if self.first_error.is_none() {
                    self.first_error = Some(Box::new(err));
                }
                self.skipped.push(SkippedLine {
                    line_no,
                    raw_form_type,
                    reason,
                });
                None
            }
        }
    }

    /// Ends the read with `err`. A record still held back is released
    /// first; the error follows on the next call.
    fn fail(&mut self, err: FecError) -> Option<Result<ParsedLine>> {
        self.done = true;
        match std::mem::replace(&mut self.lookahead, Lookahead::Discarded) {
            Lookahead::Record(previous) => {
                self.deferred = Some(err);
                Some(Ok(previous))
            }
            Lookahead::Cover | Lookahead::Discarded => Some(Err(err)),
        }
    }

    /// End of input: releases the held-back record, if any.
    fn finish(&mut self) -> Option<Result<ParsedLine>> {
        self.done = true;
        match std::mem::replace(&mut self.lookahead, Lookahead::Discarded) {
            Lookahead::Record(previous) => Some(Ok(previous)),
            Lookahead::Cover | Lookahead::Discarded => None,
        }
    }

    /// Splices a `[BEGINTEXT]` block into the record it follows. A record
    /// whose layout has no `text` field cannot carry it and the block is
    /// dropped, as in the eager parser (`hardmoney validate` reports it).
    fn attach_text(&mut self, summary: &mut ParsedLine, text: &str) {
        match &mut self.lookahead {
            Lookahead::Cover => {
                let _ = summary.set("text", text);
            }
            Lookahead::Record(line) => {
                let _ = line.set("text", text);
            }
            Lookahead::Discarded => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Sources
// ---------------------------------------------------------------------------

enum Source<R: BufRead> {
    /// Spec 6.0+: one record per physical line, ASCII-28 delimited.
    Delimited(LineSource<R>),
    /// Spec 3.x-5.x: comma-delimited with CSV quoting. The first line was
    /// consumed to detect the delimiter and is chained back in front.
    Csv(CsvSource<io::Chain<Cursor<Vec<u8>>, R>>),
}

enum LineKind {
    Blank,
    BeginText,
    Record,
}

fn classify(raw: &str) -> LineKind {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        LineKind::Blank
    } else if trimmed.eq_ignore_ascii_case("[BEGINTEXT]") {
        LineKind::BeginText
    } else {
        LineKind::Record
    }
}

/// Physical lines from a [`BufRead`], numbered from 1, with one reusable
/// byte buffer.
struct LineSource<R> {
    reader: R,
    /// The most recent line, terminator included (the comma-delimited
    /// path needs the raw bytes back).
    buf: Vec<u8>,
    line_no: u64,
    /// The first body line, read by the constructor while consuming text
    /// blocks that belong to the cover, and handed out by the next
    /// [`next_line`](Self::next_line) call.
    held: Option<(u64, String)>,
}

impl<R: BufRead> LineSource<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            buf: Vec::new(),
            line_no: 0,
            held: None,
        }
    }

    /// The next physical line and its 1-based number, without its `\n` or
    /// `\r\n` terminator (the same splitting as [`str::lines`]), decoded
    /// per [`decode_line`]. `Ok(None)` at end of input.
    fn next_line(&mut self) -> Result<Option<(u64, Cow<'_, str>)>> {
        if let Some((line_no, held)) = self.held.take() {
            return Ok(Some((line_no, Cow::Owned(held))));
        }
        self.buf.clear();
        if self.reader.read_until(b'\n', &mut self.buf)? == 0 {
            return Ok(None);
        }
        self.line_no = self.line_no.saturating_add(1);
        let line = strip_terminator(&self.buf);
        Ok(Some((self.line_no, decode_line(line))))
    }

    /// Having just read a `[BEGINTEXT]` line at `start`, collects every
    /// line up to the matching `[ENDTEXT]`, joined with `\n`. Fails with
    /// [`FecError::UnterminatedTextBlock`] if the input ends first.
    fn read_text_block(&mut self, start: u64) -> Result<String> {
        let mut text = String::new();
        let mut first = true;
        loop {
            let Some((_, raw)) = self.next_line()? else {
                return Err(FecError::UnterminatedTextBlock { line_no: start });
            };
            if raw.trim().eq_ignore_ascii_case("[ENDTEXT]") {
                return Ok(text);
            }
            if !first {
                text.push('\n');
            }
            first = false;
            text.push_str(raw.trim_end_matches('\r'));
        }
    }

    /// Consumes blank lines and any `[BEGINTEXT]` blocks that directly
    /// follow the cover line (a Form 99's own free text), splicing them
    /// into `summary`, then holds back the first body line for
    /// [`next_line`](Self::next_line). Returns `false` if the input ended
    /// without one.
    fn hold_first_body_line(&mut self, summary: &mut ParsedLine) -> Result<bool> {
        loop {
            let Some((line_no, raw)) = self.next_line()? else {
                return Ok(false);
            };
            match classify(&raw) {
                LineKind::Blank => {}
                LineKind::BeginText => {
                    let text = self.read_text_block(line_no)?;
                    let _ = summary.set("text", &text);
                }
                LineKind::Record => {
                    let owned = raw.into_owned();
                    self.held = Some((line_no, owned));
                    return Ok(true);
                }
            }
        }
    }
}

/// Drops a trailing `\n` or `\r\n`. A `\r` not followed by `\n` is kept,
/// as [`str::lines`] keeps it (the field splitter trims it anyway).
fn strip_terminator(line: &[u8]) -> &[u8] {
    let line = line.strip_suffix(b"\n").unwrap_or(line);
    line.strip_suffix(b"\r").unwrap_or(line)
}

/// Decodes one physical line: UTF-8 if valid, else Windows-1252 (the
/// per-line counterpart of [`crate::parser::filing::decode`]).
fn decode_line(bytes: &[u8]) -> Cow<'_, str> {
    match std::str::from_utf8(bytes) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => {
            encoding_rs::WINDOWS_1252
                .decode_without_bom_handling(bytes)
                .0
        }
    }
}

/// Records from the `csv` crate over the comma-delimited path, with one
/// reusable record buffer.
struct CsvSource<R> {
    reader: csv::Reader<R>,
    record: CsvRecord,
    /// The current record's fields re-decoded as Windows-1252; only filled
    /// when the record is not valid UTF-8.
    decoded: Vec<String>,
    lines_read: u64,
}

/// The one record buffer, in whichever form the last read left it. The
/// conversions between the two are moves of the same allocation, so the
/// buffer is reused across records either way.
enum CsvRecord {
    /// Validated UTF-8 -- the normal case. `csv` checks the whole record
    /// in one pass (ASCII fast path) and then slices fields without
    /// re-checking, which is what the eager parser's `StringRecord`s do.
    Text(csv::StringRecord),
    /// Not valid UTF-8; `CsvSource::decoded` holds the Windows-1252
    /// reading of every field.
    Bytes(csv::ByteRecord),
}

impl Default for CsvRecord {
    fn default() -> Self {
        Self::Bytes(csv::ByteRecord::new())
    }
}

impl CsvRecord {
    fn into_bytes(self) -> csv::ByteRecord {
        match self {
            Self::Text(text) => text.into_byte_record(),
            Self::Bytes(bytes) => bytes,
        }
    }
}

impl<R: Read> CsvSource<R> {
    fn new(reader: R) -> Self {
        Self {
            reader: csv::ReaderBuilder::new()
                .has_headers(false)
                .flexible(true)
                .from_reader(reader),
            record: CsvRecord::default(),
            decoded: Vec::new(),
            lines_read: 0,
        }
    }

    /// Reads the next record into the buffer and returns the 1-based line
    /// it starts on (what the eager parser records as `line_no`).
    /// `Ok(None)` at end of input.
    fn next_record(&mut self) -> Result<Option<u64>> {
        let mut bytes = std::mem::take(&mut self.record).into_bytes();
        let more = self.reader.read_byte_record(&mut bytes)?;
        // csv-core's line counter is 1 + newlines consumed, so after a
        // newline-terminated record `line() - 1` is that record's last
        // line. A final record with no terminator is covered by its own
        // start line instead.
        let consumed = self.reader.position().line().saturating_sub(1);
        self.lines_read = self.lines_read.max(consumed);
        let line_no = bytes.position().map(|p| p.line()).unwrap_or(0);

        self.record = match csv::StringRecord::from_byte_record(bytes) {
            Ok(text) => CsvRecord::Text(text),
            Err(not_utf8) => {
                let bytes = not_utf8.into_byte_record();
                self.decoded.clear();
                self.decoded.extend(bytes.iter().map(|f| {
                    encoding_rs::WINDOWS_1252
                        .decode_without_bom_handling(f)
                        .0
                        .into_owned()
                }));
                CsvRecord::Bytes(bytes)
            }
        };

        if !more {
            return Ok(None);
        }
        self.lines_read = self.lines_read.max(line_no);
        Ok(Some(line_no))
    }

    /// The current record's fields, decoded as a unit: all UTF-8 if every
    /// field is valid UTF-8, otherwise all Windows-1252.
    fn fields(&self) -> Vec<&str> {
        match &self.record {
            CsvRecord::Text(text) => text.iter().collect(),
            CsvRecord::Bytes(_) => self.decoded.iter().map(String::as_str).collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Filing::open
// ---------------------------------------------------------------------------

impl Filing {
    /// Parses a filing from a file on disk, strictly, by streaming it
    /// through a [`FilingReader`] rather than reading it into memory first.
    ///
    /// The result is field-for-field equal to
    /// `Filing::parse_bytes(&std::fs::read(path)?)` (see the module docs
    /// for the one encoding corner case), but peak memory is the parsed
    /// lines alone rather than the parsed lines plus the file's bytes plus
    /// its decoded text.
    ///
    /// Fails with [`FecError::Io`] (naming the path) if the file cannot be
    /// opened or read, and otherwise with whatever [`FilingReader::new`] or
    /// its iterator fails with.
    pub fn open(path: impl AsRef<Path>) -> Result<Filing> {
        Self::open_with(path, ParseOptions::STRICT)?.into_strict()
    }

    /// Like [`Filing::open`] with explicit [`ParseOptions`]: the streaming
    /// counterpart of [`Filing::parse_bytes_with`], returning the same
    /// [`Lenient<Filing>`] so callers can switch between the two freely.
    pub fn open_with(path: impl AsRef<Path>, options: ParseOptions) -> Result<Lenient<Filing>> {
        let path = path.as_ref();
        let file = File::open(path)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        let reader = BufReader::with_capacity(OPEN_BUFFER_BYTES, file);
        FilingReader::with_options(reader, options)?.into_filing()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HDR: &str = "HDR\u{1c}FEC\u{1c}8.5\u{1c}FECfile\u{1c}8.5.1.0(f34)\u{1c}\u{1c}";

    fn reader(content: &str) -> FilingReader<&[u8]> {
        FilingReader::new(content.as_bytes()).unwrap_or_else(|e| panic!("{e}"))
    }

    fn lenient(content: &str) -> FilingReader<&[u8]> {
        FilingReader::with_options(content.as_bytes(), ParseOptions::LENIENT)
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn collect(r: FilingReader<&[u8]>) -> Vec<ParsedLine> {
        r.collect::<Result<Vec<_>>>()
            .unwrap_or_else(|e| panic!("{e}"))
    }

    fn assert_same_as_eager(content: &str) {
        let eager = Filing::parse(content).unwrap_or_else(|e| panic!("{e}"));
        let (streamed, skipped) = reader(content).into_filing().unwrap().into_parts();
        assert!(skipped.is_empty());
        assert_eq!(streamed.header, eager.header);
        assert_eq!(streamed.version, eager.version);
        assert_eq!(streamed.raw_form_type, eager.raw_form_type);
        assert_eq!(streamed.base_form_type, eager.base_form_type);
        assert_eq!(streamed.is_amendment, eager.is_amendment);
        assert_eq!(streamed.amends_filing, eager.amends_filing);
        assert_eq!(streamed.summary, eager.summary);
        assert_eq!(streamed.lines, eager.lines);
    }

    #[test]
    fn preamble_matches_eager_parser() {
        let content = format!("{HDR}\nF3XN\u{1c}C00123456\u{1c}COMMITTEE NAME\n");
        let r = reader(&content);
        let p = r.preamble();
        assert_eq!(p.version, SpecVersion::electronic(8, 5));
        assert_eq!(p.header.soft_name, "FECfile");
        assert_eq!(p.raw_form_type, "F3XN");
        assert_eq!(p.base_form_type, "F3X");
        assert_eq!(p.summary.table(), Table::F3X);
        assert_eq!(p.summary.line_no, 2);
        assert!(!p.is_amendment);
        assert!(p.amends_filing.is_none());
        assert!(p.is_allowed());
        assert_eq!(r.lines_read(), 2);
        assert_same_as_eager(&content);
    }

    #[test]
    fn yields_body_lines_in_order_with_line_numbers() {
        let content = format!(
            "{HDR}\nF3XN\u{1c}C00123456\nSA11AI\u{1c}C00123456\u{1c}IND\u{1c}SMITH, JANE\n\nSB21B\u{1c}C00123456\u{1c}ORG\u{1c}VENDOR"
        );
        let lines = collect(reader(&content));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].table(), Table::SchA);
        assert_eq!(lines[0].line_no, 3);
        assert_eq!(lines[1].table(), Table::SchB);
        assert_eq!(lines[1].line_no, 5);
        assert_same_as_eager(&content);
    }

    #[test]
    fn crlf_line_endings_match_eager_parser() {
        let content = format!(
            "{HDR}\r\nF3XN\u{1c}C00123456\r\nSA11AI\u{1c}C00123456\u{1c}IND\u{1c}SMITH, JANE\r\nSB21B\u{1c}C00123456\u{1c}ORG\u{1c}VENDOR\r\n"
        );
        let lines = collect(reader(&content));
        assert_eq!(lines.len(), 2);
        assert_eq!(
            lines[1].get("back_reference_tran_id_number"),
            Some("VENDOR")
        );
        assert_same_as_eager(&content);
    }

    #[test]
    fn text_block_after_cover_is_in_preamble_before_iteration() {
        let content = format!(
            "{HDR}\nF99\u{1c}C00944124\u{1c}REVIVE OREGON\n[BEGINTEXT]\nThis is the free text explanation.\nIt spans two lines.\n[ENDTEXT]\n"
        );
        let r = reader(&content);
        assert_eq!(
            r.preamble().summary.get("text"),
            Some("This is the free text explanation.\nIt spans two lines.")
        );
        assert!(collect(r).is_empty());
        assert_same_as_eager(&content);
    }

    #[test]
    fn text_block_attaches_to_preceding_body_line_before_it_is_yielded() {
        let content = format!(
            "{HDR}\nF99\u{1c}C00944124\u{1c}REVIVE OREGON\nTEXT\u{1c}C00944124\u{1c}T1\u{1c}\u{1c}\u{1c}\n[BEGINTEXT]\nbody text\n[ENDTEXT]\nTEXT\u{1c}C00944124\u{1c}T2\n"
        );
        let lines = collect(reader(&content));
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].get("text"), Some("body text"));
        assert_eq!(lines[1].get("text"), Some(""));
        assert_same_as_eager(&content);
    }

    #[test]
    fn text_block_after_skipped_line_goes_to_last_parsed_record() {
        // Skipped lines are not records, so the block belongs to the cover
        // -- exactly what the eager parser does.
        let content = format!(
            "{HDR}\nF99\u{1c}C00944124\u{1c}REVIVE OREGON\nZZZ\u{1c}junk\n[BEGINTEXT]\nlate\n[ENDTEXT]\n"
        );
        let mut r = lenient(&content);
        assert_eq!(r.preamble().summary.get("text"), Some(""));
        assert!(r.next().is_none());
        assert_eq!(r.preamble().summary.get("text"), Some("late"));
        assert_eq!(r.skipped().len(), 1);

        let (eager, _) = Filing::parse_with(&content, &ParseOptions::LENIENT)
            .unwrap()
            .into_parts();
        assert_eq!(eager.summary.get("text"), Some("late"));
    }

    #[test]
    fn unterminated_text_block_after_cover_fails_construction() {
        let content =
            format!("{HDR}\nF99\u{1c}C00944124\u{1c}REVIVE OREGON\n[BEGINTEXT]\nnever closed\n");
        assert!(matches!(
            FilingReader::new(content.as_bytes()),
            Err(FecError::UnterminatedTextBlock { line_no: 3 })
        ));
    }

    #[test]
    fn unterminated_text_block_in_body_releases_record_then_errors() {
        let content = format!(
            "{HDR}\nF99\u{1c}C00944124\u{1c}REVIVE OREGON\nTEXT\u{1c}C00944124\u{1c}T1\n[BEGINTEXT]\nnever closed\n"
        );
        let mut r = reader(&content);
        assert!(matches!(r.next(), Some(Ok(line)) if line.table() == Table::Text));
        assert!(matches!(
            r.next(),
            Some(Err(FecError::UnterminatedTextBlock { line_no: 4 }))
        ));
        assert!(r.next().is_none());
        assert!(r.next().is_none());
    }

    #[test]
    fn strict_unknown_line_ends_the_read_with_its_line_number() {
        let content = format!(
            "{HDR}\nF3XN\u{1c}C00123456\nSA11AI\u{1c}C00123456\nZZZ\u{1c}junk\nSB21B\u{1c}C00123456\n"
        );
        let mut r = reader(&content);
        assert!(matches!(r.next(), Some(Ok(line)) if line.table() == Table::SchA));
        match r.next() {
            Some(Err(FecError::ParserMissing {
                form_type, line_no, ..
            })) => {
                assert_eq!(form_type, "ZZZ");
                assert_eq!(line_no, Some(4));
            }
            other => panic!("expected ParserMissing, got {other:?}"),
        }
        assert!(r.next().is_none());
    }

    #[test]
    fn lenient_records_skips_and_keeps_going() {
        let content = format!(
            "{HDR}\nF3XN\u{1c}C00123456\nSA11AI\u{1c}C00123456\nZZZ\u{1c}junk\nSI\u{1c}C00123456\nSB21B\u{1c}C00123456\n"
        );
        let mut r = lenient(&content);
        let lines = r.by_ref().collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[1].table(), Table::SchB);
        let skipped = r.skipped();
        assert_eq!(skipped.len(), 2);
        assert_eq!(skipped[0].line_no, 4);
        assert_eq!(skipped[0].reason, SkipReason::UnknownFormType);
        assert_eq!(skipped[1].line_no, 5);
        assert_eq!(skipped[1].reason, SkipReason::NoLayoutForVersion);
        assert_eq!(r.lines_read(), 6);

        let (eager, eager_skipped) = Filing::parse_with(&content, &ParseOptions::LENIENT)
            .unwrap()
            .into_parts();
        assert_eq!(eager.lines, lines);
        assert_eq!(eager_skipped, skipped);
    }

    #[test]
    fn filter_tables_drops_other_tables_silently_but_still_dispatches() {
        let content = format!(
            "{HDR}\nF3XN\u{1c}C00123456\nSA11AI\u{1c}C00123456\nSB21B\u{1c}C00123456\nSA11AI\u{1c}C00123456\nZZZ\u{1c}junk\n"
        );
        let lines = collect(lenient(&content).filter_tables([Table::SchB, Table::SchB]));
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].line_no, 4);

        let mut r = lenient(&content).filter_tables([Table::SchA]);
        let lines = r.by_ref().collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(lines.iter().map(|l| l.line_no).collect::<Vec<_>>(), [3, 5]);
        assert_eq!(r.skipped().len(), 1, "unknown token still reported");

        assert!(collect(lenient(&content).filter_tables([])).is_empty());
        // Strict: nothing is held back (every record was dropped), so the
        // unknown token is the very first item.
        assert!(matches!(
            reader(&content).filter_tables([Table::SchE]).next(),
            Some(Err(FecError::ParserMissing { .. }))
        ));
    }

    #[test]
    fn text_block_after_filtered_out_line_is_dropped() {
        let content = format!(
            "{HDR}\nF99\u{1c}C00944124\nTEXT\u{1c}C00944124\u{1c}T1\nSA11AI\u{1c}C00944124\n[BEGINTEXT]\nfor the SA line\n[ENDTEXT]\n"
        );
        let r = reader(&content).filter_tables([Table::Text]);
        let lines = collect(r);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].get("text"), Some(""));
    }

    #[test]
    fn old_comma_delimited_filing_streams_with_csv_quoting() {
        let content = "HDR,FEC,5.3,SoftCo,1.0\nF3XN,C00123456,\"COMMITTEE, INC.\"\nSA11AI,C00123456,IND,\"SMITH, JANE\"\n\nSB21B,C00123456,ORG,VENDOR\n";
        let mut r = reader(content);
        assert_eq!(r.preamble().version, SpecVersion::electronic(5, 3));
        assert_eq!(
            r.preamble().summary.get("committee_name"),
            Some("COMMITTEE, INC.")
        );
        let lines = r.by_ref().collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].line_no, 3);
        assert_eq!(lines[0].get("contributor_name"), Some("SMITH, JANE"));
        // The csv crate stamps a record with the position it started
        // scanning from, which is the blank line 4 rather than the record's
        // own line 5. The eager parser reports the same number (checked
        // below), and equivalence is the contract here.
        assert_eq!(lines[1].line_no, 4);
        assert_eq!(r.lines_read(), 5);
        assert_same_as_eager(content);
    }

    #[test]
    fn quoted_csv_header_round_trips_through_the_chained_first_line() {
        let content = "\"HDR\",\"FEC\",\"3.00\",\"KNOWLEDGE\",\"XP.1108\",\"\",\"FEC-24088\",\"1\",\"\"\n\"F3XA\",\"C00123456\",\"X\"\n";
        let r = reader(content);
        assert_eq!(r.preamble().header.soft_name, "KNOWLEDGE");
        assert_eq!(r.preamble().amends_filing, Some(24088));
        assert_same_as_eager(content);
    }

    #[test]
    fn windows_1252_bytes_decode_per_line() {
        let mut bytes = format!("{HDR}\nF3XN\u{1c}C00123456\u{1c}").into_bytes();
        bytes.extend_from_slice(b"CAF\xC9 PAC\nSA11AI\x1cC00123456\x1cIND\x1c\xC9MILE\n");
        let mut r = FilingReader::new(bytes.as_slice()).unwrap();
        assert_eq!(r.preamble().summary.get("committee_name"), Some("CAFÉ PAC"));
        let line = r.next().unwrap().unwrap();
        assert_eq!(line.get("back_reference_tran_id_number"), Some("ÉMILE"));

        let eager = Filing::parse_bytes(&bytes).unwrap();
        assert_eq!(eager.summary, r.preamble().summary);
        assert_eq!(eager.lines, [line]);
    }

    #[test]
    fn windows_1252_bytes_decode_per_record_on_the_comma_path() {
        let mut bytes = b"HDR,FEC,5.3,SoftCo,1.0\nF3XN,C00123456,\"CAF\xC9, INC.\"\n".to_vec();
        bytes.extend_from_slice(b"SA11AI,C00123456,IND,\xC9MILE\nSA11AI,C00123456,IND,PLAIN\n");
        let mut r = FilingReader::new(bytes.as_slice()).unwrap();
        assert_eq!(
            r.preamble().summary.get("committee_name"),
            Some("CAFÉ, INC.")
        );
        let lines = r.by_ref().collect::<Result<Vec<_>>>().unwrap();
        assert_eq!(lines[0].get("contributor_name"), Some("ÉMILE"));
        assert_eq!(lines[1].get("contributor_name"), Some("PLAIN"));

        let eager = Filing::parse_bytes(&bytes).unwrap();
        assert_eq!(eager.summary, r.preamble().summary);
        assert_eq!(eager.lines, lines);
    }

    #[test]
    fn construction_errors_name_the_problem() {
        assert!(matches!(
            FilingReader::new("/* old format\n".as_bytes()),
            Err(FecError::DeprecatedHeaderFormat)
        ));
        for input in [
            "",
            "HDR\u{1c}FEC\u{1c}8.5",
            "HDR,FEC,5.3\n",
            "\u{1c}\u{1c}\u{1c}\n\n",
            "HDR\u{1c}FEC\u{1c}8.5\n\u{1c}\u{1c}",
        ] {
            assert!(
                FilingReader::new(input.as_bytes()).is_err(),
                "{input:?} should not construct"
            );
        }
        assert!(matches!(
            FilingReader::new("HDR\u{1c}FEC\u{1c}9.9\u{1c}X\nF3XN\u{1c}C1\n".as_bytes()),
            Err(FecError::UnknownElectronicHeaderVersion(_))
        ));
        assert!(matches!(
            FilingReader::new(format!("{HDR}\nZZ9\u{1c}C1\n").as_bytes()),
            Err(FecError::ParserMissing {
                line_no: Some(2),
                ..
            })
        ));
    }

    #[test]
    fn io_errors_surface_as_fec_io() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("disk on fire"))
            }
        }
        impl BufRead for Broken {
            fn fill_buf(&mut self) -> io::Result<&[u8]> {
                Err(io::Error::other("disk on fire"))
            }
            fn consume(&mut self, _: usize) {}
        }
        assert!(matches!(FilingReader::new(Broken), Err(FecError::Io(_))));

        let good = format!("{HDR}\nF3XN\u{1c}C00123456\nSA11AI\u{1c}C00123456\n");
        let mut r = FilingReader::new(good.as_bytes().chain(Broken)).unwrap();
        assert!(
            matches!(r.next(), Some(Ok(_))),
            "held record is released first"
        );
        assert!(matches!(r.next(), Some(Err(FecError::Io(_)))));
        assert!(r.next().is_none());
    }

    #[test]
    fn open_reads_a_fixture_identically_to_parse_bytes() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/F99_2011828.fec");
        let opened = Filing::open(&path).unwrap();
        let eager = Filing::parse_bytes(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(opened.summary, eager.summary);
        assert_eq!(opened.lines, eager.lines);
        assert!(opened.summary.get("text").is_some_and(|t| !t.is_empty()));
    }

    #[test]
    fn open_names_the_path_on_io_error() {
        let err = Filing::open("/definitely/not/here.fec").unwrap_err();
        assert!(matches!(err, FecError::Io(_)));
        assert!(
            err.to_string().contains("/definitely/not/here.fec"),
            "{err}"
        );
    }
}
