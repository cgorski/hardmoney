//! A client for the parts of openFEC (<https://api.open.fec.gov/developers/>)
//! that find filings: `/filings/` (processed metadata, with amendment
//! chains and cover-page totals), `/efile/filings/` (raw e-filing
//! metadata, minutes after receipt), and `/operations-log/` (when the
//! FEC finished loading each report's summary and its transactions).
//!
//! [`OpenFec::processing_status`] joins the three for a set of filing ids,
//! which is how `hardmoney lag` measures how far the processed data is
//! behind the raw e-filings.
//!
//! Requires the `serde` feature in addition to `fetch`.
//!
//! Response structs model the fields openFEC documents, every one
//! optional and defaulted, and ignore fields they do not know, so a new
//! column on the FEC's side is not a breaking change here. Money is
//! [`rust_decimal::Decimal`] (openFEC serialises amounts as JSON numbers;
//! they are decoded through their shortest decimal representation, so
//! `25270.16` is exactly `25270.16`). Dates are [`chrono::NaiveDate`] and
//! timestamps [`chrono::NaiveDateTime`]; openFEC gives neither a zone.
//!
//! # Example
//!
//! ```no_run
//! use hardmoney::fec::openfec::{FilingsQuery, OpenFec};
//! use hardmoney::fec::{Cache, fetch_filing_bytes};
//! use hardmoney::{Filing, ParseOptions};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let api = OpenFec::from_env()?; // FEC_API_KEY or ~/fec_api_key.txt
//! let cache = Cache::from_env();
//! let query = FilingsQuery::new()
//!     .committee_id("C00554709")
//!     .form_type("F3")
//!     .most_recent(true);
//! for record in api.filings_all(&query) {
//!     let record = record?;
//!     // Paper filings have no raw .fec (and a negative file_number).
//!     let Some(id) = record.filing_id() else { continue };
//!     let bytes = fetch_filing_bytes(id, &cache, record.raw_url().as_deref())?;
//!     let filing = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)?;
//!     println!("{id}: {} error(s)", filing.validate().error_count());
//! }
//! # Ok(())
//! # }
//! ```

use std::collections::VecDeque;
use std::fmt;
use std::path::PathBuf;
use std::time::Duration;

use chrono::{NaiveDate, NaiveDateTime};
use rust_decimal::Decimal;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use super::{FecApiError, Result, agent, get_ok, redact_url};

/// An openFEC API key. `Debug` and `Display` print `REDACTED`; the only
/// way to see the key is to have supplied it.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    /// Wraps a key. Fails with [`FecApiError::InvalidQuery`] if it is
    /// empty or contains whitespace or control characters (a pasted key
    /// with a stray newline would otherwise fail every request with 403).
    pub fn new(key: impl Into<String>) -> Result<Self> {
        let key: String = key.into();
        if key.is_empty() {
            return Err(FecApiError::InvalidQuery(
                "the API key is empty".to_string(),
            ));
        }
        if key.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(FecApiError::InvalidQuery(
                "the API key contains whitespace or control characters".to_string(),
            ));
        }
        Ok(ApiKey(key))
    }

    /// The key from `FEC_API_KEY`, else from `~/fec_api_key.txt` (first
    /// line, trimmed). Fails with [`FecApiError::MissingApiKey`] naming
    /// both places if neither has one.
    pub fn from_env() -> Result<Self> {
        let mut looked_in = vec!["FEC_API_KEY".to_string()];
        if let Some(v) = std::env::var_os("FEC_API_KEY") {
            let s = v.to_string_lossy();
            let s = s.trim();
            if !s.is_empty() {
                return ApiKey::new(s);
            }
        }
        if let Some(path) = key_file_path() {
            looked_in.push(path.display().to_string());
            if let Ok(contents) = std::fs::read_to_string(&path) {
                let first = contents.lines().next().unwrap_or("").trim();
                if !first.is_empty() {
                    return ApiKey::new(first);
                }
            }
        }
        Err(FecApiError::MissingApiKey { looked_in })
    }

    /// The key itself, for building requests. Deliberately crate-private.
    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

fn key_file_path() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(|h| PathBuf::from(h).join("fec_api_key.txt"))
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(REDACTED)")
    }
}

impl fmt::Display for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("REDACTED")
    }
}

/// The openFEC client.
///
/// Holds a pooled HTTP agent, the key, and a retry policy for HTTP 429.
/// `Debug` shows the base URL and policy, never the key.
#[derive(Clone)]
pub struct OpenFec {
    client: ureq::Agent,
    key: ApiKey,
    base: String,
    max_retries: u32,
    max_wait: Duration,
}

impl fmt::Debug for OpenFec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OpenFec")
            .field("base", &self.base)
            .field("key", &self.key)
            .field("max_retries", &self.max_retries)
            .field("max_wait", &self.max_wait)
            .finish()
    }
}

impl OpenFec {
    /// The production API root.
    pub const BASE_URL: &'static str = "https://api.open.fec.gov/v1/";

    /// Default retry budget on 429: three retries ...
    pub const DEFAULT_MAX_RETRIES: u32 = 3;
    /// ... each sleeping at most this long.
    pub const DEFAULT_MAX_WAIT: Duration = Duration::from_secs(60);

    /// A client for [`Self::BASE_URL`] with `key` and the default retry
    /// policy.
    #[must_use]
    pub fn new(key: ApiKey) -> Self {
        OpenFec {
            client: agent().clone(),
            key,
            base: Self::BASE_URL.to_string(),
            max_retries: Self::DEFAULT_MAX_RETRIES,
            max_wait: Self::DEFAULT_MAX_WAIT,
        }
    }

    /// [`Self::new`] with the key from [`ApiKey::from_env`].
    pub fn from_env() -> Result<Self> {
        Ok(Self::new(ApiKey::from_env()?))
    }

    /// Points the client at another API root (a proxy, a recording
    /// server). A trailing slash is added if missing.
    #[must_use]
    pub fn with_base_url(mut self, base: impl Into<String>) -> Self {
        let mut base: String = base.into();
        if !base.ends_with('/') {
            base.push('/');
        }
        self.base = base;
        self
    }

    /// Sets how many times a 429 is retried and the longest single sleep
    /// the client will accept from `Retry-After` (or its own back-off).
    /// A `Retry-After` beyond `max_wait` is not slept on: the call returns
    /// [`FecApiError::RateLimited`] at once with the server's figure.
    #[must_use]
    pub fn retry_policy(mut self, max_retries: u32, max_wait: Duration) -> Self {
        self.max_retries = max_retries;
        self.max_wait = max_wait;
        self
    }

    /// The API root this client talks to.
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base
    }

    /// The URL a request for `path` with `query` (a query string without
    /// the key, as [`FilingsQuery::query_string`] produces) would use,
    /// with the key shown as `api_key=REDACTED`. For logs and tests; the
    /// key goes first in the query string.
    #[must_use]
    pub fn url(&self, path: &str, query: &str) -> String {
        redact_url(&self.url_with_key(path, query))
    }

    fn url_with_key(&self, path: &str, query: &str) -> String {
        let mut url = format!(
            "{}{}?api_key={}",
            self.base,
            path.trim_start_matches('/'),
            percent_encode(self.key.expose())
        );
        if !query.is_empty() {
            url.push('&');
            url.push_str(query);
        }
        url
    }

    /// One page of `/filings/`.
    pub fn filings(&self, query: &FilingsQuery) -> Result<Page<FilingRecord>> {
        self.get_json("filings/", &query.query_string()?)
    }

    /// Every `/filings/` result matching `query`, across pages, in the
    /// API's order. Pagination starts from `query`'s page (1 by default)
    /// and stops at the last page the API reports; an error ends the
    /// iteration after yielding it.
    #[must_use]
    pub fn filings_all(&self, query: &FilingsQuery) -> FilingsIter<'_> {
        FilingsIter {
            client: self,
            query: query.clone(),
            buffered: VecDeque::new(),
            done: false,
        }
    }

    /// One page of `/efile/filings/`.
    pub fn efile_filings(&self, query: &EfileQuery) -> Result<Page<EfileRecord>> {
        self.get_json("efile/filings/", &query.query_string()?)
    }

    /// One page of `/operations-log/`: the FEC's record of when each
    /// report's summary and transactions finished loading.
    pub fn operations_log(&self, query: &OperationsLogQuery) -> Result<Page<OperationsLogRecord>> {
        self.get_json("operations-log/", &query.query_string()?)
    }

    /// Every `/operations-log/` row matching `query`, across pages.
    pub fn operations_log_all(
        &self,
        query: &OperationsLogQuery,
    ) -> Result<Vec<OperationsLogRecord>> {
        let mut query = query.clone();
        let mut out = Vec::new();
        loop {
            let page = self.operations_log(&query)?;
            let more = page.has_more() && !page.results.is_empty();
            out.extend(page.results);
            if !more {
                return Ok(out);
            }
            query.page = query.page.saturating_add(1);
        }
    }

    /// The raw-filing URL openFEC publishes for `filing_id`: the `fec_url`
    /// from `/efile/filings/` (available minutes after receipt), else from
    /// `/filings/` (once processed). `Ok(None)` when neither endpoint knows
    /// the filing or neither record carries a URL; an error only when a
    /// request fails.
    ///
    /// This is the replacement for building the `docquery.fec.gov` URL by
    /// hand ([`docquery_url`](super::docquery_url)), which the FEC has said
    /// it is inventorying for retirement (`openFEC#6717`).
    pub fn resolve_fec_url(&self, filing_id: u64) -> Result<Option<String>> {
        let efile = self.efile_filings(&EfileQuery::new().file_number(filing_id).per_page(1))?;
        if let Some(url) = efile.results.into_iter().find_map(|r| r.fec_url) {
            return Ok(Some(url));
        }
        let processed = self.filings(&FilingsQuery::new().file_number(filing_id).per_page(1))?;
        Ok(processed.results.into_iter().find_map(|r| r.fec_url))
    }

    /// Where the FEC's processing stands for each of `filing_ids`, joining
    /// `/efile/filings/` (receipt), `/filings/` (processed metadata), and
    /// `/operations-log/` (completion dates) on the filing number and its
    /// beginning image number. Ids are looked up in batches of
    /// [`PROCESSING_BATCH`], so `n` ids cost about `3 * ceil(n / 50)`
    /// requests. Ids unknown to every endpoint come back with every field
    /// `None` and `in_filings == false`. The result is in the order given.
    pub fn processing_status(&self, filing_ids: &[u64]) -> Result<Vec<ProcessingStatus>> {
        let mut out: Vec<ProcessingStatus> = filing_ids
            .iter()
            .map(|&id| ProcessingStatus::unknown(id))
            .collect();
        for batch in filing_ids.chunks(PROCESSING_BATCH) {
            let efile = self.efile_filings(
                &EfileQuery::new()
                    .file_numbers(batch.iter().copied())
                    .per_page(100),
            )?;
            for r in efile.results {
                let Some(id) = r.filing_id() else { continue };
                for s in out.iter_mut().filter(|s| s.filing_id == id) {
                    s.committee_id = r.committee_id.clone();
                    s.form_type = r.form_type.clone();
                    s.received = r.receipt_date.or(s.received);
                    s.fec_url = r.fec_url.clone().or(s.fec_url.take());
                    s.beginning_image_number = r.beginning_image_number.clone();
                    s.ending_image_number = r.ending_image_number.clone();
                    s.in_efile = true;
                }
            }
            let processed = self.filings(
                &FilingsQuery::new()
                    .file_numbers(batch.iter().copied())
                    .sort(None::<String>)
                    .per_page(100),
            )?;
            for r in processed.results {
                let Some(id) = r.filing_id() else { continue };
                for s in out.iter_mut().filter(|s| s.filing_id == id) {
                    s.in_filings = true;
                    s.committee_id = s.committee_id.take().or(r.committee_id.clone());
                    s.form_type = s.form_type.take().or(r.form_type.clone());
                    s.report_type = r.report_type.clone();
                    s.received = s.received.or(r.receipt_date);
                    s.fec_url = s.fec_url.take().or(r.fec_url.clone());
                    s.sub_id = r.sub_id.clone();
                    s.cycle = r.cycle;
                    s.beginning_image_number = s
                        .beginning_image_number
                        .take()
                        .or(r.beginning_image_number.clone());
                    s.ending_image_number = s
                        .ending_image_number
                        .take()
                        .or(r.ending_image_number.clone());
                }
            }
            let images: Vec<String> = out
                .iter()
                .filter(|s| batch.contains(&s.filing_id))
                .filter_map(|s| s.beginning_image_number.clone())
                .collect();
            if images.is_empty() {
                continue;
            }
            let log = self.operations_log(
                &OperationsLogQuery::new()
                    .beginning_image_numbers(images)
                    .per_page(100),
            )?;
            join_operations_log(&mut out, log.results);
        }
        Ok(out)
    }

    /// Processing status for every e-filed report `committee_id` filed in
    /// `cycle` (every version, not only the most recent; `form_type`
    /// narrows to one base form). Joins `/filings/` with `/operations-log/`
    /// for the cycle's two report years, so a committee costs one
    /// `/filings/` page per 100 filings plus one `/operations-log/` page per
    /// 100 rows, whatever its size. Receipt times come from `/filings/`,
    /// which records dates, not timestamps. Newest receipt first.
    pub fn committee_processing_status(
        &self,
        committee_id: &str,
        cycle: u16,
        form_type: Option<&str>,
    ) -> Result<Vec<ProcessingStatus>> {
        let mut query = FilingsQuery::new().committee_id(committee_id).cycle(cycle);
        if let Some(form) = form_type {
            query = query.form_type(form);
        }
        let mut out: Vec<ProcessingStatus> = Vec::new();
        for record in self.filings_all(&query) {
            let record = record?;
            let Some(id) = record.filing_id() else {
                continue;
            };
            let mut s = ProcessingStatus::unknown(id);
            s.in_filings = true;
            s.committee_id = record.committee_id.clone();
            s.form_type = record.form_type.clone();
            s.report_type = record.report_type.clone();
            s.received = record.receipt_date;
            s.fec_url = record.fec_url.clone();
            s.sub_id = record.sub_id.clone();
            s.cycle = record.cycle;
            s.beginning_image_number = record.beginning_image_number.clone();
            s.ending_image_number = record.ending_image_number.clone();
            out.push(s);
        }
        for year in [cycle.saturating_sub(1), cycle] {
            let mut log = OperationsLogQuery::new()
                .committee_id(committee_id)
                .report_year(year);
            if let Some(form) = form_type {
                log = log.form_type(form);
            }
            join_operations_log(&mut out, self.operations_log_all(&log)?);
        }
        Ok(out)
    }

    /// How many processed rows a schedule endpoint holds for one filing,
    /// found by the filing's image-number range (every itemized row
    /// carries the image number of the page it was filed on). One request
    /// with `per_page=1`; the answer is the endpoint's `pagination`, whose
    /// `count` may be approximate when `is_count_exact` is `Some(false)`.
    /// `cycle` is the filing's two-year period; omit it and openFEC
    /// defaults to the current one.
    pub fn processed_rows(
        &self,
        schedule: ScheduleEndpoint,
        committee_id: &str,
        image_range: (&str, &str),
        cycle: Option<u16>,
    ) -> Result<Pagination> {
        if let Some(c) = cycle {
            check_cycle(c)?;
        }
        let mut p: Vec<(&str, String)> = vec![
            ("committee_id", committee_id.to_string()),
            ("min_image_number", image_range.0.to_string()),
            ("max_image_number", image_range.1.to_string()),
        ];
        if let Some(c) = cycle {
            p.push((schedule.cycle_param(), c.to_string()));
        }
        p.push(("per_page", "1".to_string()));
        let page: Page<serde_json::Value> = self.get_json(schedule.path(), &encode_params(&p))?;
        Ok(page.pagination)
    }

    /// GET + decode, with the 429 retry loop.
    fn get_json<T: DeserializeOwned>(&self, path: &str, query: &str) -> Result<T> {
        let url = self.url_with_key(path, query);
        let mut attempt = 0u32;
        let response = loop {
            match get_ok(&self.client, &url) {
                Ok(r) => break r,
                Err(FecApiError::RateLimited { retry_after }) if attempt < self.max_retries => {
                    let wait = retry_after.unwrap_or_else(|| backoff(attempt));
                    if wait > self.max_wait {
                        return Err(FecApiError::RateLimited { retry_after });
                    }
                    std::thread::sleep(wait);
                    attempt = attempt.saturating_add(1);
                }
                Err(e) => return Err(e),
            }
        };
        // Pages are at most a few hundred KB; the limit is a safety net
        // against a runaway response, not a budget.
        let text = response
            .into_body()
            .with_config()
            .limit(64 * 1024 * 1024)
            .read_to_string()?;
        serde_json::from_str(&text).map_err(|source| FecApiError::Json {
            url: redact_url(&url),
            source,
        })
    }
}

/// How many filing ids [`OpenFec::processing_status`] looks up per
/// request. openFEC accepts repeated `file_number=` parameters; 50 keeps
/// the URL well under common length limits even with 18-digit image
/// numbers in the `/operations-log/` request.
pub const PROCESSING_BATCH: usize = 50;

/// Copies completion dates from `/operations-log/` rows onto the statuses
/// with the same beginning image number.
fn join_operations_log(out: &mut [ProcessingStatus], rows: Vec<OperationsLogRecord>) {
    for r in rows {
        let Some(image) = r.beginning_image_number.as_deref() else {
            continue;
        };
        for s in out
            .iter_mut()
            .filter(|s| s.beginning_image_number.as_deref() == Some(image))
        {
            s.in_operations_log = true;
            s.summary_data_complete = r.summary_data_complete_date;
            s.transaction_data_complete = r.transaction_data_complete_date;
            s.report_type = s.report_type.take().or(r.report_type.clone());
        }
    }
}

/// The itemized-transaction endpoints [`OpenFec::processed_rows`] counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::Display)]
#[non_exhaustive]
pub enum ScheduleEndpoint {
    /// `/schedules/schedule_a/` (receipts).
    #[strum(serialize = "SchA")]
    A,
    /// `/schedules/schedule_b/` (disbursements).
    #[strum(serialize = "SchB")]
    B,
    /// `/schedules/schedule_e/` (independent expenditures).
    #[strum(serialize = "SchE")]
    E,
}

impl ScheduleEndpoint {
    fn path(self) -> &'static str {
        match self {
            ScheduleEndpoint::A => "schedules/schedule_a/",
            ScheduleEndpoint::B => "schedules/schedule_b/",
            ScheduleEndpoint::E => "schedules/schedule_e/",
        }
    }

    /// Schedules A and B call the two-year period
    /// `two_year_transaction_period`; Schedule E calls it `cycle`.
    fn cycle_param(self) -> &'static str {
        match self {
            ScheduleEndpoint::A | ScheduleEndpoint::B => "two_year_transaction_period",
            ScheduleEndpoint::E => "cycle",
        }
    }
}

/// 1 s, 2 s, 4 s, ... when the server sends no `Retry-After`.
fn backoff(attempt: u32) -> Duration {
    Duration::from_secs(1u64 << attempt.min(6))
}

/// Percent-encodes everything but RFC 3986 unreserved characters.
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(char::from(b));
            }
            _ => {
                out.push('%');
                out.push_str(&format!("{b:02X}"));
            }
        }
    }
    out
}

/// Checks a cycle the way [`crate::Cycle`] does (even, 1976..=2100)
/// without requiring the `bulk` feature.
fn check_cycle(year: u16) -> Result<()> {
    if !(1976..=2100).contains(&year) {
        return Err(FecApiError::InvalidQuery(format!(
            "cycle {year} is outside the FEC's published range (1976-2100)"
        )));
    }
    if !year.is_multiple_of(2) {
        return Err(FecApiError::InvalidQuery(format!(
            "cycle {year} is not an even year (cycles are the even year of a two-year period)"
        )));
    }
    Ok(())
}

fn check_per_page(per_page: u32) -> Result<()> {
    if !(1..=100).contains(&per_page) {
        return Err(FecApiError::InvalidQuery(format!(
            "per_page must be between 1 and 100 (openFEC's maximum), not {per_page}"
        )));
    }
    Ok(())
}

fn check_page(page: u32) -> Result<()> {
    if page == 0 {
        return Err(FecApiError::InvalidQuery(
            "page numbers start at 1".to_string(),
        ));
    }
    Ok(())
}

fn push_param(params: &mut Vec<(&'static str, String)>, key: &'static str, value: Option<String>) {
    if let Some(v) = value {
        params.push((key, v));
    }
}

fn encode_params(params: &[(&str, String)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{k}={}", percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Parameters for `/filings/`. Build with the chainable setters; every
/// filter is optional. Validation (`cycle` even and in range, `per_page`
/// 1..=100, `page` >= 1) happens when the query is sent, as
/// [`FecApiError::InvalidQuery`].
///
/// Defaults: `per_page` 100 (the API's maximum), `page` 1, `sort`
/// `-receipt_date` (newest first).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct FilingsQuery {
    pub committee_id: Option<String>,
    pub candidate_id: Option<String>,
    pub form_type: Option<String>,
    pub report_type: Option<String>,
    pub cycle: Option<u16>,
    pub most_recent: Option<bool>,
    pub min_receipt_date: Option<NaiveDate>,
    pub max_receipt_date: Option<NaiveDate>,
    /// Specific filings (`file_number=` repeated); empty for no filter.
    pub file_numbers: Vec<u64>,
    pub sort: Option<String>,
    pub per_page: u32,
    pub page: u32,
}

impl Default for FilingsQuery {
    fn default() -> Self {
        FilingsQuery {
            committee_id: None,
            candidate_id: None,
            form_type: None,
            report_type: None,
            cycle: None,
            most_recent: None,
            min_receipt_date: None,
            max_receipt_date: None,
            file_numbers: Vec::new(),
            sort: Some("-receipt_date".to_string()),
            per_page: 100,
            page: 1,
        }
    }
}

impl FilingsQuery {
    /// An unfiltered query with the defaults above.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `committee_id=C00554709`.
    #[must_use]
    pub fn committee_id(mut self, id: impl Into<String>) -> Self {
        self.committee_id = Some(id.into());
        self
    }

    /// `candidate_id=H4CA11114`.
    #[must_use]
    pub fn candidate_id(mut self, id: impl Into<String>) -> Self {
        self.candidate_id = Some(id.into());
        self
    }

    /// `form_type=F3X` (the base form type, no amendment suffix).
    #[must_use]
    pub fn form_type(mut self, form_type: impl Into<String>) -> Self {
        self.form_type = Some(form_type.into());
        self
    }

    /// `report_type=Q2`.
    #[must_use]
    pub fn report_type(mut self, report_type: impl Into<String>) -> Self {
        self.report_type = Some(report_type.into());
        self
    }

    /// `cycle=2026`. Checked when sent: even, 1976..=2100.
    #[must_use]
    pub fn cycle(mut self, year: u16) -> Self {
        self.cycle = Some(year);
        self
    }

    /// `most_recent=true` keeps only the latest version of each report.
    #[must_use]
    pub fn most_recent(mut self, most_recent: bool) -> Self {
        self.most_recent = Some(most_recent);
        self
    }

    /// `min_receipt_date=YYYY-MM-DD` (inclusive).
    #[must_use]
    pub fn min_receipt_date(mut self, date: NaiveDate) -> Self {
        self.min_receipt_date = Some(date);
        self
    }

    /// `max_receipt_date=YYYY-MM-DD` (inclusive).
    #[must_use]
    pub fn max_receipt_date(mut self, date: NaiveDate) -> Self {
        self.max_receipt_date = Some(date);
        self
    }

    /// `file_number=2011929`: one specific filing (added to any already
    /// set).
    #[must_use]
    pub fn file_number(mut self, file_number: u64) -> Self {
        self.file_numbers.push(file_number);
        self
    }

    /// Several specific filings, as repeated `file_number=` parameters.
    #[must_use]
    pub fn file_numbers(mut self, file_numbers: impl IntoIterator<Item = u64>) -> Self {
        self.file_numbers.extend(file_numbers);
        self
    }

    /// `sort=` field, `-` prefix for descending; `None` for the API's
    /// default order.
    #[must_use]
    pub fn sort(mut self, sort: Option<impl Into<String>>) -> Self {
        self.sort = sort.map(Into::into);
        self
    }

    /// Results per page, 1..=100.
    #[must_use]
    pub fn per_page(mut self, per_page: u32) -> Self {
        self.per_page = per_page;
        self
    }

    /// 1-based page number.
    #[must_use]
    pub fn page(mut self, page: u32) -> Self {
        self.page = page;
        self
    }

    /// The query string (no leading `?`, no API key) in a fixed
    /// parameter order, percent-encoded. Fails with
    /// [`FecApiError::InvalidQuery`] for an invalid cycle, `per_page`, or
    /// `page`.
    pub fn query_string(&self) -> Result<String> {
        if let Some(c) = self.cycle {
            check_cycle(c)?;
        }
        check_per_page(self.per_page)?;
        check_page(self.page)?;
        let mut p = Vec::new();
        push_param(&mut p, "committee_id", self.committee_id.clone());
        push_param(&mut p, "candidate_id", self.candidate_id.clone());
        push_param(&mut p, "form_type", self.form_type.clone());
        push_param(&mut p, "report_type", self.report_type.clone());
        push_param(&mut p, "cycle", self.cycle.map(|c| c.to_string()));
        push_param(
            &mut p,
            "most_recent",
            self.most_recent.map(|b| b.to_string()),
        );
        push_param(
            &mut p,
            "min_receipt_date",
            self.min_receipt_date.map(|d| d.to_string()),
        );
        push_param(
            &mut p,
            "max_receipt_date",
            self.max_receipt_date.map(|d| d.to_string()),
        );
        for n in &self.file_numbers {
            p.push(("file_number", n.to_string()));
        }
        push_param(&mut p, "sort", self.sort.clone());
        p.push(("per_page", self.per_page.to_string()));
        p.push(("page", self.page.to_string()));
        Ok(encode_params(&p))
    }
}

/// Parameters for `/efile/filings/`. Same conventions as
/// [`FilingsQuery`]; defaults `per_page` 100, `page` 1, `sort`
/// `-receipt_date`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct EfileQuery {
    pub committee_id: Option<String>,
    pub form_type: Option<String>,
    pub file_number: Option<u64>,
    /// Further specific filings; sent with `file_number` as repeated
    /// `file_number=` parameters.
    pub file_numbers: Vec<u64>,
    pub min_receipt_date: Option<NaiveDate>,
    pub max_receipt_date: Option<NaiveDate>,
    pub sort: Option<String>,
    pub per_page: u32,
    pub page: u32,
}

impl Default for EfileQuery {
    fn default() -> Self {
        EfileQuery {
            committee_id: None,
            form_type: None,
            file_number: None,
            file_numbers: Vec::new(),
            min_receipt_date: None,
            max_receipt_date: None,
            sort: Some("-receipt_date".to_string()),
            per_page: 100,
            page: 1,
        }
    }
}

impl EfileQuery {
    /// An unfiltered query with the defaults above.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `committee_id=C00554709`.
    #[must_use]
    pub fn committee_id(mut self, id: impl Into<String>) -> Self {
        self.committee_id = Some(id.into());
        self
    }

    /// `form_type=F3X`.
    #[must_use]
    pub fn form_type(mut self, form_type: impl Into<String>) -> Self {
        self.form_type = Some(form_type.into());
        self
    }

    /// `file_number=2011929`: one specific filing.
    #[must_use]
    pub fn file_number(mut self, file_number: u64) -> Self {
        self.file_number = Some(file_number);
        self
    }

    /// Several specific filings, as repeated `file_number=` parameters
    /// (added to any already set).
    #[must_use]
    pub fn file_numbers(mut self, file_numbers: impl IntoIterator<Item = u64>) -> Self {
        self.file_numbers.extend(file_numbers);
        self
    }

    /// `min_receipt_date=YYYY-MM-DD` (inclusive).
    #[must_use]
    pub fn min_receipt_date(mut self, date: NaiveDate) -> Self {
        self.min_receipt_date = Some(date);
        self
    }

    /// `max_receipt_date=YYYY-MM-DD` (inclusive).
    #[must_use]
    pub fn max_receipt_date(mut self, date: NaiveDate) -> Self {
        self.max_receipt_date = Some(date);
        self
    }

    /// `sort=` field; `None` for the API's default order.
    #[must_use]
    pub fn sort(mut self, sort: Option<impl Into<String>>) -> Self {
        self.sort = sort.map(Into::into);
        self
    }

    /// Results per page, 1..=100.
    #[must_use]
    pub fn per_page(mut self, per_page: u32) -> Self {
        self.per_page = per_page;
        self
    }

    /// 1-based page number.
    #[must_use]
    pub fn page(mut self, page: u32) -> Self {
        self.page = page;
        self
    }

    /// The query string (no leading `?`, no API key); see
    /// [`FilingsQuery::query_string`].
    pub fn query_string(&self) -> Result<String> {
        check_per_page(self.per_page)?;
        check_page(self.page)?;
        let mut p = Vec::new();
        push_param(&mut p, "committee_id", self.committee_id.clone());
        push_param(&mut p, "form_type", self.form_type.clone());
        push_param(
            &mut p,
            "file_number",
            self.file_number.map(|n| n.to_string()),
        );
        for n in &self.file_numbers {
            p.push(("file_number", n.to_string()));
        }
        push_param(
            &mut p,
            "min_receipt_date",
            self.min_receipt_date.map(|d| d.to_string()),
        );
        push_param(
            &mut p,
            "max_receipt_date",
            self.max_receipt_date.map(|d| d.to_string()),
        );
        push_param(&mut p, "sort", self.sort.clone());
        p.push(("per_page", self.per_page.to_string()));
        p.push(("page", self.page.to_string()));
        Ok(encode_params(&p))
    }
}

/// openFEC's `pagination` object.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Pagination {
    /// Total matching results (an estimate when `is_count_exact` is
    /// false).
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub is_count_exact: Option<bool>,
    /// This page, 1-based.
    #[serde(default)]
    pub page: u32,
    /// Total pages.
    #[serde(default)]
    pub pages: u32,
    #[serde(default)]
    pub per_page: u32,
}

/// One page of results from any openFEC list endpoint.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[non_exhaustive]
pub struct Page<T> {
    #[serde(default)]
    pub api_version: Option<String>,
    #[serde(default)]
    pub pagination: Pagination,
    #[serde(default)]
    pub results: Vec<T>,
}

impl<T> Page<T> {
    /// True if the API reports pages after this one.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.pagination.page < self.pagination.pages
    }
}

/// A row of `/filings/`: the FEC's processed view of one filing (paper
/// or electronic), including where it sits in its amendment chain.
///
/// Every field is optional because openFEC documents every one as
/// nullable; unknown fields are ignored. Filing numbers are `i64`
/// because openFEC gives **paper** filings negative numbers (and they
/// appear inside e-filed reports' `amendment_chain`s when a chain began
/// on paper); [`Self::filing_id`] is the positive id a raw `.fec` exists
/// for.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
#[non_exhaustive]
pub struct FilingRecord {
    /// The filing number: positive for e-filings (`1151343`, the id the
    /// document store and the e-filing system use), negative for paper
    /// filings, `None` for some paper filings.
    pub file_number: Option<i64>,
    /// `FEC-1151343`.
    pub fec_file_id: Option<String>,
    /// Base form type: `F3`, `F3X`, `F24`, ...
    pub form_type: Option<String>,
    /// `REPORT`, `STATEMENT`, `NOTICE`, ...
    pub form_category: Option<String>,
    /// Report type code: `Q2`, `12G`, `M9`, ...
    pub report_type: Option<String>,
    pub report_type_full: Option<String>,
    pub report_year: Option<u16>,
    pub cycle: Option<u16>,
    pub committee_id: Option<String>,
    pub committee_name: Option<String>,
    pub committee_type: Option<String>,
    pub candidate_id: Option<String>,
    pub candidate_name: Option<String>,
    /// `N` new, `A` amendment, `T` termination.
    pub amendment_indicator: Option<String>,
    /// Every filing id in this report's chain up to and including this
    /// one, oldest first. Empty when the API has `null` (paper filings,
    /// unprocessed chains).
    #[serde(deserialize_with = "flexible::null_as_empty")]
    pub amendment_chain: Vec<i64>,
    /// Position in the chain: 0 for the original, 1 for the first
    /// amendment, ...
    pub amendment_version: Option<u32>,
    /// True if this is the latest version of its report.
    pub most_recent: Option<bool>,
    pub most_recent_file_number: Option<i64>,
    pub previous_file_number: Option<i64>,
    pub is_amended: Option<bool>,
    #[serde(deserialize_with = "flexible::opt_datetime")]
    pub receipt_date: Option<NaiveDateTime>,
    #[serde(deserialize_with = "flexible::opt_date")]
    pub coverage_start_date: Option<NaiveDate>,
    #[serde(deserialize_with = "flexible::opt_date")]
    pub coverage_end_date: Option<NaiveDate>,
    #[serde(deserialize_with = "flexible::opt_date")]
    pub update_date: Option<NaiveDate>,
    /// The raw `.fec` on the document store.
    pub fec_url: Option<String>,
    pub csv_url: Option<String>,
    pub pdf_url: Option<String>,
    pub html_url: Option<String>,
    /// `e-file` or `paper`.
    pub means_filed: Option<String>,
    pub document_description: Option<String>,
    pub pages: Option<u32>,
    pub beginning_image_number: Option<String>,
    pub ending_image_number: Option<String>,
    pub total_receipts: Option<Decimal>,
    pub total_disbursements: Option<Decimal>,
    pub total_individual_contributions: Option<Decimal>,
    pub total_independent_expenditures: Option<Decimal>,
    pub total_communication_cost: Option<Decimal>,
    pub cash_on_hand_beginning_period: Option<Decimal>,
    pub cash_on_hand_end_period: Option<Decimal>,
    pub debts_owed_by_committee: Option<Decimal>,
    pub debts_owed_to_committee: Option<Decimal>,
    pub net_donations: Option<Decimal>,
    pub house_personal_funds: Option<Decimal>,
    pub senate_personal_funds: Option<Decimal>,
    pub opposition_personal_funds: Option<Decimal>,
    pub office: Option<String>,
    pub state: Option<String>,
    pub party: Option<String>,
    pub primary_general_indicator: Option<String>,
    pub election_year: Option<u16>,
    pub treasurer_name: Option<String>,
    pub request_type: Option<String>,
    pub document_type: Option<String>,
    pub document_type_full: Option<String>,
    pub sub_id: Option<String>,
}

impl FilingRecord {
    /// The id a raw `.fec` can be fetched by: `file_number` when it is
    /// positive. `None` for paper filings (negative or absent number),
    /// which have no electronic original.
    #[must_use]
    pub fn filing_id(&self) -> Option<u64> {
        self.file_number.and_then(positive_id)
    }

    /// True if openFEC marks this as filed on paper (`means_filed`), or
    /// the number is negative.
    #[must_use]
    pub fn is_paper(&self) -> bool {
        self.means_filed.as_deref() == Some("paper") || self.file_number.is_some_and(|n| n < 0)
    }

    /// The raw-filing URL to fetch: `fec_url` if the API gave one, else
    /// the document store's URL for [`Self::filing_id`]. `None` without
    /// either (paper filings).
    #[must_use]
    pub fn raw_url(&self) -> Option<String> {
        self.fec_url
            .clone()
            .or_else(|| self.filing_id().map(super::docquery_url))
    }
}

fn positive_id(n: i64) -> Option<u64> {
    u64::try_from(n).ok().filter(|&n| n > 0)
}

/// A row of `/efile/filings/`: an e-filing as the e-filing system
/// recorded it, available minutes after receipt and before processing.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
#[non_exhaustive]
pub struct EfileRecord {
    /// Positive: everything here was e-filed. `i64` for symmetry with
    /// [`FilingRecord::file_number`].
    pub file_number: Option<i64>,
    pub fec_file_id: Option<String>,
    pub committee_id: Option<String>,
    pub committee_name: Option<String>,
    /// Base form type (`F3X`, `F99`), without amendment suffix.
    pub form_type: Option<String>,
    #[serde(deserialize_with = "flexible::opt_datetime")]
    pub receipt_date: Option<NaiveDateTime>,
    #[serde(deserialize_with = "flexible::opt_date")]
    pub filed_date: Option<NaiveDate>,
    #[serde(deserialize_with = "flexible::opt_datetime")]
    pub load_timestamp: Option<NaiveDateTime>,
    #[serde(deserialize_with = "flexible::opt_date")]
    pub coverage_start_date: Option<NaiveDate>,
    #[serde(deserialize_with = "flexible::opt_date")]
    pub coverage_end_date: Option<NaiveDate>,
    /// The chain up to this filing; empty when the API has `null`.
    #[serde(deserialize_with = "flexible::null_as_empty")]
    pub amendment_chain: Vec<i64>,
    pub amendment_number: Option<u32>,
    pub amended_by: Option<i64>,
    pub amends_file: Option<i64>,
    pub is_amended: Option<bool>,
    pub most_recent: Option<bool>,
    pub most_recent_filing: Option<i64>,
    pub fec_url: Option<String>,
    pub csv_url: Option<String>,
    pub pdf_url: Option<String>,
    pub html_url: Option<String>,
    pub beginning_image_number: Option<String>,
    pub ending_image_number: Option<String>,
}

impl EfileRecord {
    /// The id a raw `.fec` can be fetched by; see
    /// [`FilingRecord::filing_id`].
    #[must_use]
    pub fn filing_id(&self) -> Option<u64> {
        self.file_number.and_then(positive_id)
    }

    /// The raw-filing URL to fetch; see [`FilingRecord::raw_url`].
    #[must_use]
    pub fn raw_url(&self) -> Option<String> {
        self.fec_url
            .clone()
            .or_else(|| self.filing_id().map(super::docquery_url))
    }
}

/// Parameters for `/operations-log/`. The endpoint has no `file_number`
/// filter; a filing is found by its `beginning_image_number` (which
/// `/filings/` and `/efile/filings/` both carry) or by committee and
/// report year. Defaults: `per_page` 100, `page` 1, the API's own sort
/// (`-report_year`).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct OperationsLogQuery {
    /// `candidate_committee_id=`: the filer.
    pub committee_id: Option<String>,
    pub form_type: Option<String>,
    pub report_type: Option<String>,
    pub report_year: Option<u16>,
    /// `beginning_image_number=` repeated; empty for no filter.
    pub beginning_image_numbers: Vec<String>,
    pub min_receipt_date: Option<NaiveDate>,
    pub max_receipt_date: Option<NaiveDate>,
    pub sort: Option<String>,
    pub per_page: u32,
    pub page: u32,
}

impl Default for OperationsLogQuery {
    fn default() -> Self {
        OperationsLogQuery {
            committee_id: None,
            form_type: None,
            report_type: None,
            report_year: None,
            beginning_image_numbers: Vec::new(),
            min_receipt_date: None,
            max_receipt_date: None,
            sort: None,
            per_page: 100,
            page: 1,
        }
    }
}

impl OperationsLogQuery {
    /// An unfiltered query with the defaults above.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// `candidate_committee_id=C00554709`.
    #[must_use]
    pub fn committee_id(mut self, id: impl Into<String>) -> Self {
        self.committee_id = Some(id.into());
        self
    }

    /// `form_type=F3X`.
    #[must_use]
    pub fn form_type(mut self, form_type: impl Into<String>) -> Self {
        self.form_type = Some(form_type.into());
        self
    }

    /// `report_type=M9`.
    #[must_use]
    pub fn report_type(mut self, report_type: impl Into<String>) -> Self {
        self.report_type = Some(report_type.into());
        self
    }

    /// `report_year=2026` (the year of the coverage end date).
    #[must_use]
    pub fn report_year(mut self, year: u16) -> Self {
        self.report_year = Some(year);
        self
    }

    /// One or more `beginning_image_number=` values.
    #[must_use]
    pub fn beginning_image_numbers<I, S>(mut self, numbers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.beginning_image_numbers
            .extend(numbers.into_iter().map(Into::into));
        self
    }

    /// `min_receipt_date=YYYY-MM-DD` (inclusive).
    #[must_use]
    pub fn min_receipt_date(mut self, date: NaiveDate) -> Self {
        self.min_receipt_date = Some(date);
        self
    }

    /// `max_receipt_date=YYYY-MM-DD` (inclusive).
    #[must_use]
    pub fn max_receipt_date(mut self, date: NaiveDate) -> Self {
        self.max_receipt_date = Some(date);
        self
    }

    /// `sort=` field; `None` for the API's default order.
    #[must_use]
    pub fn sort(mut self, sort: Option<impl Into<String>>) -> Self {
        self.sort = sort.map(Into::into);
        self
    }

    /// Results per page, 1..=100.
    #[must_use]
    pub fn per_page(mut self, per_page: u32) -> Self {
        self.per_page = per_page;
        self
    }

    /// 1-based page number.
    #[must_use]
    pub fn page(mut self, page: u32) -> Self {
        self.page = page;
        self
    }

    /// The query string (no leading `?`, no API key); see
    /// [`FilingsQuery::query_string`].
    pub fn query_string(&self) -> Result<String> {
        check_per_page(self.per_page)?;
        check_page(self.page)?;
        let mut p = Vec::new();
        push_param(&mut p, "candidate_committee_id", self.committee_id.clone());
        push_param(&mut p, "form_type", self.form_type.clone());
        push_param(&mut p, "report_type", self.report_type.clone());
        push_param(
            &mut p,
            "report_year",
            self.report_year.map(|y| y.to_string()),
        );
        for n in &self.beginning_image_numbers {
            p.push(("beginning_image_number", n.clone()));
        }
        push_param(
            &mut p,
            "min_receipt_date",
            self.min_receipt_date.map(|d| d.to_string()),
        );
        push_param(
            &mut p,
            "max_receipt_date",
            self.max_receipt_date.map(|d| d.to_string()),
        );
        push_param(&mut p, "sort", self.sort.clone());
        p.push(("per_page", self.per_page.to_string()));
        p.push(("page", self.page.to_string()));
        Ok(encode_params(&p))
    }
}

/// A row of `/operations-log/`: the FEC's record of one report's passage
/// through its processing pipeline. `summary_data_complete_date` is when
/// the cover-page totals were loaded (pass 1; the report then appears in
/// `/filings/` and `/reports/`); `transaction_data_complete_date` is when
/// every itemized transaction was loaded (pass 2; the report's rows then
/// appear in `/schedules/schedule_a/` and the rest). Either is `None`
/// while that pass is pending.
#[derive(Debug, Clone, PartialEq, Default, Deserialize, Serialize)]
#[serde(default)]
#[non_exhaustive]
pub struct OperationsLogRecord {
    /// The FEC's internal report id, an integer of up to 19 digits.
    pub sub_id: Option<i64>,
    /// The filer (openFEC's name for the field).
    pub candidate_committee_id: Option<String>,
    pub form_type: Option<String>,
    pub report_type: Option<String>,
    pub report_year: Option<u16>,
    pub amendment_indicator: Option<String>,
    pub beginning_image_number: Option<String>,
    pub ending_image_number: Option<String>,
    #[serde(deserialize_with = "flexible::opt_datetime")]
    pub receipt_date: Option<NaiveDateTime>,
    #[serde(deserialize_with = "flexible::opt_date")]
    pub coverage_start_date: Option<NaiveDate>,
    #[serde(deserialize_with = "flexible::opt_date")]
    pub coverage_end_date: Option<NaiveDate>,
    /// 0 entered but not verified, 1 verified.
    pub status_num: Option<i32>,
    #[serde(deserialize_with = "flexible::opt_datetime")]
    pub summary_data_complete_date: Option<NaiveDateTime>,
    #[serde(deserialize_with = "flexible::opt_datetime")]
    pub summary_data_verification_date: Option<NaiveDateTime>,
    #[serde(deserialize_with = "flexible::opt_date")]
    pub transaction_data_complete_date: Option<NaiveDate>,
}

/// How far the FEC has processed one e-filing, as
/// [`OpenFec::processing_status`] assembles it from three endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[non_exhaustive]
pub struct ProcessingStatus {
    pub filing_id: u64,
    pub committee_id: Option<String>,
    pub form_type: Option<String>,
    pub report_type: Option<String>,
    /// True if `/efile/filings/` knows the filing (minutes after receipt).
    pub in_efile: bool,
    /// True if `/filings/` knows the filing: its summary has been
    /// processed.
    pub in_filings: bool,
    /// True if `/operations-log/` has a row for the filing.
    pub in_operations_log: bool,
    /// When the e-filing system received it (to the second from
    /// `/efile/filings/`; midnight from `/filings/` if only that knew it).
    pub received: Option<NaiveDateTime>,
    pub fec_url: Option<String>,
    pub sub_id: Option<String>,
    /// The two-year period `/filings/` assigns (`None` until processed).
    pub cycle: Option<u16>,
    pub beginning_image_number: Option<String>,
    pub ending_image_number: Option<String>,
    /// Pass 1 done: cover-page totals loaded.
    pub summary_data_complete: Option<NaiveDateTime>,
    /// Pass 2 done: every itemized transaction loaded.
    pub transaction_data_complete: Option<NaiveDate>,
}

impl ProcessingStatus {
    fn unknown(filing_id: u64) -> Self {
        ProcessingStatus {
            filing_id,
            committee_id: None,
            form_type: None,
            report_type: None,
            in_efile: false,
            in_filings: false,
            in_operations_log: false,
            received: None,
            fec_url: None,
            sub_id: None,
            cycle: None,
            beginning_image_number: None,
            ending_image_number: None,
            summary_data_complete: None,
            transaction_data_complete: None,
        }
    }

    /// Whole days from receipt to the summary load, or `None` while
    /// pending or when the receipt date is unknown. Negative values are
    /// clamped to zero (the log records dates, not times).
    #[must_use]
    pub fn summary_lag_days(&self) -> Option<i64> {
        let received = self.received?.date();
        let done = self.summary_data_complete?.date();
        Some(done.signed_duration_since(received).num_days().max(0))
    }

    /// Whole days from receipt to the transaction load, or `None` while
    /// pending or when the receipt date is unknown.
    #[must_use]
    pub fn transaction_lag_days(&self) -> Option<i64> {
        let received = self.received?.date();
        let done = self.transaction_data_complete?;
        Some(done.signed_duration_since(received).num_days().max(0))
    }

    /// Whole days from receipt to `today` while the transaction load is
    /// pending; `None` once it is complete or when the receipt date is
    /// unknown.
    #[must_use]
    pub fn days_pending(&self, today: NaiveDate) -> Option<i64> {
        if self.transaction_data_complete.is_some() {
            return None;
        }
        let received = self.received?.date();
        Some(today.signed_duration_since(received).num_days().max(0))
    }

    /// The furthest stage reached.
    #[must_use]
    pub fn stage(&self) -> ProcessingStage {
        if self.transaction_data_complete.is_some() {
            ProcessingStage::TransactionsLoaded
        } else if self.summary_data_complete.is_some() || self.in_filings {
            ProcessingStage::SummaryLoaded
        } else if self.in_efile {
            ProcessingStage::Received
        } else {
            ProcessingStage::Unknown
        }
    }
}

/// Where a filing is in the FEC's pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, strum::Display, Serialize)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ProcessingStage {
    /// No endpoint knows the filing id.
    Unknown,
    /// The e-filing system has it; nothing is processed yet.
    Received,
    /// The cover-page totals are in `/filings/`; transactions are pending.
    SummaryLoaded,
    /// Every transaction is loaded.
    TransactionsLoaded,
}

/// Auto-paginating iterator from [`OpenFec::filings_all`].
#[derive(Debug)]
pub struct FilingsIter<'a> {
    client: &'a OpenFec,
    query: FilingsQuery,
    buffered: VecDeque<FilingRecord>,
    done: bool,
}

impl Iterator for FilingsIter<'_> {
    type Item = Result<FilingRecord>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(r) = self.buffered.pop_front() {
                return Some(Ok(r));
            }
            if self.done {
                return None;
            }
            match self.client.filings(&self.query) {
                Ok(page) => {
                    if page.results.is_empty() || !page.has_more() {
                        self.done = true;
                    }
                    self.query.page = self.query.page.saturating_add(1);
                    self.buffered.extend(page.results);
                    if self.buffered.is_empty() {
                        return None;
                    }
                }
                Err(e) => {
                    self.done = true;
                    return Some(Err(e));
                }
            }
        }
    }
}

/// openFEC dates arrive as `YYYY-MM-DD`; timestamps as
/// `YYYY-MM-DDTHH:MM:SS` (sometimes with fractional seconds, rarely
/// with a zone suffix we drop). Accept all of those, `null`, and the
/// empty string; anything else is a decode error naming the value.
mod flexible {
    use super::*;

    const DATETIME_FORMATS: &[&str] = &[
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f%:z",
        "%Y-%m-%dT%H:%M:%S%.fZ",
        "%Y-%m-%d %H:%M:%S%.f",
    ];

    fn parse_datetime(s: &str) -> Option<NaiveDateTime> {
        if let Ok(d) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
            return d.and_hms_opt(0, 0, 0);
        }
        DATETIME_FORMATS
            .iter()
            .find_map(|f| NaiveDateTime::parse_from_str(s, f).ok())
            .or_else(|| {
                chrono::DateTime::parse_from_rfc3339(s)
                    .ok()
                    .map(|dt| dt.naive_local())
            })
    }

    pub fn opt_datetime<'de, D: Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Option<NaiveDateTime>, D::Error> {
        let raw: Option<String> = Option::deserialize(d)?;
        match raw.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some(s) => parse_datetime(s).map(Some).ok_or_else(|| {
                serde::de::Error::custom(format!("'{s}' is not an ISO date or timestamp"))
            }),
        }
    }

    /// `null` (or a missing field, via `default`) as an empty list.
    pub fn null_as_empty<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
        d: D,
    ) -> std::result::Result<Vec<T>, D::Error> {
        Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
    }

    pub fn opt_date<'de, D: Deserializer<'de>>(
        d: D,
    ) -> std::result::Result<Option<NaiveDate>, D::Error> {
        let raw: Option<String> = Option::deserialize(d)?;
        match raw.as_deref().map(str::trim) {
            None | Some("") => Ok(None),
            Some(s) => parse_datetime(s)
                .map(|dt| Some(dt.date()))
                .ok_or_else(|| serde::de::Error::custom(format!("'{s}' is not an ISO date"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_validation_and_redaction() {
        assert!(matches!(ApiKey::new(""), Err(FecApiError::InvalidQuery(_))));
        assert!(matches!(
            ApiKey::new("abc\n"),
            Err(FecApiError::InvalidQuery(_))
        ));
        let key = ApiKey::new("SECRETKEY").unwrap();
        assert_eq!(format!("{key:?}"), "ApiKey(REDACTED)");
        assert_eq!(key.to_string(), "REDACTED");
        let client = OpenFec::new(key);
        let dbg = format!("{client:?}");
        assert!(!dbg.contains("SECRETKEY"), "{dbg}");
        assert!(dbg.contains("api.open.fec.gov"));
    }

    #[test]
    fn query_validation() {
        assert!(FilingsQuery::new().cycle(2027).query_string().is_err());
        assert!(FilingsQuery::new().cycle(1974).query_string().is_err());
        assert!(FilingsQuery::new().per_page(0).query_string().is_err());
        assert!(FilingsQuery::new().per_page(101).query_string().is_err());
        assert!(FilingsQuery::new().page(0).query_string().is_err());
        assert!(EfileQuery::new().per_page(500).query_string().is_err());
        assert_eq!(
            FilingsQuery::new().query_string().unwrap(),
            "sort=-receipt_date&per_page=100&page=1"
        );
        assert_eq!(
            FilingsQuery::new()
                .sort(None::<String>)
                .form_type("F3 X")
                .query_string()
                .unwrap(),
            "form_type=F3%20X&per_page=100&page=1"
        );
    }

    #[test]
    fn flexible_dates() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "flexible::opt_datetime")]
            t: Option<NaiveDateTime>,
            #[serde(default, deserialize_with = "flexible::opt_date")]
            d: Option<NaiveDate>,
        }
        let r: R = serde_json::from_str(r#"{"t":"2017-03-04T00:00:00","d":"2016-10-01"}"#).unwrap();
        assert_eq!(r.t.unwrap().to_string(), "2017-03-04 00:00:00");
        assert_eq!(r.d.unwrap().to_string(), "2016-10-01");
        let r: R =
            serde_json::from_str(r#"{"t":"2017-03-04","d":"2016-10-01T12:00:00.5"}"#).unwrap();
        assert_eq!(r.t.unwrap().to_string(), "2017-03-04 00:00:00");
        assert_eq!(r.d.unwrap().to_string(), "2016-10-01");
        let r: R = serde_json::from_str(r#"{"t":null,"d":""}"#).unwrap();
        assert_eq!((r.t, r.d), (None, None));
        let r: R = serde_json::from_str(r#"{"t":"2026-09-14T22:50:35+00:00"}"#).unwrap();
        assert_eq!(r.t.unwrap().to_string(), "2026-09-14 22:50:35");
        assert!(serde_json::from_str::<R>(r#"{"d":"yesterday"}"#).is_err());
    }
}
