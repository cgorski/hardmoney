//! WebCheck: the FEC's own validator as a conformance oracle for
//! [`Filing::validate`](crate::Filing::validate).
//!
//! The FEC runs every electronic filing through one validator before
//! accepting it. The same engine is exposed at
//! <https://efoservices.fec.gov/webcheck/> ("WebCheck"), where a filer --
//! or the FEC's own FECfile+ team, which has no validator step of its own
//! -- uploads a `.fec` and reads the failing and warning messages back.
//! This module submits a file there, parses the report, and diffs it
//! against ours, so that a rule we implement differently from the FEC is
//! visible as `only ours` / `only theirs` rather than an argument.
//!
//! ```no_run
//! use hardmoney::Filing;
//! use hardmoney::parser::webcheck::{WebCheck, diff};
//!
//! let bytes = std::fs::read("filing.fec")?;
//! let ours = Filing::parse_bytes(&bytes)?.validate();
//! let theirs = WebCheck::new().submit("filing.fec", &bytes, None)?;
//! let d = diff(&ours, &theirs);
//! print!("{d}");
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! # Two channels
//!
//! WebCheck has two entry points, and this module speaks both:
//!
//! * **The public upload** (`POST /webcheck/services/upload`,
//!   `multipart/form-data`) is what the WebCheck web page itself calls. It
//!   needs **no credentials** and answers synchronously with an HTML
//!   fragment: a `validationObj` JSON literal (`SUCCESS` / `ERRORS`),
//!   `errorsCount` / `warningsCount`, and one numbered block per finding
//!   giving the record type (with the record's name in braces), the
//!   1-based field number and the FEC's field label, and the message. It
//!   does **not** report line numbers. Files over 20 MB get their results
//!   by e-mail instead ([`WebCheckError::Deferred`]). This is the channel
//!   used when [`WebCheck::submit`] is given no [`Credentials`], and the
//!   one this module has been tested against (2026-09-15: all 25 accepted
//!   fixtures pass; each `tests/fixtures/invalid/*.fec` fails as
//!   expected).
//! * **The SOAP service** (`POST /webcheck/services/validate`, operation
//!   `validate(arg0: string, arg1: string, arg2: base64Binary) -> string`,
//!   WSDL at `?wsdl`) is the vendor interface. It answers with an MTOM
//!   `multipart/related` body wrapping a SOAP envelope whose `<return>` is
//!   a JSON object (`status`, `message`, `msg_url`, `submission_id`,
//!   `batch_id`, `success`), and a `status(batch_id, submission_id)`
//!   operation exists for polling. Every request without a valid **vendor
//!   API key** (issued through the FEC's Vendor Registration) is answered
//!   `"Error! API Key is invalid."` -- whatever the arguments, even an
//!   empty body -- so the meaning of `arg0`/`arg1` cannot be learned
//!   without a key. This module sends the key as `arg0` and the contact
//!   e-mail as `arg1` (the WSDL loses the parameter names, and the
//!   service's sister `webload` `upload` operation takes its key and
//!   e-mail in a JSON payload, so this is the closest documented shape),
//!   decodes the JSON, and follows a `msg_url` if one comes back. **The
//!   path beyond the key check is untested**: with a key, expect to adjust
//!   [`soap_envelope`]'s argument order.
//!
//! # Dependencies
//!
//! Everything here is hand-rolled on `ureq` plus the crate's existing
//! `regex` and `encoding_rs`: a 30-line base64 encoder ([`base64_encode`])
//! rather than the `base64` crate, a multipart body builder rather than
//! `ureq`'s optional `multipart` feature, and string scanning rather than
//! an XML or JSON parser. The parser-only build (`--no-default-features
//! --features fetch`) stays as light as before, and the responses are
//! simple enough that a tolerant scanner is the right tool: nothing here
//! panics on an unexpected shape -- it returns
//! [`WebCheckError::Unparseable`] with a snippet, or a finding whose
//! `message` is the raw text.
//!
//! # Matching ours to theirs
//!
//! [`diff`] pairs findings by `(record type, 1-based field number,
//! message template)`, as a multiset. Ours get the field number from the
//! bundled spec ([`FieldSpec::column`](crate::parser::FieldSpec::column) + 1) and the template from
//! [`Rule::fec_message`]; theirs give the number directly. A template
//! matches a WebCheck message when the template's words, blanks removed,
//! appear in order in the message's words -- so `Tran ID is NOT UNIQUE -
//! This one is same as other(s)` matches `Tran ID 'SB21B.4120' is NOT
//! UNIQUE - This one is same as other(s)`. Where the live validator's
//! wording differs from the FEC's published list, [`live_alternates`]
//! carries the observed form (e.g. `Filing Format must be Version 8.5`
//! for `Filing must be in the current FEC format`).

use std::collections::HashMap;
use std::fmt;
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;

use crate::fec::{EndpointError, Endpoints};
use crate::parser::form::table_for_form_type;
use crate::parser::tables::Table;
use crate::parser::validate::{Finding, Rule, Severity, Validation};

// ---------------------------------------------------------------------------
// Endpoints and client
// ---------------------------------------------------------------------------

/// The WebCheck application's production base URL: the default of
/// [`Endpoints::webcheck_endpoint`].
pub const DEFAULT_ENDPOINT: &str = Endpoints::DEFAULT_WEBCHECK_ENDPOINT;

/// Path (under the endpoint) of the credential-free multipart upload the
/// WebCheck web page uses.
pub const UPLOAD_PATH: &str = "/services/upload";

/// Path (under the endpoint) of the SOAP `ValidateService`.
pub const SOAP_PATH: &str = "/services/validate";

/// The SOAP service's XML namespace, from the WSDL.
pub const SOAP_NAMESPACE: &str = "http://service.webcheck.efo.fec.gov/";

/// WebCheck's stated cut-off above which results are e-mailed rather
/// than returned.
pub const EMAIL_RESULTS_ABOVE_BYTES: u64 = 20 * 1024 * 1024;

/// Vendor credentials for the SOAP channel.
///
/// Not needed for the public upload channel; see the [module
/// docs](self).
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Credentials {
    /// A vendor API key from the FEC's Vendor Registration.
    pub api_key: String,
    /// Contact e-mail; WebCheck mails results for files over 20 MB.
    pub email: Option<String>,
}

impl Credentials {
    /// Credentials with an API key and no e-mail.
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            email: None,
        }
    }

    /// Sets the contact e-mail.
    #[must_use]
    pub fn with_email(mut self, email: impl Into<String>) -> Self {
        self.email = Some(email.into());
        self
    }
}

impl fmt::Debug for Credentials {
    /// The key is redacted.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Credentials")
            .field("api_key", &"REDACTED")
            .field("email", &self.email)
            .finish()
    }
}

/// A client for the FEC's WebCheck validator.
#[derive(Debug, Clone)]
pub struct WebCheck {
    /// Base URL; [`UPLOAD_PATH`] and [`SOAP_PATH`] are appended.
    pub endpoint: String,
    agent: ureq::Agent,
}

impl Default for WebCheck {
    fn default() -> Self {
        Self::new()
    }
}

impl WebCheck {
    /// A client for the live service at [`DEFAULT_ENDPOINT`]. Does not
    /// read the environment: an infallible constructor cannot report a
    /// malformed override, so the `HARDMONEY_WEBCHECK_ENDPOINT` variable
    /// is honoured by [`WebCheck::from_env`] (and by the CLI's
    /// `--webcheck-endpoint`), not here.
    ///
    /// Generous timeouts: WebCheck warns of delays during heavy filing
    /// periods, and a large filing's report can run to tens of megabytes.
    #[must_use]
    pub fn new() -> Self {
        Self::with_endpoint(DEFAULT_ENDPOINT)
    }

    /// A client for [`Endpoints::webcheck_endpoint`] as read from the
    /// environment ([`Endpoints::from_env`]): the live service unless
    /// `HARDMONEY_WEBCHECK_ENDPOINT` is set. Fails, naming the variable,
    /// if an override is not a usable URL; never falls back silently.
    pub fn from_env() -> Result<Self, EndpointError> {
        Ok(Self::with_endpoints(&Endpoints::from_env()?))
    }

    /// A client for [`Endpoints::webcheck_endpoint`].
    #[must_use]
    pub fn with_endpoints(endpoints: &Endpoints) -> Self {
        Self::with_endpoint(endpoints.webcheck_endpoint.as_str())
    }

    /// A client for another base URL (a mirror, or a test server). Sets
    /// the same value as [`WebCheck::with_endpoints`], without the URL
    /// check.
    #[must_use]
    pub fn with_endpoint(endpoint: impl Into<String>) -> Self {
        let agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(crate::fec::USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(600)))
            .build()
            .new_agent();
        Self {
            endpoint: endpoint.into().trim_end_matches('/').to_string(),
            agent,
        }
    }

    /// The URL of the public upload channel: [`UPLOAD_PATH`] under the
    /// endpoint.
    #[must_use]
    pub fn upload_url(&self) -> String {
        format!("{}{UPLOAD_PATH}", self.endpoint.trim_end_matches('/'))
    }

    /// The URL of the SOAP service: [`SOAP_PATH`] under the endpoint.
    #[must_use]
    pub fn soap_url(&self) -> String {
        format!("{}{SOAP_PATH}", self.endpoint.trim_end_matches('/'))
    }

    /// Submits `bytes` (named `filename`; WebCheck's page accepts only
    /// `.fec`) and returns the FEC's report.
    ///
    /// Without `credentials` the public upload channel is used; with them,
    /// the SOAP service (see the [module docs](self) for the state of
    /// each). Fails on a transport error, a non-2xx status, a SOAP fault,
    /// a service-level rejection such as an invalid API key
    /// ([`WebCheckError::Rejected`]), a response whose results will be
    /// e-mailed ([`WebCheckError::Deferred`]), or a body this module
    /// cannot read ([`WebCheckError::Unparseable`]).
    pub fn submit(
        &self,
        filename: &str,
        bytes: &[u8],
        credentials: Option<&Credentials>,
    ) -> Result<OracleReport, WebCheckError> {
        match credentials {
            None => self.submit_upload(filename, bytes),
            Some(c) => self.submit_soap(filename, bytes, c),
        }
    }

    /// The public upload channel: `multipart/form-data` with a `file`
    /// part (and an empty `email`, as the page sends).
    pub fn submit_upload(
        &self,
        filename: &str,
        bytes: &[u8],
    ) -> Result<OracleReport, WebCheckError> {
        let boundary = multipart_boundary(bytes);
        let body = multipart_body(&boundary, filename, "", bytes);
        let url = self.upload_url();
        let response = self
            .agent
            .post(&url)
            .header(
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            )
            .header("Accept", "*/*")
            .send(&body[..])?;
        let text = read_text(response, &url)?;
        parse_upload_response(&text)
    }

    /// The SOAP channel (untested past the API-key check; see the [module
    /// docs](self)).
    pub fn submit_soap(
        &self,
        filename: &str,
        bytes: &[u8],
        credentials: &Credentials,
    ) -> Result<OracleReport, WebCheckError> {
        let envelope = soap_envelope(
            &credentials.api_key,
            credentials.email.as_deref().unwrap_or(""),
            bytes,
        );
        let url = self.soap_url();
        let response = self
            .agent
            .post(&url)
            .header("Content-Type", "text/xml; charset=utf-8")
            .header("SOAPAction", "\"\"")
            .header("Accept", "*/*")
            .send(envelope.as_bytes())?;
        let status = response.status().as_u16();
        // A SOAP fault comes back as HTTP 500 with an XML body; decode it
        // before treating the status as a plain HTTP failure.
        let text = if status == 500 {
            read_body_text(response)?
        } else {
            read_text(response, &url)?
        };
        let ret = parse_soap_response(&text)?;
        if !ret.success {
            return Err(WebCheckError::Rejected {
                message: if ret.message.is_empty() {
                    ret.status.clone()
                } else {
                    ret.message.clone()
                },
            });
        }
        if !ret.msg_url.is_empty() {
            let follow = self.agent.get(&ret.msg_url).call()?;
            let report_text = read_text(follow, &ret.msg_url)?;
            return parse_report_text(&report_text);
        }
        // Success with no results URL: we cannot know where the report is
        // without a key to experiment with. Hand back what the service
        // said so a caller can act on `submission_id`/`batch_id`.
        Err(WebCheckError::Unparseable {
            snippet: snippet(&format!(
                "SOAP validate succeeded (status {status}, submission_id {:?}, batch_id {}) but \
                 returned no results URL; filename {filename}: {}",
                ret.submission_id, ret.batch_id, ret.raw
            )),
        })
    }
}

/// Reads a 2xx response's body as text (charset from `Content-Type`,
/// Windows-1252 otherwise -- WebCheck answers `ISO-8859-1`); a non-2xx
/// status becomes [`WebCheckError::Http`] with a body snippet.
fn read_text(
    response: ureq::http::Response<ureq::Body>,
    url: &str,
) -> Result<String, WebCheckError> {
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return read_body_text(response);
    }
    let body_snippet = read_body_text(response)
        .map(|s| snippet(&s))
        .unwrap_or_default();
    Err(WebCheckError::Http {
        status,
        url: url.to_string(),
        body_snippet,
    })
}

/// Largest response body this module will read (a 32,000-problem
/// WebCheck report is roughly 50 MB of HTML).
const MAX_BODY_BYTES: u64 = 512 * 1024 * 1024;

fn read_body_text(response: ureq::http::Response<ureq::Body>) -> Result<String, WebCheckError> {
    let (_, mut body) = response.into_parts();
    let utf8 = body
        .charset()
        .is_some_and(|c| c.eq_ignore_ascii_case("utf-8") || c.eq_ignore_ascii_case("utf8"));
    let bytes = body.with_config().limit(MAX_BODY_BYTES).read_to_vec()?;
    Ok(if utf8 {
        String::from_utf8_lossy(&bytes).into_owned()
    } else {
        encoding_rs::WINDOWS_1252.decode(&bytes).0.into_owned()
    })
}

/// Maximum characters kept for an error snippet.
const SNIPPET_CHARS: usize = 400;

fn snippet(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= SNIPPET_CHARS {
        collapsed
    } else {
        let mut out: String = collapsed.chars().take(SNIPPET_CHARS).collect();
        out.push_str("...");
        out
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a WebCheck submission produced no report.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WebCheckError {
    /// Could not reach the server, or the connection failed mid-response.
    #[error("WebCheck transport error: {0}")]
    Transport(#[from] ureq::Error),

    /// A non-2xx HTTP status that was not a SOAP fault.
    #[error("{url} returned HTTP {status}{}", body_hint(body_snippet))]
    Http {
        status: u16,
        url: String,
        /// The first few hundred characters of the body, whitespace-collapsed.
        body_snippet: String,
    },

    /// The SOAP service returned a `<soap:Fault>`.
    #[error("WebCheck SOAP fault {code}: {string}")]
    SoapFault { code: String, string: String },

    /// The service answered, but refused to validate (`"success": false`
    /// -- an invalid API key, an unreadable upload).
    #[error("WebCheck rejected the submission: {message}")]
    Rejected { message: String },

    /// The service accepted the file but will e-mail the results (files
    /// over 20 MB) rather than return them.
    #[error("WebCheck will e-mail the results rather than return them: {message}")]
    Deferred { message: String },

    /// The response was not in a shape this module knows.
    #[error("could not parse WebCheck's response: {snippet}")]
    Unparseable { snippet: String },
}

fn body_hint(snippet: &str) -> String {
    if snippet.is_empty() {
        String::new()
    } else {
        format!(": {snippet}")
    }
}

// ---------------------------------------------------------------------------
// Request bodies
// ---------------------------------------------------------------------------

/// Standard base64 (RFC 4648 §4, with `=` padding) of `bytes`.
///
/// Hand-rolled to keep the parser-only build free of another crate; the
/// SOAP `arg2` is the only base64 in hardmoney.
#[must_use]
pub fn base64_encode(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    // `& 0x3f` keeps the index below 64, so the `get` never misses; the
    // fallbacks only exist to keep the function panic-free by construction.
    let sextet = |n: u32, shift: u32| {
        let index = usize::try_from((n >> shift) & 0x3f).unwrap_or(usize::MAX);
        ALPHABET.get(index).map_or('=', |&b| char::from(b))
    };
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    let (chunks, remainder) = bytes.as_chunks::<3>();
    for [b0, b1, b2] in chunks {
        let n = (u32::from(*b0) << 16) | (u32::from(*b1) << 8) | u32::from(*b2);
        for shift in [18, 12, 6, 0] {
            out.push(sextet(n, shift));
        }
    }
    match remainder {
        [b0] => {
            let n = u32::from(*b0) << 16;
            out.push(sextet(n, 18));
            out.push(sextet(n, 12));
            out.push_str("==");
        }
        [b0, b1] => {
            let n = (u32::from(*b0) << 16) | (u32::from(*b1) << 8);
            out.push(sextet(n, 18));
            out.push(sextet(n, 12));
            out.push(sextet(n, 6));
            out.push('=');
        }
        _ => {}
    }
    out
}

/// Escapes `&`, `<`, `>`, `"` for XML text content.
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(ch),
        }
    }
    out
}

/// The SOAP 1.1 request envelope for `validate(arg0, arg1, arg2)`: a
/// document/literal body in [`SOAP_NAMESPACE`] with `arg0` and `arg1`
/// XML-escaped and `arg2` the base64 of `bytes`.
///
/// `arg0`/`arg1` are what this module believes to be the API key and the
/// contact e-mail; see the [module docs](self) for why that is a belief.
#[must_use]
pub fn soap_envelope(arg0: &str, arg1: &str, bytes: &[u8]) -> String {
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>",
            "<soapenv:Envelope xmlns:soapenv=\"http://schemas.xmlsoap.org/soap/envelope/\" ",
            "xmlns:ser=\"{ns}\">",
            "<soapenv:Header/>",
            "<soapenv:Body>",
            "<ser:validate>",
            "<arg0>{a0}</arg0>",
            "<arg1>{a1}</arg1>",
            "<arg2>{a2}</arg2>",
            "</ser:validate>",
            "</soapenv:Body>",
            "</soapenv:Envelope>"
        ),
        ns = SOAP_NAMESPACE,
        a0 = xml_escape(arg0),
        a1 = xml_escape(arg1),
        a2 = base64_encode(bytes),
    )
}

/// A multipart boundary that does not occur in `bytes`.
///
/// Derived from the content length and a simple checksum rather than a
/// random source, so the request is reproducible; extended with `-` until
/// it is absent from the payload.
fn multipart_boundary(bytes: &[u8]) -> String {
    let checksum = bytes.iter().fold(0u32, |acc, &b| {
        acc.wrapping_mul(31).wrapping_add(u32::from(b))
    });
    let mut boundary = format!("----hardmoney-{:08x}-{:x}", checksum, bytes.len());
    while bytes
        .windows(boundary.len())
        .any(|w| w == boundary.as_bytes())
    {
        boundary.push('-');
    }
    boundary
}

/// The `multipart/form-data` body the WebCheck page posts: a `file` part
/// named `filename` with `bytes`, and an `email` text part.
///
/// Quotes and backslashes in `filename` are backslash-escaped; CR and LF
/// are dropped. Pair with `Content-Type: multipart/form-data;
/// boundary=<boundary>`.
#[must_use]
pub fn multipart_body(boundary: &str, filename: &str, email: &str, bytes: &[u8]) -> Vec<u8> {
    let safe_name: String = filename
        .chars()
        .filter(|c| !matches!(c, '\r' | '\n'))
        .flat_map(|c| match c {
            '"' | '\\' => vec!['\\', c],
            _ => vec![c],
        })
        .collect();
    let mut body = Vec::with_capacity(bytes.len() + 512);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!(
            "Content-Disposition: form-data; name=\"file\"; filename=\"{safe_name}\"\r\n\
             Content-Type: application/octet-stream\r\n\r\n"
        )
        .as_bytes(),
    );
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"email\"\r\n\r\n{email}\r\n").as_bytes(),
    );
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    body
}

// ---------------------------------------------------------------------------
// The oracle's report
// ---------------------------------------------------------------------------

/// One message from WebCheck.
///
/// The upload channel reports no line numbers, so `line_no` is `None`
/// there; the record is identified by `form_type` and `item` (the
/// record's name as WebCheck prints it -- a payee, a contributor, a city)
/// and the field by its 1-based `field_no` and FEC label.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct OracleFinding {
    /// 1-based physical line, when the report gives one.
    pub line_no: Option<u64>,
    /// From the report's section (`Errors` / `Warnings`); `None` for text
    /// that arrived outside either.
    pub severity: Option<Severity>,
    /// The record type as WebCheck prints it (`F3XA`, `SB21B`, `HDR`).
    pub form_type: Option<String>,
    /// WebCheck's `{...}` identification of the record, e.g. `Election CFO`.
    pub item: Option<String>,
    /// 1-based field number (`#022`).
    pub field_no: Option<u16>,
    /// The FEC's field label (`Treasurer's Signature Date`).
    pub field_label: Option<String>,
    /// The message text, as received.
    pub message: String,
}

impl OracleFinding {
    /// A finding carrying only a message.
    #[must_use]
    pub fn from_message(message: impl Into<String>) -> Self {
        Self {
            line_no: None,
            severity: None,
            form_type: None,
            item: None,
            field_no: None,
            field_label: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for OracleFinding {
    /// `ERROR SB21B #020 Date of Expenditure {Election CFO}: <message>` --
    /// the same order as a [`Finding`]'s `Display`, with WebCheck's field
    /// number and label where we print a canonical field name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.severity {
            Some(Severity::Error) => f.write_str("ERROR")?,
            Some(Severity::Warning) => f.write_str("WARN ")?,
            None => f.write_str("?    ")?,
        }
        if let Some(n) = self.line_no {
            write!(f, " line {n}")?;
        }
        if let Some(t) = &self.form_type {
            write!(f, " {t}")?;
        }
        if let Some(n) = self.field_no {
            write!(f, " #{n:03}")?;
        }
        if let Some(l) = &self.field_label {
            write!(f, " {l}")?;
        }
        if let Some(i) = &self.item {
            write!(f, " {{{i}}}")?;
        }
        write!(f, ": {}", self.message)
    }
}

/// WebCheck's report on one file.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[must_use]
#[non_exhaustive]
pub struct OracleReport {
    /// The response body as received (HTML fragment or text).
    pub raw: String,
    /// WebCheck's verdict word: `SUCCESS` (no messages), `WARNINGS` (only
    /// warnings), `ERRORS` (at least one failing message), ...
    pub result: Option<String>,
    /// The error count WebCheck itself stated (`errorsCount`), so a
    /// finding this parser missed is detectable.
    pub errors_reported: Option<usize>,
    /// The warning count WebCheck itself stated.
    pub warnings_reported: Option<usize>,
    /// The filing type WebCheck read from the cover (`F3XA`).
    pub filing_type: Option<String>,
    /// The committee id WebCheck read from the cover.
    pub committee_id: Option<String>,
    pub findings: Vec<OracleFinding>,
}

impl OracleReport {
    /// True when the FEC would accept the filing: WebCheck said `SUCCESS`
    /// or `WARNINGS` (a warning "will not prevent the filing from being
    /// processed", in the FEC's words), or -- with no verdict word -- no
    /// error-severity finding was parsed. Only `ERRORS` (or any other
    /// verdict) is a rejection.
    #[must_use]
    pub fn is_acceptable(&self) -> bool {
        match self.result.as_deref() {
            Some(r) => r.eq_ignore_ascii_case("SUCCESS") || r.eq_ignore_ascii_case("WARNINGS"),
            None => self.error_count() == 0,
        }
    }

    /// Findings from the `Errors` section.
    pub fn errors(&self) -> impl Iterator<Item = &OracleFinding> + '_ {
        self.findings
            .iter()
            .filter(|f| f.severity == Some(Severity::Error))
    }

    /// Findings from the `Warnings` section.
    pub fn warnings(&self) -> impl Iterator<Item = &OracleFinding> + '_ {
        self.findings
            .iter()
            .filter(|f| f.severity == Some(Severity::Warning))
    }

    #[must_use]
    pub fn error_count(&self) -> usize {
        self.errors().count()
    }

    #[must_use]
    pub fn warning_count(&self) -> usize {
        self.warnings().count()
    }

    /// True when the counts WebCheck stated differ from the findings
    /// parsed -- a sign the response format has changed.
    #[must_use]
    pub fn counts_disagree(&self) -> bool {
        self.errors_reported
            .is_some_and(|n| n != self.error_count())
            || self
                .warnings_reported
                .is_some_and(|n| n != self.warning_count())
    }
}

impl fmt::Display for OracleReport {
    /// One finding per line, then a verdict line in the shape of
    /// `hardmoney validate`'s.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for finding in &self.findings {
            writeln!(f, "{finding}")?;
        }
        let verdict = match self.result.as_deref() {
            Some(r) if r.eq_ignore_ascii_case("SUCCESS") => "ACCEPTABLE".to_string(),
            Some(r) if self.is_acceptable() => format!("ACCEPTABLE ({r})"),
            Some(r) => format!("NOT ACCEPTABLE ({r})"),
            None if self.error_count() == 0 => "ACCEPTABLE".to_string(),
            None => "NOT ACCEPTABLE".to_string(),
        };
        write!(
            f,
            "WebCheck {verdict}: {} error(s), {} warning(s)",
            self.errors_reported.unwrap_or_else(|| self.error_count()),
            self.warnings_reported
                .unwrap_or_else(|| self.warning_count()),
        )?;
        if let Some(t) = &self.filing_type {
            write!(f, ", filing type {t}")?;
        }
        if self.counts_disagree() {
            write!(
                f,
                " [parsed {} error(s), {} warning(s) -- response format may have changed]",
                self.error_count(),
                self.warning_count()
            )?;
        }
        writeln!(f)
    }
}

// ---------------------------------------------------------------------------
// Response parsing: the upload channel's HTML fragment
// ---------------------------------------------------------------------------

static VALIDATION_OBJ: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r#"validationObj\s*=\s*'([^']*)'"#).ok());
static ERRORS_COUNT: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"errorsCount\s*=\s*'(\d*)'").ok());
static WARNINGS_COUNT: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"warningsCount\s*=\s*'(\d*)'").ok());
static LARGE_FILE: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"largeFileUploadResponse\s*=\s*'([^']*)'").ok());
static TD: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?is)<td[^>]*>(.*?)</td>").ok());
static TAG: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"(?s)<[^>]*>").ok());
static ITEM_NO: LazyLock<Option<Regex>> = LazyLock::new(|| Regex::new(r"^\d+\.$").ok());
static FORM_ITEM: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^Form\{Item\}:\s*(\S+)\s*(?:\{(.*)\})?\s*$").ok());
static FIELD_NAME: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"^Field Name:\s*#(\d+)\s*(.*)$").ok());
static COVER_ROW: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?is)<td[^>]*>\s*(Committee ID|Committee Name|Filing Type):\s*</td>\s*<td[^>]*>(.*?)</td>")
        .ok()
});
static LINE_PREFIX: LazyLock<Option<Regex>> = LazyLock::new(|| {
    Regex::new(r"(?i)^(?:line|ln|row|record|rec)\s*#?\s*(\d+)\b\s*[:\-]?\s*").ok()
});
static SEVERITY_PREFIX: LazyLock<Option<Regex>> =
    LazyLock::new(|| Regex::new(r"(?i)^(error|failing|fail|warning|warn)\s*[:\-]\s*").ok());

/// Decodes the HTML entities WebCheck emits (`&nbsp;`, `&#039;`, `&amp;`,
/// `&lt;`, `&gt;`, `&quot;`, decimal and hex numeric references).
fn html_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        let (before, after) = rest.split_at(pos);
        out.push_str(before);
        let Some(end) = after.find(';') else {
            out.push_str(after);
            return out;
        };
        // `after` starts with the `&` just found and `;` is ASCII, so
        // `1..end` and `end + 1..` are in bounds and on char boundaries.
        let entity = after.get(1..end).unwrap_or("");
        let decoded = match entity {
            "nbsp" => Some(' '),
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix('#')
                .and_then(|n| match n.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => n.parse().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(c) if end < 12 => {
                out.push(c);
                rest = after.get(end + 1..).unwrap_or("");
            }
            _ => {
                out.push('&');
                rest = after.get(1..).unwrap_or("");
            }
        }
    }
    out.push_str(rest);
    out
}

/// Strips tags, decodes entities, collapses whitespace.
fn cell_text(html: &str) -> String {
    let no_tags = match TAG.as_ref() {
        Some(re) => re.replace_all(html, " "),
        None => std::borrow::Cow::Borrowed(html),
    };
    html_unescape(&no_tags)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn capture<'a>(re: &'a Option<Regex>, text: &'a str) -> Option<&'a str> {
    re.as_ref()?.captures(text)?.get(1).map(|m| m.as_str())
}

/// The slice of `html` between `id="<section>"` and the next
/// `id="<next>"` (or the end).
fn section<'a>(html: &'a str, section: &str, next: Option<&str>) -> Option<&'a str> {
    let marker = format!("id=\"{section}\"");
    let start = html.find(&marker)?.saturating_add(marker.len());
    let rest = html.get(start..)?;
    let end = next
        .and_then(|n| rest.find(&format!("id=\"{n}\"")))
        .unwrap_or(rest.len());
    rest.get(..end)
}

/// Groups the text cells of one section into findings.
///
/// A WebCheck section is a table whose cells run `1.`, `Form{Item}: ...`,
/// `Field Name: #NNN ...`, `<message>`, `2.`, ... Cells before the first
/// number, or of an unknown shape, are kept as message text rather than
/// dropped.
fn parse_section(html: &str, severity: Severity) -> Vec<OracleFinding> {
    let Some(td) = TD.as_ref() else {
        return Vec::new();
    };
    let cells: Vec<String> = td
        .captures_iter(html)
        .filter_map(|c| c.get(1).map(|m| cell_text(m.as_str())))
        .filter(|s| !s.is_empty())
        .collect();

    let mut findings = Vec::new();
    let mut current: Option<OracleFinding> = None;
    for cell in cells {
        if ITEM_NO.as_ref().is_some_and(|re| re.is_match(&cell)) {
            if let Some(f) = current.take() {
                findings.push(f);
            }
            let mut f = OracleFinding::from_message("");
            f.severity = Some(severity);
            current = Some(f);
            continue;
        }
        let Some(f) = current.as_mut() else {
            // Text before the first numbered item: the section's banner
            // ("Validation Errors must be corrected ...") -- not a finding.
            continue;
        };
        if let Some(caps) = FORM_ITEM.as_ref().and_then(|re| re.captures(&cell)) {
            f.form_type = caps.get(1).map(|m| m.as_str().to_string());
            f.item = caps
                .get(2)
                .map(|m| m.as_str().trim().to_string())
                .filter(|s| !s.is_empty());
        } else if let Some(caps) = FIELD_NAME.as_ref().and_then(|re| re.captures(&cell)) {
            f.field_no = caps.get(1).and_then(|m| m.as_str().parse().ok());
            f.field_label = caps
                .get(2)
                .map(|m| m.as_str().trim().to_string())
                .filter(|s| !s.is_empty());
        } else if f.message.is_empty() {
            f.message = cell;
        } else {
            f.message.push(' ');
            f.message.push_str(&cell);
        }
    }
    if let Some(f) = current.take() {
        findings.push(f);
    }
    findings
}

/// Parses the HTML fragment the upload channel returns.
///
/// Recognises the `validationObj` verdict, `errorsCount`/`warningsCount`,
/// the cover summary table, and the `Errors`/`Warnings` sections. A body
/// with a non-empty `largeFileUploadResponse` is [`WebCheckError::Deferred`];
/// a body with none of the markers is [`WebCheckError::Unparseable`]; a
/// JSON body (the page's own error path, `{"result": "..."}` with a
/// non-2xx status) is [`WebCheckError::Rejected`].
pub fn parse_upload_response(html: &str) -> Result<OracleReport, WebCheckError> {
    if let Some(msg) = capture(&LARGE_FILE, html).filter(|m| !m.trim().is_empty()) {
        return Err(WebCheckError::Deferred {
            message: cell_text(msg),
        });
    }
    let validation_obj = capture(&VALIDATION_OBJ, html);
    let errors_reported = capture(&ERRORS_COUNT, html).and_then(|n| n.parse().ok());
    let warnings_reported = capture(&WARNINGS_COUNT, html).and_then(|n| n.parse().ok());
    let has_sections = html.contains("id=\"errors\"") || html.contains("id=\"warnings\"");
    if validation_obj.is_none() && errors_reported.is_none() && !has_sections {
        let trimmed = html.trim();
        if trimmed.starts_with('{') {
            let result = json_string_field(trimmed, "result")
                .or_else(|| json_string_field(trimmed, "message"))
                .unwrap_or_else(|| snippet(trimmed));
            return Err(WebCheckError::Rejected { message: result });
        }
        return Err(WebCheckError::Unparseable {
            snippet: snippet(html),
        });
    }

    let result = validation_obj.and_then(|j| json_string_field(j, "result"));
    let mut filing_type = None;
    let mut committee_id = None;
    if let Some(re) = COVER_ROW.as_ref() {
        for caps in re.captures_iter(html) {
            let value = caps.get(2).map(|m| cell_text(m.as_str()));
            match caps.get(1).map(|m| m.as_str()) {
                Some("Filing Type") => filing_type = value,
                Some("Committee ID") => committee_id = value,
                _ => {}
            }
        }
    }

    let mut findings = Vec::new();
    if let Some(errors) = section(html, "errors", Some("warnings")) {
        findings.extend(parse_section(errors, Severity::Error));
    }
    if let Some(warnings) = section(html, "warnings", None) {
        findings.extend(parse_section(warnings, Severity::Warning));
    }

    Ok(OracleReport {
        raw: html.to_string(),
        result,
        errors_reported,
        warnings_reported,
        filing_type,
        committee_id,
        findings,
    })
}

/// Parses a report that is either the upload channel's HTML fragment or
/// plain text, one message per line.
///
/// Plain text is what the SOAP channel's `msg_url` is expected to serve
/// (the web page calls it "your validation results as a Text file");
/// this module has not seen one, so each line is kept whole as the
/// message, with a leading `line N` / `ERROR:` / `WARNING:` prefix
/// recognised when present. Blank lines are skipped; an empty body is
/// [`WebCheckError::Unparseable`].
pub fn parse_report_text(text: &str) -> Result<OracleReport, WebCheckError> {
    if text.contains("validationObj") || text.contains("id=\"errors\"") {
        return parse_upload_response(text);
    }
    if text.trim().is_empty() {
        return Err(WebCheckError::Unparseable {
            snippet: String::new(),
        });
    }
    let findings = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(parse_text_line)
        .collect();
    Ok(OracleReport {
        raw: text.to_string(),
        findings,
        ..OracleReport::default()
    })
}

fn parse_text_line(line: &str) -> OracleFinding {
    let mut f = OracleFinding::from_message("");
    let mut rest = line;
    if let Some(caps) = SEVERITY_PREFIX.as_ref().and_then(|re| re.captures(rest)) {
        f.severity = caps.get(1).map(|m| {
            if m.as_str().to_ascii_lowercase().starts_with("warn") {
                Severity::Warning
            } else {
                Severity::Error
            }
        });
        if let Some(m) = caps.get(0) {
            rest = rest.get(m.end()..).unwrap_or("");
        }
    }
    if let Some(caps) = LINE_PREFIX.as_ref().and_then(|re| re.captures(rest)) {
        f.line_no = caps.get(1).and_then(|m| m.as_str().parse().ok());
        if let Some(m) = caps.get(0) {
            rest = rest.get(m.end()..).unwrap_or("");
        }
    }
    f.message = rest.trim().to_string();
    f
}

// ---------------------------------------------------------------------------
// Response parsing: the SOAP channel
// ---------------------------------------------------------------------------

/// The JSON object the SOAP `validate`/`status` operations return inside
/// `<return>`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct SoapReturn {
    /// `"FAILED"`, `"PROCESSING"`, `"ACCEPTED"`, ... (`status`).
    pub status: String,
    /// Human-readable explanation (`message`).
    pub message: String,
    /// Where the results can be fetched, when given (`msg_url`).
    pub msg_url: String,
    /// The e-mail the service will notify, when given (`email`).
    pub email: String,
    /// Identifiers for the `status` operation.
    pub submission_id: String,
    pub batch_id: i64,
    /// `success`.
    pub success: bool,
    /// The JSON text as received.
    pub raw: String,
}

/// Unwraps a SOAP response -- MTOM `multipart/related` or bare XML --
/// into the `<return>` JSON, or a [`WebCheckError::SoapFault`].
///
/// Fails with [`WebCheckError::Unparseable`] if there is neither a
/// `<return>` element nor a fault.
pub fn parse_soap_response(body: &str) -> Result<SoapReturn, WebCheckError> {
    if let Some(fault) = element_text(body, "Fault") {
        let code = element_text(&fault, "faultcode").unwrap_or_default();
        let string = element_text(&fault, "faultstring").unwrap_or_default();
        return Err(WebCheckError::SoapFault { code, string });
    }
    let Some(ret) = element_text(body, "return") else {
        return Err(WebCheckError::Unparseable {
            snippet: snippet(body),
        });
    };
    let json = html_unescape(&ret);
    let json = json.trim();
    if !json.starts_with('{') {
        return Err(WebCheckError::Unparseable {
            snippet: snippet(json),
        });
    }
    Ok(SoapReturn {
        status: json_string_field(json, "status").unwrap_or_default(),
        message: json_string_field(json, "message").unwrap_or_default(),
        msg_url: json_string_field(json, "msg_url").unwrap_or_default(),
        email: json_string_field(json, "email").unwrap_or_default(),
        submission_id: json_string_field(json, "submission_id").unwrap_or_default(),
        batch_id: json_scalar_field(json, "batch_id")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0),
        success: json_scalar_field(json, "success").as_deref() == Some("true"),
        raw: json.to_string(),
    })
}

/// The text content of the first `<name>` or `<prefix:name>` element in
/// `xml`, tags of any nested elements included verbatim.
///
/// Every slice below starts or ends at an index `find` returned for an
/// ASCII needle on the same string (or one past it), so each is in bounds
/// and on a character boundary; `get` is used regardless so a mistake
/// would surface as `None`, never a panic.
fn element_text(xml: &str, name: &str) -> Option<String> {
    let mut search = 0;
    while let Some(rel) = xml.get(search..)?.find('<') {
        let open = search.saturating_add(rel);
        let after = xml.get(open.saturating_add(1)..)?;
        let tag_end = after.find(['>', ' ', '/'])?;
        let tag = after.get(..tag_end)?;
        let local = tag.rsplit(':').next().unwrap_or(tag);
        if local == name && !tag.starts_with('/') && !tag.starts_with('?') && !tag.starts_with('!')
        {
            let content_start = after.find('>')?.saturating_add(1);
            let close_a = format!("</{tag}>");
            let close_b = format!("</{name}>");
            let rest = after.get(content_start..)?;
            let end = rest
                .find(&close_a)
                .into_iter()
                .chain(rest.find(&close_b))
                .min()?;
            return rest.get(..end).map(str::to_string);
        }
        search = open.saturating_add(1);
    }
    None
}

/// The value of `"key": "..."` in a flat JSON object, unescaped. `None`
/// if absent or not a string.
fn json_string_field(json: &str, key: &str) -> Option<String> {
    let value = json_raw_value(json, key)?;
    let inner = value.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Some(ch) = u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32) {
                        out.push(ch);
                    }
                }
                other => out.push(other),
            },
            _ => out.push(c),
        }
    }
    None
}

/// The raw text of a non-string scalar (`true`, `0`, `null`) for `key`.
fn json_scalar_field(json: &str, key: &str) -> Option<String> {
    let value = json_raw_value(json, key)?;
    if value.starts_with('"') {
        return None;
    }
    let end = value
        .find([',', '}', ' ', '\n', '\r', '\t'])
        .unwrap_or(value.len());
    value.get(..end).map(str::to_string)
}

/// The text of `json` from the start of `key`'s value.
fn json_raw_value<'a>(json: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("\"{key}\"");
    let mut from = 0;
    while let Some(rel) = json.get(from..)?.find(&needle) {
        let after_key = from.saturating_add(rel).saturating_add(needle.len());
        let rest = json.get(after_key..)?.trim_start();
        if let Some(value) = rest.strip_prefix(':') {
            return Some(value.trim_start());
        }
        from = after_key;
    }
    None
}

// ---------------------------------------------------------------------------
// Diff
// ---------------------------------------------------------------------------

/// Our findings against WebCheck's, as a multiset match on `(record type,
/// field number, message template)`. See the [module docs](self).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[must_use]
#[non_exhaustive]
pub struct OracleDiff {
    /// Pairs both validators reported.
    pub matched: Vec<(Finding, OracleFinding)>,
    /// Ours that WebCheck did not report.
    pub only_ours: Vec<Finding>,
    /// WebCheck's that we did not report.
    pub only_theirs: Vec<OracleFinding>,
}

impl OracleDiff {
    /// True when neither side reported anything the other did not.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.only_ours.is_empty() && self.only_theirs.is_empty()
    }

    /// Matched pairs where the two validators disagree on severity (for
    /// instance our deliberate demotion of `required_field_empty` on a
    /// superseded format).
    pub fn severity_disagreements(&self) -> impl Iterator<Item = &(Finding, OracleFinding)> + '_ {
        self.matched
            .iter()
            .filter(|(ours, theirs)| theirs.severity.is_some_and(|s| s != ours.severity))
    }
}

impl fmt::Display for OracleDiff {
    /// Three sections then a one-line summary
    /// (`N matched, M only ours, K only theirs`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Matched ({}):", self.matched.len())?;
        for (ours, theirs) in &self.matched {
            writeln!(f, "  {ours}")?;
            let severity_note = if theirs.severity.is_some_and(|s| s != ours.severity) {
                "  [severity differs]"
            } else {
                ""
            };
            writeln!(f, "    ~ {theirs}{severity_note}")?;
        }
        writeln!(f, "Only ours ({}):", self.only_ours.len())?;
        for ours in &self.only_ours {
            writeln!(f, "  {ours}")?;
        }
        writeln!(f, "Only theirs ({}):", self.only_theirs.len())?;
        for theirs in &self.only_theirs {
            writeln!(f, "  {theirs}")?;
        }
        writeln!(
            f,
            "{} matched, {} only ours, {} only theirs",
            self.matched.len(),
            self.only_ours.len(),
            self.only_theirs.len()
        )
    }
}

/// Wordings the live WebCheck uses where the FEC's published list says
/// something else, observed against the live service on 2026-09-15.
///
/// Each is checked in addition to [`Rule::fec_message`] by [`diff`].
/// Blanks (`____`) and `{...}` placeholders match any text.
#[must_use]
pub fn live_alternates(rule: Rule) -> &'static [&'static str] {
    match rule {
        // tests/fixtures/F3XN_210000_v5.3.fec: HDR #003 FEC Version#.
        Rule::CurrentFormat => &["Filing Format must be Version ____"],
        // tests/fixtures/invalid/dangling_back_reference.fec.
        Rule::BackReferenceNotFound => {
            &["No Match Found for Back-Reference to Schedule/TranID - ____"]
        }
        // tests/fixtures/invalid/bad_dates_and_amounts.fec:
        // "$5,500.00 not a Valid Amount of Expenditure value".
        Rule::InvalidAmount => &["____ not a Valid {field} value"],
        // tests/fixtures/F3A_2004471.fec: a blank payee state -- an
        // `X (warning)` column in the workbook -- comes back at warning
        // severity but worded like the failing message #1-#4.
        Rule::RecommendedFieldEmpty => &["is Required, but field is Empty"],
        // tests/fixtures/invalid/bad_filer_id.fec (an eight-character
        // committee id on an F3XA cover): the same bytes get the published
        // wording on some submissions and this Form 5 message on others --
        // the service is not deterministic here.
        Rule::FilerIdFormat => &["An FEC 'C9xxxxxxx' ID must be used to file Form 5"],
        _ => &[],
    }
}

/// Lower-cases, drops `{...}` placeholders and `_` blanks, turns every
/// other non-alphanumeric character into a space, and drops all-digit
/// words (lengths, dates, amounts, the year range) -- leaving the words
/// that identify a message.
#[must_use]
pub fn template_words(s: &str) -> Vec<String> {
    let mut cleaned = String::with_capacity(s.len());
    let mut depth = 0usize;
    for ch in s.chars() {
        match ch {
            '{' => depth += 1,
            '}' => depth = depth.saturating_sub(1),
            _ if depth > 0 => {}
            c if c.is_alphanumeric() => cleaned.extend(c.to_lowercase()),
            _ => cleaned.push(' '),
        }
    }
    cleaned
        .split_whitespace()
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .map(str::to_string)
        .collect()
}

/// True when every word of `template` appears, in order, among the words
/// of `message` (a blank in the template may stand for any run of words).
fn template_matches(template: &str, message_words: &[String]) -> bool {
    let wanted = template_words(template);
    if wanted.is_empty() {
        return false;
    }
    let mut it = message_words.iter();
    wanted.iter().all(|w| it.any(|m| m == w))
}

/// True when WebCheck's `message` is an instance of `rule`'s message --
/// the published template or a [`live_alternates`] wording.
#[must_use]
pub fn message_matches_rule(rule: Rule, message: &str) -> bool {
    let words = template_words(message);
    template_matches(rule.fec_message(), &words)
        || live_alternates(rule)
            .iter()
            .any(|alt| template_matches(alt, &words))
}

/// The 1-based field number WebCheck would print for our finding's
/// field: its column in the bundled spec plus one. `None` for a finding
/// with no field, an unknown record type, or a field the current spec
/// does not have.
#[must_use]
pub fn field_number(finding: &Finding) -> Option<u16> {
    let field = finding.field?;
    let table = if finding.form_type.eq_ignore_ascii_case("HDR") {
        Table::Hdr
    } else {
        table_for_form_type(&finding.form_type)?
    };
    table.spec(field).map(|s| s.column.saturating_add(1))
}

/// Pairs our findings with WebCheck's.
///
/// Two findings match when they name the same record type (compared
/// case-insensitively), the same field number ([`field_number`] for ours,
/// `field_no` for theirs, `None` matching `None`), and WebCheck's message
/// is an instance of our rule's template ([`message_matches_rule`]). Each
/// finding is used at most once; order is preserved on both sides.
pub fn diff(ours: &Validation, theirs: &OracleReport) -> OracleDiff {
    // Index theirs by (form type, field number) so the per-finding search
    // is over a handful of candidates, not the whole report.
    let mut by_key: HashMap<(String, Option<u16>), Vec<usize>> = HashMap::new();
    for (i, t) in theirs.findings.iter().enumerate() {
        let form = t.form_type.as_deref().unwrap_or("").to_ascii_uppercase();
        by_key.entry((form, t.field_no)).or_default().push(i);
    }
    let mut used = vec![false; theirs.findings.len()];
    let mut matched = Vec::new();
    let mut only_ours = Vec::new();

    for f in &ours.findings {
        let key = (f.form_type.to_ascii_uppercase(), field_number(f));
        let hit = by_key.get(&key).and_then(|candidates| {
            candidates.iter().copied().find(|&i| {
                !used.get(i).copied().unwrap_or(true)
                    && theirs
                        .findings
                        .get(i)
                        .is_some_and(|t| message_matches_rule(f.rule, &t.message))
            })
        });
        match hit.and_then(|i| theirs.findings.get(i).map(|t| (i, t))) {
            Some((i, t)) => {
                if let Some(slot) = used.get_mut(i) {
                    *slot = true;
                }
                matched.push((f.clone(), t.clone()));
            }
            None => only_ours.push(f.clone()),
        }
    }

    let only_theirs = theirs
        .findings
        .iter()
        .zip(&used)
        .filter(|(_, used)| !**used)
        .map(|(t, _)| t.clone())
        .collect();

    OracleDiff {
        matched,
        only_ours,
        only_theirs,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_rfc4648_vectors() {
        for (input, expected) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64_encode(input.as_bytes()), expected, "{input:?}");
        }
        assert_eq!(base64_encode(&[0xff, 0xfe, 0xfd]), "//79");
    }

    #[test]
    fn html_unescape_handles_named_numeric_and_broken_entities() {
        assert_eq!(
            html_unescape("Treasurer&#039;s &amp; &nbsp;Co &lt;x&gt; &#x41; &bogus; & done"),
            "Treasurer's &  Co <x> A &bogus; & done"
        );
    }

    #[test]
    fn template_words_drop_blanks_placeholders_and_numbers() {
        assert_eq!(
            template_words("{field} exceeds maximum length of ______"),
            ["exceeds", "maximum", "length", "of"]
        );
        assert_eq!(
            template_words("Tran ID 'SB21B.4120' is NOT UNIQUE - This one is same as other(s)"),
            [
                "tran", "id", "sb21b", "is", "not", "unique", "this", "one", "is", "same", "as",
                "other", "s"
            ]
        );
    }

    #[test]
    fn message_matching_follows_published_and_live_wordings() {
        assert!(message_matches_rule(
            Rule::DuplicateTransactionId,
            "Tran ID 'SB21B.4120' is NOT UNIQUE - This one is same as other(s)"
        ));
        assert!(message_matches_rule(
            Rule::IllegalCharacter,
            "Illegal character(s) [at position:9 char:127] found in text field - ElectionCFO"
        ));
        assert!(message_matches_rule(
            Rule::RequiredFieldEmpty,
            "is Required, but field is Empty"
        ));
        assert!(message_matches_rule(
            Rule::CurrentFormat,
            "Filing Format must be Version 8.5"
        ));
        assert!(message_matches_rule(
            Rule::InvalidAmount,
            "$5,500.00 not a Valid Amount of Expenditure value"
        ));
        assert!(message_matches_rule(
            Rule::BackReferenceNotFound,
            "No Match Found for Back-Reference to Schedule/TranID - SA11AI.9999"
        ));
        assert!(!message_matches_rule(
            Rule::NotARealDate,
            "Bad Date - 2026-06-22 not YYYYMMDD format"
        ));
        assert!(!message_matches_rule(Rule::FieldTooLong, ""));
    }

    #[test]
    fn json_fields_are_extracted_tolerantly() {
        let j = r#"{"status":"FAILED","msg_url":"","message":"Error! \"quoted\" \u0041","batch_id":0,"success":false}"#;
        assert_eq!(json_string_field(j, "status").as_deref(), Some("FAILED"));
        assert_eq!(json_string_field(j, "msg_url").as_deref(), Some(""));
        assert_eq!(
            json_string_field(j, "message").as_deref(),
            Some("Error! \"quoted\" A")
        );
        assert_eq!(json_scalar_field(j, "batch_id").as_deref(), Some("0"));
        assert_eq!(json_scalar_field(j, "success").as_deref(), Some("false"));
        assert_eq!(json_string_field(j, "absent"), None);
        assert_eq!(json_string_field(j, "batch_id"), None);
    }

    #[test]
    fn element_text_finds_prefixed_and_bare_tags() {
        let xml = r#"<soap:Envelope xmlns:soap="x"><soap:Body><ns2:validateResponse xmlns:ns2="y"><return>{"a":1}</return></ns2:validateResponse></soap:Body></soap:Envelope>"#;
        assert_eq!(element_text(xml, "return").as_deref(), Some(r#"{"a":1}"#));
        assert_eq!(
            element_text(xml, "Body").as_deref(),
            Some(
                r#"<ns2:validateResponse xmlns:ns2="y"><return>{"a":1}</return></ns2:validateResponse>"#
            )
        );
        assert_eq!(element_text(xml, "nope"), None);
    }

    #[test]
    fn multipart_boundary_never_appears_in_payload() {
        let bytes = b"abc";
        let b = multipart_boundary(bytes);
        let mut payload = b.clone().into_bytes();
        payload.extend_from_slice(b"tail");
        let b2 = multipart_boundary(&payload);
        assert!(!payload.windows(b2.len()).any(|w| w == b2.as_bytes()));
    }

    #[test]
    fn snippet_is_bounded() {
        let long = "x ".repeat(1000);
        let s = snippet(&long);
        assert!(s.chars().count() <= SNIPPET_CHARS + 3);
        assert!(s.ends_with("..."));
    }

    #[test]
    fn credentials_debug_redacts_the_key() {
        let c = Credentials::new("secret").with_email("a@b.c");
        let d = format!("{c:?}");
        assert!(!d.contains("secret"));
        assert!(d.contains("REDACTED"));
    }

    #[test]
    fn text_lines_parse_prefixes() {
        let f = parse_text_line("ERROR: line 12: Something bad");
        assert_eq!(f.severity, Some(Severity::Error));
        assert_eq!(f.line_no, Some(12));
        assert_eq!(f.message, "Something bad");
        let f = parse_text_line("Warning - Ln 3 Something mild");
        assert_eq!(f.severity, Some(Severity::Warning));
        assert_eq!(f.line_no, Some(3));
        assert_eq!(f.message, "Something mild");
        let f = parse_text_line("plain");
        assert_eq!(f.severity, None);
        assert_eq!(f.line_no, None);
        assert_eq!(f.message, "plain");
    }
}
