//! Filing tools over HTTP: parse, validate, reconcile, and write a `.fec`
//! filing without touching the database. They back the browser UI
//! (`hardmoney serve --ui`) and are usable from scripts.
//!
//! | Route | Body | Response |
//! |---|---|---|
//! | `POST /tools/parse` | raw `.fec` bytes | [`ParsedDocument`] |
//! | `GET /tools/fetch/{filing_id}` | none | [`ParsedDocument`] for the filing downloaded from the FEC's document store, if it fits the body cap (413 otherwise, and the download stops there; needs the `fetch` feature, 501 otherwise) |
//! | `POST /tools/validate` | a [`Document`] | [`Validation`] |
//! | `POST /tools/reconcile` | a [`Document`] | [`Reconciliation`], or 400 for a form without rules |
//! | `POST /tools/write` | a [`Document`] | the `.fec` bytes, `Content-Disposition: attachment`, `X-Hardmoney-Validation-Errors: N` |
//! | `GET /tools/spec/{table}?version=8.5` | none | [`TableSpec`]: the layout at that version with the FEC's field specs |
//!
//! Errors are `{"error": "..."}` like the rest of the API: 400 for a
//! malformed body or an unparseable filing (the message names the line),
//! 404 for an unknown table, 413 when the body exceeds the server's cap
//! (or when a [`Document`] would *write* to more than the cap: a JSON
//! record is a few bytes but builds a full-width line in memory, so the
//! sum of the records' layout widths is held to the same limit),
//! 415 for a JSON route called without `Content-Type: application/json`,
//! 501 when `fetch` is not compiled in, 502 when the FEC download fails.
//!
//! The router is state-free ([`router`] returns a `Router<S>` for any `S`)
//! so it can be mounted without a database, which is how `tests/ui_routes.rs`
//! exercises it. Mounted by [`crate::api::router`], `fetch` downloads from
//! the [`Endpoints`](crate::fec::Endpoints) in
//! [`ApiConfig::endpoints`](crate::api::ApiConfig::endpoints), which that
//! router supplies as a request extension; mounted on its own it falls
//! back to [`Endpoints::from_env`](crate::fec::Endpoints::from_env), which
//! reads `HARDMONEY_DOCQUERY_BASE`. At most [`MAX_CONCURRENT_FETCHES`]
//! downloads run at once; further ones wait (and time out with the
//! request) rather than each holding a filing-sized buffer and a thread.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::{BytesRejection, JsonRejection};
use axum::extract::{DefaultBodyLimit, Extension, Path, Query};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tower_http::limit::RequestBodyLimitLayer;

use crate::parser::form::table_for_form_type;
use crate::parser::{
    BUNDLED_SPEC_VERSION, FieldSpec, Filing, Header, ParseOptions, ParsedLine, Reconciliation,
    SkippedLine, SpecVersion, Table, Validation,
};

/// Server limits the tools routes enforce, shared with the rest of the API
/// through [`ApiConfig`](crate::api::ApiConfig).
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    /// Largest request body (and largest fetched filing) in bytes.
    pub max_body_bytes: usize,
}

/// How many `GET /tools/fetch/{id}` downloads one server runs at a time.
/// Each holds up to [`Limits::max_body_bytes`] and a blocking-pool thread
/// for the length of the download; a burst beyond this waits its turn.
pub const MAX_CONCURRENT_FETCHES: usize = 4;

/// Builds the `/tools/*` routes. State-free: mount it into any router.
/// The body cap in `limits` is enforced here as well as by the API's outer
/// layer, so a standalone mount is bounded too. Axum's own 2 MiB
/// extractor default is raised to the same cap; without that a 3 MB
/// document would be refused regardless of the configured limit.
pub fn router<S: Clone + Send + Sync + 'static>(limits: Limits) -> Router<S> {
    Router::new()
        .route("/tools/parse", post(parse))
        .route("/tools/fetch/{filing_id}", get(fetch))
        .route("/tools/validate", post(validate))
        .route("/tools/reconcile", post(reconcile))
        .route("/tools/write", post(write))
        .route("/tools/spec/{table}", get(spec))
        .layer(DefaultBodyLimit::max(limits.max_body_bytes))
        .layer(RequestBodyLimitLayer::new(limits.max_body_bytes))
        .layer(Extension(limits))
        .layer(Extension(FetchSlots(Arc::new(Semaphore::new(
            MAX_CONCURRENT_FETCHES,
        )))))
}

/// The semaphore behind [`MAX_CONCURRENT_FETCHES`], installed by
/// [`router`] as a request extension so that the router stays state-free.
/// Opaque; nothing outside this module constructs one.
#[derive(Clone, Debug)]
pub struct FetchSlots(Arc<Semaphore>);

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors from the tools routes, rendered as `{"error": "..."}`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ToolsError {
    /// Malformed JSON, an unknown table or field, an inconsistent document,
    /// or bytes that do not parse as a filing.
    #[error("{0}")]
    BadRequest(String),
    #[error("{0}")]
    NotFound(String),
    /// The body (or the fetched filing) exceeds [`Limits::max_body_bytes`].
    #[error("{0}")]
    PayloadTooLarge(String),
    /// A JSON route was called without `Content-Type: application/json`.
    #[error("{0}")]
    UnsupportedMediaType(String),
    /// The build lacks the feature the route needs.
    #[error("{0}")]
    NotImplemented(String),
    /// The FEC's document store could not be reached or refused the id.
    #[error("{0}")]
    UpstreamFailed(String),
    /// The server's own worker failed (a background task panicked or was
    /// cancelled); nothing about the request caused it.
    #[error("{0}")]
    Internal(String),
}

impl IntoResponse for ToolsError {
    fn into_response(self) -> Response {
        let status = match &self {
            ToolsError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ToolsError::NotFound(_) => StatusCode::NOT_FOUND,
            ToolsError::PayloadTooLarge(_) => StatusCode::PAYLOAD_TOO_LARGE,
            ToolsError::UnsupportedMediaType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ToolsError::NotImplemented(_) => StatusCode::NOT_IMPLEMENTED,
            ToolsError::UpstreamFailed(_) => StatusCode::BAD_GATEWAY,
            ToolsError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        };
        (status, Json(json!({ "error": self.to_string() }))).into_response()
    }
}

/// Runs `f` on tokio's blocking pool. Parsing, validating, reconciling, or
/// writing a filing of up to [`Limits::max_body_bytes`] takes real CPU
/// time; on a runtime worker it would stall every other request on that
/// worker (and the database pool's housekeeping) for the duration.
async fn offload<T, F>(f: F) -> Result<T, ToolsError>
where
    F: FnOnce() -> Result<T, ToolsError> + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| ToolsError::Internal(format!("worker task failed: {e}")))?
}

fn too_large(limits: Limits) -> ToolsError {
    ToolsError::PayloadTooLarge(format!(
        "the request body is larger than this server's {} byte limit; use the `hardmoney` CLI for large filings",
        limits.max_body_bytes
    ))
}

fn bytes_rejection(rej: BytesRejection, limits: Limits) -> ToolsError {
    if rej.status() == StatusCode::PAYLOAD_TOO_LARGE {
        too_large(limits)
    } else {
        ToolsError::BadRequest(rej.body_text())
    }
}

fn json_rejection(rej: JsonRejection, limits: Limits) -> ToolsError {
    match rej.status() {
        StatusCode::PAYLOAD_TOO_LARGE => too_large(limits),
        StatusCode::UNSUPPORTED_MEDIA_TYPE => ToolsError::UnsupportedMediaType(format!(
            "{}; send the document with Content-Type: application/json",
            rej.body_text()
        )),
        _ => ToolsError::BadRequest(rej.body_text()),
    }
}

// ---------------------------------------------------------------------------
// Parse
// ---------------------------------------------------------------------------

/// A parsed filing with everything the workbench shows about it.
///
/// `summary` and each entry of `lines` serialise as
/// `{raw_form_type, table, line_no, fields: {name: value}}` with every
/// field of the layout present (blank as `""`), in layout order. Field
/// values are always strings. `reconciliation` is `null` for forms without
/// reconciliation rules, with the reason in `reconcile_error`.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct ParsedDocument {
    pub header: Header,
    pub version: SpecVersion,
    /// The cover line's form-type token as filed, upper-cased (`F3XA`).
    pub form_type: String,
    pub base_form_type: String,
    pub is_amendment: bool,
    pub amends_filing: Option<u64>,
    pub summary: ParsedLine,
    pub lines: Vec<ParsedLine>,
    /// Body lines the lenient parse could not interpret; also reported as
    /// `unrecognized_form_type` warnings in `validation`.
    pub skipped: Vec<SkippedLine>,
    pub line_count: usize,
    /// Body lines per table, keyed by the table's serialised name.
    pub tables: BTreeMap<Table, usize>,
    pub validation: Validation,
    pub reconciliation: Option<Reconciliation>,
    pub reconcile_error: Option<String>,
}

fn parse_document(bytes: &[u8]) -> Result<ParsedDocument, ToolsError> {
    let lenient = Filing::parse_bytes_with(bytes, &ParseOptions::LENIENT)
        .map_err(|e| ToolsError::BadRequest(format!("could not parse filing: {e}")))?;
    let validation = lenient.validate();
    let (filing, skipped) = lenient.into_parts();
    Ok(describe(filing, skipped, validation))
}

fn describe(filing: Filing, skipped: Vec<SkippedLine>, validation: Validation) -> ParsedDocument {
    let (reconciliation, reconcile_error) = match filing.reconcile() {
        Ok(r) => (Some(r), None),
        Err(e) => (None, Some(e.to_string())),
    };
    let mut tables = BTreeMap::new();
    for line in &filing.lines {
        *tables.entry(line.table()).or_insert(0usize) += 1;
    }
    let Filing {
        header,
        version,
        raw_form_type,
        base_form_type,
        is_amendment,
        amends_filing,
        summary,
        lines,
    } = filing;
    ParsedDocument {
        header,
        version,
        form_type: raw_form_type,
        base_form_type,
        is_amendment,
        amends_filing,
        line_count: lines.len(),
        summary,
        lines,
        skipped,
        tables,
        validation,
        reconciliation,
        reconcile_error,
    }
}

/// `POST /tools/parse`: the body is the raw `.fec` file
/// (`Content-Type: application/octet-stream` or `text/plain`; the body is
/// decoded as UTF-8 with a Windows-1252 fallback, like the CLI).
pub async fn parse(
    Extension(limits): Extension<Limits>,
    body: Result<Bytes, BytesRejection>,
) -> Result<Json<ParsedDocument>, ToolsError> {
    let bytes = body.map_err(|rej| bytes_rejection(rej, limits))?;
    offload(move || parse_document(&bytes)).await.map(Json)
}

/// `GET /tools/fetch/{filing_id}`: downloads the filing from the
/// document store and returns the same document `POST /tools/parse` would.
/// The store is the [`Endpoints`](crate::fec::Endpoints) extension the API
/// router installs (`docquery_base`); without one, the environment's
/// ([`Endpoints::from_env`](crate::fec::Endpoints::from_env)). A filing
/// larger than the body cap is refused with 413 -- from its
/// `Content-Length` before any of it is read when the store sends one,
/// and otherwise as soon as the cap is passed, so the server never holds
/// more than the cap for a client's choice of id. A download failure,
/// including a malformed endpoint override, is 502.
#[cfg(feature = "fetch")]
pub async fn fetch(
    Extension(limits): Extension<Limits>,
    Extension(slots): Extension<FetchSlots>,
    endpoints: Option<Extension<crate::fec::Endpoints>>,
    Path(filing_id): Path<u64>,
) -> Result<Json<ParsedDocument>, ToolsError> {
    use crate::fec::{Endpoints, FecApiError, download_filing_bytes_capped};

    // Waiting here is cancelled with the request if the timeout fires,
    // which a running `spawn_blocking` download would not be.
    let _slot = slots
        .0
        .acquire()
        .await
        .map_err(|e| ToolsError::UpstreamFailed(format!("download slot unavailable: {e}")))?;
    let max_bytes = u64::try_from(limits.max_body_bytes).unwrap_or(u64::MAX);
    // Download and parse in one blocking task: both are off the runtime,
    // and the bytes never cross back to it.
    offload(move || {
        let endpoints = match endpoints {
            Some(Extension(endpoints)) => endpoints,
            None => Endpoints::from_env().map_err(|e| upstream(filing_id, &e.into()))?,
        };
        let bytes = download_filing_bytes_capped(filing_id, &endpoints, max_bytes).map_err(
            |e| match e {
                FecApiError::TooLarge { content_length, .. } => {
                    ToolsError::PayloadTooLarge(format!(
                        "filing {filing_id} is{} larger than this server's {} byte limit; use `hardmoney parse {filing_id}` instead",
                        content_length.map_or(String::new(), |n| format!(" {n} bytes,")),
                        limits.max_body_bytes
                    ))
                }
                other => upstream(filing_id, &other),
            },
        )?;
        parse_document(&bytes)
    })
    .await
    .map(Json)
}

#[cfg(feature = "fetch")]
fn upstream(filing_id: u64, e: &crate::fec::FecApiError) -> ToolsError {
    ToolsError::UpstreamFailed(format!(
        "could not download filing {filing_id} from the FEC: {e}"
    ))
}

/// `GET /tools/fetch/{filing_id}` in a build without the `fetch` feature:
/// always 501.
#[cfg(not(feature = "fetch"))]
pub async fn fetch(Path(filing_id): Path<u64>) -> ToolsError {
    ToolsError::NotImplemented(format!(
        "cannot fetch filing {filing_id}: this build of hardmoney has no `fetch` feature"
    ))
}

// ---------------------------------------------------------------------------
// Document (the editable form of a filing)
// ---------------------------------------------------------------------------

/// A filing as JSON, the input to `validate`, `reconcile`, and `write`.
///
/// `version` is the spec version every line is laid out with (`"8.5"`).
/// `summary.fields.form_type` must be the cover line's token as filed
/// (`F3XN`, not `F3X`), and every body line's `fields.form_type` must be
/// its token as filed too (`SA11AI`, `SB21B`, `TEXT`): a schedule has
/// many tokens and no default one. A form-type token that belongs to a
/// different table than the record's `table` is a 400. `line_no` is
/// optional; when absent the cover is line 2 and body lines are numbered
/// from 3, matching a written file. Unknown keys are ignored, so the
/// output of `POST /tools/parse` can be sent back unchanged.
#[derive(Debug, Deserialize)]
pub struct Document {
    pub version: String,
    #[serde(default)]
    pub header: HeaderInput,
    pub summary: LineInput,
    #[serde(default)]
    pub lines: Vec<LineInput>,
}

/// The `HDR` record's columns. Blank `record_type`/`ef_type` default to
/// `HDR`/`FEC`; a blank `fec_version_raw` defaults to the document's
/// `version`. `name_delim` is only written for spec 3.x-5.x.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct HeaderInput {
    pub record_type: String,
    pub ef_type: String,
    pub fec_version_raw: String,
    pub soft_name: String,
    pub soft_ver: String,
    pub name_delim: Option<String>,
    pub report_id: String,
    pub report_number: String,
    pub comment: String,
}

/// One record: its table (`"SchA"`, `"F3X"`; case-insensitive) and its
/// fields by canonical name. Values must be strings (`null` is blank);
/// fields not listed are blank.
#[derive(Debug, Deserialize)]
pub struct LineInput {
    pub table: String,
    #[serde(default)]
    pub line_no: Option<u64>,
    #[serde(default)]
    pub fields: serde_json::Map<String, Value>,
}

fn build_line(
    input: &LineInput,
    version: SpecVersion,
    default_line_no: u64,
) -> Result<ParsedLine, ToolsError> {
    let line_no = input.line_no.unwrap_or(default_line_no);
    let table: Table = input.table.trim().parse().map_err(|_| {
        ToolsError::BadRequest(format!(
            "line {line_no}: unknown table '{}' (expected a name like SchA or F3X)",
            input.table
        ))
    })?;
    let mut pairs = Vec::with_capacity(input.fields.len());
    for (name, value) in &input.fields {
        let v = match value {
            Value::String(s) => s.as_str(),
            Value::Null => "",
            other => {
                return Err(ToolsError::BadRequest(format!(
                    "line {line_no}: field '{name}' must be a string, not {other}"
                )));
            }
        };
        pairs.push((name.as_str(), v));
    }
    // `Filing::from_parts` rejects a record whose form-type token belongs
    // to another table -- or is blank, since `ParsedLine::from_pairs` then
    // fills in the table's name, which is not a token for any schedule --
    // but its message ("no format table for ...") is about a token nobody
    // wrote; say what actually disagrees.
    let token = pairs
        .iter()
        .find(|(k, _)| *k == "form_type")
        .map(|(_, v)| v.trim())
        .unwrap_or("");
    if token.is_empty() {
        if table_for_form_type(table.as_str()) != Some(table) {
            return Err(ToolsError::BadRequest(format!(
                "line {line_no}: fields.form_type is required for a {table} record (the form-type token as filed, for example {})",
                example_token(table)
            )));
        }
    } else if let Some(actual) = table_for_form_type(&token.to_ascii_uppercase())
        && actual != table
    {
        return Err(ToolsError::BadRequest(format!(
            "line {line_no}: table '{}' does not match form type '{token}', which is a {actual} record",
            input.table.trim()
        )));
    }
    ParsedLine::from_pairs(table, version, line_no, pairs)
        .map_err(|e| ToolsError::BadRequest(format!("line {line_no}: {e}")))
}

/// The delimited cells the document's records would occupy when written:
/// the layout width of every record whose table is known at `version`.
/// A lower bound on the written file's size in bytes (each cell costs at
/// least its delimiter), used to refuse a document that is small as JSON
/// but large as a filing before any line is built.
fn written_cells(doc: &Document, version: SpecVersion) -> usize {
    std::iter::once(&doc.summary)
        .chain(&doc.lines)
        .filter_map(|l| l.table.trim().parse::<Table>().ok())
        .filter_map(|t| t.layout(version))
        .fold(0usize, |n, layout| {
            n.saturating_add(usize::from(layout.width))
        })
}

/// A form-type token a record of `table` could carry, for error messages.
/// The common schedules get their usual line token; anything else gets
/// the FEC's spec sample if there is one, else the table name.
fn example_token(table: Table) -> String {
    match table {
        Table::SchA => "SA11AI".to_string(),
        Table::SchB => "SB21B".to_string(),
        Table::SchC => "SC/10".to_string(),
        Table::SchD => "SD10".to_string(),
        Table::SchE => "SE".to_string(),
        _ => table
            .spec("form_type")
            .and_then(|s| s.sample)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| table.as_str().to_string()),
    }
}

/// Rebuilds a [`Filing`] from a [`Document`], reporting the first
/// inconsistency as a 400 that names the line, and a document that would
/// write to more than `limits.max_body_bytes` as a 413.
fn build_filing(doc: &Document, limits: Limits) -> Result<Filing, ToolsError> {
    let version: SpecVersion = doc
        .version
        .parse()
        .map_err(|e: crate::parser::InvalidSpecVersion| ToolsError::BadRequest(e.to_string()))?;
    let cells = written_cells(doc, version);
    if cells > limits.max_body_bytes {
        return Err(ToolsError::PayloadTooLarge(format!(
            "the document's {} record(s) would write to at least {cells} bytes, more than this server's {} byte limit; use the `hardmoney` CLI for large filings",
            doc.lines.len().saturating_add(1),
            limits.max_body_bytes
        )));
    }

    let h = &doc.header;
    let fec_version_raw = if h.fec_version_raw.trim().is_empty() {
        version.to_string()
    } else {
        h.fec_version_raw.clone()
    };
    let or_default = |value: &str, default: &'static str| -> String {
        if value.trim().is_empty() {
            default.to_string()
        } else {
            value.to_string()
        }
    };
    let record_type = or_default(&h.record_type, "HDR");
    let ef_type = or_default(&h.ef_type, "FEC");
    // Wire order, as `Header::to_fields` emits it.
    let mut fields: Vec<&str> = vec![
        &record_type,
        &ef_type,
        &fec_version_raw,
        &h.soft_name,
        &h.soft_ver,
    ];
    if version.has_name_delim_header() {
        fields.push(h.name_delim.as_deref().unwrap_or(""));
    }
    fields.extend([h.report_id.as_str(), &h.report_number, &h.comment]);
    let header =
        Header::from_fields(&fields).map_err(|e| ToolsError::BadRequest(format!("header: {e}")))?;
    if header.version != version {
        return Err(ToolsError::BadRequest(format!(
            "header.fec_version_raw '{}' is spec version {}, but the document's version is {version}",
            header.fec_version_raw, header.version
        )));
    }

    let cover_token = doc
        .summary
        .fields
        .get("form_type")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    if cover_token.is_empty() {
        return Err(ToolsError::BadRequest(
            "summary.fields.form_type must be the cover line's form-type token as filed (for example F3XN)"
                .to_string(),
        ));
    }
    let summary = build_line(&doc.summary, version, 2)?;
    let lines = doc
        .lines
        .iter()
        .enumerate()
        .map(|(i, l)| build_line(l, version, (i as u64).saturating_add(3)))
        .collect::<Result<Vec<_>, _>>()?;
    Filing::from_parts(header, summary, lines).map_err(|e| ToolsError::BadRequest(e.to_string()))
}

fn take_document(
    body: Result<Json<Document>, JsonRejection>,
    limits: Limits,
) -> Result<Document, ToolsError> {
    body.map(|Json(d)| d)
        .map_err(|rej| json_rejection(rej, limits))
}

/// `POST /tools/validate`: [`Filing::validate`] on a [`Document`].
pub async fn validate(
    Extension(limits): Extension<Limits>,
    body: Result<Json<Document>, JsonRejection>,
) -> Result<Json<Validation>, ToolsError> {
    let doc = take_document(body, limits)?;
    offload(move || Ok(build_filing(&doc, limits)?.validate()))
        .await
        .map(Json)
}

/// `POST /tools/reconcile`: [`Filing::reconcile`] on a [`Document`]; 400
/// with the reason for a cover form that has no rules.
pub async fn reconcile(
    Extension(limits): Extension<Limits>,
    body: Result<Json<Document>, JsonRejection>,
) -> Result<Json<Reconciliation>, ToolsError> {
    let doc = take_document(body, limits)?;
    offload(move || {
        build_filing(&doc, limits)?
            .reconcile()
            .map_err(|e| ToolsError::BadRequest(e.to_string()))
    })
    .await
    .map(Json)
}

/// Keeps ASCII letters, digits, `-`, and `_` for a download filename.
fn filename_token(raw: &str, fallback: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    if cleaned.is_empty() {
        fallback.to_string()
    } else {
        cleaned
    }
}

/// `POST /tools/write`: [`Filing::to_fec`] on a [`Document`]. The response
/// is the `.fec` bytes with `Content-Disposition: attachment;
/// filename="<form>-<committee>.fec"` and `X-Hardmoney-Validation-Errors`
/// set to the number of error-severity findings (the file is written even
/// when that is non-zero; the FEC would reject it).
pub async fn write(
    Extension(limits): Extension<Limits>,
    body: Result<Json<Document>, JsonRejection>,
) -> Result<Response, ToolsError> {
    let doc = take_document(body, limits)?;
    let (filename, errors, bytes) = offload(move || {
        let filing = build_filing(&doc, limits)?;
        let errors = filing.validate().error_count();
        let committee = filing
            .summary
            .get_non_empty("filer_committee_id_number")
            .or_else(|| filing.summary.get_non_empty("candidate_id_number"))
            .unwrap_or("filing");
        let filename = format!(
            "{}-{}.fec",
            filename_token(&filing.raw_form_type, "filing"),
            filename_token(committee, "filing")
        );
        Ok((filename, errors, filing.to_fec()))
    })
    .await?;

    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{filename}\"")) {
        headers.insert(header::CONTENT_DISPOSITION, v);
    }
    if let Ok(v) = HeaderValue::from_str(&errors.to_string()) {
        headers.insert("x-hardmoney-validation-errors", v);
    }
    Ok(response)
}

// ---------------------------------------------------------------------------
// Spec
// ---------------------------------------------------------------------------

/// `GET /tools/spec/{table}` query: `version` defaults to the bundled spec
/// version.
#[derive(Debug, Deserialize)]
pub struct SpecParams {
    pub version: Option<String>,
}

/// One column of a layout with the FEC's specification for it, when the
/// bundled spec still documents the field.
#[derive(Debug, Serialize)]
pub struct FieldInfo {
    pub name: &'static str,
    pub column: u16,
    pub spec: Option<&'static FieldSpec>,
}

/// A table's layout at one spec version, with field specs.
#[derive(Debug, Serialize)]
#[non_exhaustive]
pub struct TableSpec {
    pub table: Table,
    pub version: SpecVersion,
    /// The version the `spec` entries describe (`FieldSpec` is bundled for
    /// one version only).
    pub bundled_spec_version: &'static str,
    /// Every version this layout applies to.
    pub versions: &'static [SpecVersion],
    /// Delimited cells a writer emits for this table.
    pub width: u16,
    pub fields: Vec<FieldInfo>,
}

/// `GET /tools/spec/{table}?version=`: 404 for an unknown table or a
/// version the bundled data has no layout for.
pub async fn spec(
    Path(table): Path<String>,
    Query(params): Query<SpecParams>,
) -> Result<Json<TableSpec>, ToolsError> {
    let table: Table = table
        .trim()
        .parse()
        .map_err(|_| ToolsError::NotFound(format!("no table named '{table}'")))?;
    let version: SpecVersion = params
        .version
        .as_deref()
        .unwrap_or(BUNDLED_SPEC_VERSION)
        .parse()
        .map_err(|e: crate::parser::InvalidSpecVersion| ToolsError::BadRequest(e.to_string()))?;
    let layout = table.layout(version).ok_or_else(|| {
        ToolsError::NotFound(format!(
            "table {table} has no column layout at spec version {version}"
        ))
    })?;
    let fields = layout
        .fields
        .iter()
        .map(|f| FieldInfo {
            name: f.name,
            column: f.column,
            spec: table.spec(f.name),
        })
        .collect();
    Ok(Json(TableSpec {
        table,
        version,
        bundled_spec_version: BUNDLED_SPEC_VERSION,
        versions: layout.versions,
        width: layout.width,
        fields,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(json: Value) -> Document {
        serde_json::from_value(json).unwrap()
    }

    const NO_LIMIT: Limits = Limits {
        max_body_bytes: usize::MAX,
    };

    #[test]
    fn build_filing_defaults_header_columns_and_numbers_lines() {
        let d = doc(json!({
            "version": "8.5",
            "summary": { "table": "F3X", "fields": { "form_type": "F3XN", "filer_committee_id_number": "C00123456" } },
            "lines": [ { "table": "SchA", "fields": { "form_type": "SA11AI", "contribution_amount": "10.00" } } ]
        }));
        let f = build_filing(&d, NO_LIMIT).unwrap();
        assert_eq!(f.header.record_type, "HDR");
        assert_eq!(f.header.ef_type, "FEC");
        assert_eq!(f.header.fec_version_raw, "8.5");
        assert_eq!(f.summary.line_no, 2);
        assert_eq!(f.lines[0].line_no, 3);
        assert_eq!(f.base_form_type, "F3X");
        assert!(f.to_fec_string().starts_with("HDR\u{1c}FEC\u{1c}8.5\u{1c}"));
    }

    #[test]
    fn build_filing_rejects_each_inconsistency_with_a_message() {
        let cases: [(Value, &str); 9] = [
            (
                json!({ "version": "abc", "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } } }),
                "not an FEC spec version",
            ),
            (
                json!({ "version": "9.9", "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } } }),
                "unsupported or malformed FEC version",
            ),
            (
                json!({ "version": "8.5", "header": { "fec_version_raw": "8.4" },
                        "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } } }),
                "document's version is 8.5",
            ),
            (
                json!({ "version": "8.5", "summary": { "table": "F3X", "fields": {} } }),
                "summary.fields.form_type",
            ),
            (
                json!({ "version": "8.5", "summary": { "table": "Nope", "fields": { "form_type": "F3XN" } } }),
                "unknown table 'Nope'",
            ),
            (
                json!({ "version": "8.5", "summary": { "table": "F3X", "fields": { "form_type": "F3XN", "bogus": "1" } } }),
                "no field named 'bogus'",
            ),
            (
                json!({ "version": "8.5", "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } },
                        "lines": [ { "table": "SchA", "line_no": 12, "fields": { "contribution_amount": 10 } } ] }),
                "line 12: field 'contribution_amount' must be a string",
            ),
            // A record whose `table` disagrees with its form-type token.
            (
                json!({ "version": "8.5", "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } },
                        "lines": [ { "table": "SchB", "line_no": 7, "fields": { "form_type": "SA11AI" } } ] }),
                "line 7: table 'SchB' does not match form type 'SA11AI', which is a SchA record",
            ),
            // A cover whose `table` disagrees with the document's form type.
            (
                json!({ "version": "8.5", "summary": { "table": "F3", "fields": { "form_type": "F3XN" } } }),
                "line 2: table 'F3' does not match form type 'F3XN', which is a F3X record",
            ),
        ];
        for (input, expected) in cases {
            match build_filing(&doc(input), NO_LIMIT) {
                Err(ToolsError::BadRequest(msg)) => {
                    assert!(
                        msg.contains(expected),
                        "{msg:?} should mention {expected:?}"
                    )
                }
                other => panic!("expected BadRequest mentioning {expected:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn old_versions_carry_the_name_delim_header_column() {
        let d = doc(json!({
            "version": "5.3",
            "header": { "soft_name": "X", "soft_ver": "1", "name_delim": "^" },
            "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } }
        }));
        let f = build_filing(&d, NO_LIMIT).unwrap();
        assert_eq!(f.header.name_delim.as_deref(), Some("^"));
        assert_eq!(f.header.to_fields().len(), 9);
    }

    /// Many tiny JSON records build many full-width lines: the written
    /// size, not the JSON size, is what the body cap bounds.
    #[test]
    fn a_document_that_would_write_past_the_body_cap_is_413() {
        let lines: Vec<Value> = (0..100)
            .map(|_| json!({ "table": "SchA", "fields": { "form_type": "SA11AI" } }))
            .collect();
        let d = doc(json!({
            "version": "8.5",
            "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } },
            "lines": lines
        }));
        let cells = written_cells(&d, SpecVersion::electronic(8, 5));
        let sch_a = usize::from(
            Table::SchA
                .layout(SpecVersion::electronic(8, 5))
                .unwrap()
                .width,
        );
        assert!(
            cells > 100 * sch_a,
            "{cells} cells for 100 SchA lines + cover"
        );

        let tight = Limits {
            max_body_bytes: cells - 1,
        };
        match build_filing(&d, tight) {
            Err(ToolsError::PayloadTooLarge(msg)) => {
                assert!(msg.contains("101 record(s)"), "{msg}");
                assert!(msg.contains(&format!("{} byte limit", cells - 1)), "{msg}");
            }
            other => panic!("expected 413, got {other:?}"),
        }
        let exact = Limits {
            max_body_bytes: cells,
        };
        assert!(build_filing(&d, exact).is_ok());
    }

    #[test]
    fn filename_token_strips_unsafe_characters() {
        assert_eq!(filename_token("F3XA", "x"), "F3XA");
        assert_eq!(filename_token("../C00\"1", "x"), "C001");
        assert_eq!(filename_token("///", "filing"), "filing");
    }
}
