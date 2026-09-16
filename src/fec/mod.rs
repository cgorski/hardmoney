//! Filing discovery: find filings through the FEC's openFEC API, follow
//! the electronic-filing feed, and download raw `.fec` files through a
//! local cache.
//!
//! The parser in [`crate::parser`] needs bytes; this module is how you get
//! them without knowing a filing id in advance. Three sources, in
//! decreasing order of latency and increasing order of rawness:
//!
//! | Source | What it is | Latency | Needs an API key |
//! |---|---|---|---|
//! | [`openfec::OpenFec::filings`] (`/filings/`) | **Processed** filing metadata: amendment chains, `most_recent`, cover-page totals, both e-filed and paper. | Hours to days (the FEC's processing pipeline). | yes |
//! | [`openfec::OpenFec::efile_filings`] (`/efile/filings/`) | **Raw** e-filing metadata straight from the e-filing system. | Minutes. | yes |
//! | [`efile::EfileFeed`] (RSS) and [`efile::daily_zip_filings`] (daily archives) | The e-filing system's own feed of what just arrived, and one zip per day of every e-filing received. | Minutes (feed); next day (zips). | no |
//!
//! Whichever source names a filing, [`fetch_filing_bytes`] downloads the
//! raw `.fec` once and keeps it in the [`cache::Cache`]
//! (`~/.cache/hardmoney/filings/<id>.fec`, override with
//! `HARDMONEY_CACHE_DIR`).
//!
//! # Where the requests go
//!
//! Every URL this module requests is built by [`Endpoints`] from a base
//! that defaults to the FEC's production host and can be overridden by a
//! `HARDMONEY_*` environment variable or a builder ([`endpoints`] has the
//! table). Functions here that take an `&Endpoints` use it; the ones that
//! do not and return a `Result` read [`Endpoints::from_env`] and fail if
//! an override is malformed; infallible constructors (`EfileFeed::new`,
//! `OpenFec::new`) use the production defaults.
//!
//! # Feature flags
//!
//! This module is enabled by the `fetch` feature. The openFEC client in
//! [`openfec`] additionally needs the `serde` feature (it decodes JSON);
//! with `fetch` alone you get the cache, [`fetch_filing_bytes`], the RSS
//! feed, and the daily zips.
//!
//! # API key
//!
//! openFEC requires a key (free at <https://api.data.gov/signup/>).
//! [`openfec::ApiKey::from_env`] reads `FEC_API_KEY`, then
//! `~/fec_api_key.txt`. The key never appears in `Debug` output, error
//! messages, or URLs this module reports: [`FecApiError::Http`] carries
//! the URL with `api_key=REDACTED`.
//!
//! # Rate limiting
//!
//! openFEC allows 1,000 requests per hour per key. On HTTP 429 the client
//! honours `Retry-After` and retries up to three times (each wait capped
//! at 60 seconds by default); past that, or when the server asks for a
//! longer wait, it returns [`FecApiError::RateLimited`] so the caller can
//! decide.
//!
//! # Example: validate everything on the feed right now
//!
//! No API key needed. (For the openFEC route see the example on
//! [`openfec`].)
//!
//! ```no_run
//! use hardmoney::fec::{Cache, EfileFeed, fetch_filing_bytes};
//! use hardmoney::{Filing, ParseOptions};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let cache = Cache::from_env();
//! for item in EfileFeed::new().poll()? {
//!     if !item.matches_form_type("F3X") {
//!         continue;
//!     }
//!     let bytes = fetch_filing_bytes(item.filing_id, &cache, Some(&item.url))?;
//!     let filing = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)?;
//!     println!("{}: {} error(s)", item.filing_id, filing.validate().error_count());
//! }
//! # Ok(())
//! # }
//! ```

pub mod cache;
pub mod efile;
pub mod endpoints;
#[cfg(feature = "serde")]
pub mod openfec;

use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

use ureq::http;

pub use cache::{Cache, CacheInfo};
pub use efile::{DailyZipFilings, EfileFeed, FeedItem, daily_zip_filings};
pub use endpoints::{EndpointError, Endpoints, Url};
#[cfg(feature = "serde")]
pub use openfec::{
    ApiKey, EfileQuery, EfileRecord, FilingRecord, FilingsIter, FilingsQuery, OpenFec,
    OperationsLogQuery, OperationsLogRecord, Page, Pagination, ProcessingStage, ProcessingStatus,
    ScheduleEndpoint,
};

/// Errors from the openFEC API, the e-file feed, and raw-filing downloads.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum FecApiError {
    /// No API key in `FEC_API_KEY` or `~/fec_api_key.txt`.
    #[error(
        "no openFEC API key found (looked in {}); set FEC_API_KEY or save the key in \
         ~/fec_api_key.txt -- a free key is at https://api.data.gov/signup/",
        looked_in.join(", ")
    )]
    MissingApiKey {
        /// Where the key was looked for, in order.
        looked_in: Vec<String>,
    },

    /// A non-success HTTP status other than 429. `url` has any `api_key`
    /// value replaced with `REDACTED`.
    #[error("{url} returned HTTP {status}{}", body_hint(body_snippet))]
    Http {
        status: u16,
        url: String,
        /// The first few hundred characters of the response body,
        /// whitespace-collapsed, for diagnosis. Empty if there was none.
        body_snippet: String,
    },

    /// HTTP 429 after the retry budget was spent, or with a `Retry-After`
    /// longer than the client is willing to sleep.
    #[error(
        "openFEC rate limit reached (1,000 requests per hour per key); {}",
        match retry_after {
            Some(d) => format!("the server asks to retry after {} second(s)", d.as_secs()),
            None => "retry later".to_string(),
        }
    )]
    RateLimited { retry_after: Option<Duration> },

    /// Could not reach the server, or the connection failed mid-response.
    #[error("HTTP transport error: {0}")]
    Transport(#[from] ureq::Error),

    /// The response was not the JSON we expected. `url` is redacted.
    #[cfg(feature = "serde")]
    #[error("could not decode the JSON from {url}: {source}")]
    Json {
        url: String,
        #[source]
        source: serde_json::Error,
    },

    /// A cache read or write failed.
    #[error("{0}")]
    Io(#[from] std::io::Error),

    /// A query parameter is out of range (odd cycle, `per_page` > 100,
    /// a date the FEC has no archive for, ...).
    #[error("invalid query: {0}")]
    InvalidQuery(String),

    /// The RSS feed was not the document we expected.
    #[error("could not parse the e-file RSS feed: {0}")]
    Rss(String),

    /// A daily e-filing archive could not be read as a zip.
    #[error("{} is not a readable zip archive: {source}", path.display())]
    Zip {
        path: std::path::PathBuf,
        #[source]
        source: zip::result::ZipError,
    },

    /// A `HARDMONEY_*` endpoint override is not a usable URL (see
    /// [`Endpoints::from_env`]). Raised before any request is made.
    #[error("{0}")]
    Endpoint(#[from] EndpointError),
}

fn body_hint(snippet: &str) -> String {
    if snippet.is_empty() {
        String::new()
    } else {
        format!(": {snippet}")
    }
}

/// `Result` with [`FecApiError`].
pub type Result<T> = std::result::Result<T, FecApiError>;

/// The `User-Agent` this module sends. The FEC asks API consumers to
/// identify themselves.
pub const USER_AGENT: &str = concat!("hardmoney/", env!("CARGO_PKG_VERSION"));

/// The raw-filing URL on the FEC's production document store for a
/// filing id: `https://docquery.fec.gov/dcdev/posted/<id>.fec`.
///
/// This is [`Endpoints::docquery_filing`] at the default base; use that
/// method when a `HARDMONEY_DOCQUERY_BASE` override must be honoured.
/// openFEC's `fec_url` is this same URL today, but the FEC is
/// inventorying `docquery.fec.gov` for retirement (`openFEC#6717`), so
/// prefer [`resolve_fec_url`], which asks openFEC and only falls back to
/// this template. [`fetch_filing_bytes`] does that when no URL is given.
#[must_use]
pub fn docquery_url(filing_id: u64) -> String {
    Endpoints::default().docquery_filing(filing_id)
}

/// The URL to download `filing_id`'s raw `.fec` from.
///
/// [`resolve_fec_url_with`] at [`Endpoints::from_env`]. Fails with
/// [`FecApiError::Endpoint`] if a `HARDMONEY_*` override is malformed,
/// otherwise as `resolve_fec_url_with` does.
pub fn resolve_fec_url(filing_id: u64) -> Result<String> {
    resolve_fec_url_with(filing_id, &Endpoints::from_env()?)
}

/// The URL to download `filing_id`'s raw `.fec` from, with the openFEC
/// and document-store hosts taken from `endpoints`.
///
/// With an openFEC key available ([`openfec::ApiKey::from_env`]) and the
/// `serde` feature, asks `/efile/filings/` and then `/filings/` for the
/// filing's `fec_url` ([`openfec::OpenFec::resolve_fec_url`]); that is one
/// or two API requests. The answer has any production `docquery.fec.gov`
/// host rewritten to `endpoints.docquery_base`
/// ([`Endpoints::rewrite_docquery`]). Returns
/// [`Endpoints::docquery_filing`] when there is no key, when neither
/// endpoint knows the filing, or in a build without `serde`. Fails only
/// when an API request fails ([`FecApiError::Http`],
/// [`FecApiError::RateLimited`], [`FecApiError::Transport`], ...).
pub fn resolve_fec_url_with(filing_id: u64, endpoints: &Endpoints) -> Result<String> {
    #[cfg(feature = "serde")]
    {
        match openfec::ApiKey::from_env() {
            Ok(key) => {
                let api = openfec::OpenFec::new(key).with_endpoints(endpoints);
                if let Some(url) = api.resolve_fec_url(filing_id)? {
                    return Ok(endpoints.rewrite_docquery(&url));
                }
            }
            Err(FecApiError::MissingApiKey { .. }) => {}
            Err(e) => return Err(e),
        }
    }
    Ok(endpoints.docquery_filing(filing_id))
}

/// Parses an HTTP `Retry-After` header value: either a non-negative
/// number of seconds or an HTTP-date (RFC 7231 / RFC 2822 form), which is
/// converted to a duration from now (zero if it is already past).
///
/// Returns `None` for anything else, including an empty value.
#[must_use]
pub fn parse_retry_after(value: &str) -> Option<Duration> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    if let Ok(secs) = value.parse::<u64>() {
        return Some(Duration::from_secs(secs));
    }
    let when = chrono::DateTime::parse_from_rfc2822(value).ok()?;
    let wait = when.signed_duration_since(chrono::Utc::now());
    Some(wait.to_std().unwrap_or(Duration::ZERO))
}

/// Replaces the value of any `api_key` query parameter in `url` with
/// `REDACTED`. Idempotent; a URL without the parameter is unchanged.
#[must_use]
pub fn redact_url(url: &str) -> String {
    const KEY: &str = "api_key=";
    let mut out = String::with_capacity(url.len());
    let mut rest = url;
    while let Some(pos) = rest.find(KEY) {
        let (before, after) = rest.split_at(pos);
        out.push_str(before);
        out.push_str(KEY);
        out.push_str("REDACTED");
        let value = &after[KEY.len()..];
        rest = match value.find(['&', '#']) {
            Some(end) => &value[end..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// One shared, connection-pooling HTTP agent for the whole module.
///
/// Status codes are *not* turned into `ureq` errors: every caller
/// inspects the status itself so 429 and 404 become typed errors with the
/// URL and body attached.
pub(crate) fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .user_agent(USER_AGENT)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .build()
            .new_agent()
    })
}

/// Maximum body bytes kept for an error message.
const SNIPPET_BYTES: u64 = 400;

/// Performs a GET and returns the response if its status is 2xx. A 429
/// becomes [`FecApiError::RateLimited`] (with the parsed `Retry-After`),
/// any other non-2xx [`FecApiError::Http`] with the redacted URL and a
/// short body snippet.
pub(crate) fn get_ok(agent: &ureq::Agent, url: &str) -> Result<http::Response<ureq::Body>> {
    let response = agent.get(url).call().map_err(redact_transport_error)?;
    let status = response.status().as_u16();
    if (200..300).contains(&status) {
        return Ok(response);
    }
    if status == 429 {
        let retry_after = response
            .headers()
            .get(http::header::RETRY_AFTER)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_retry_after);
        return Err(FecApiError::RateLimited { retry_after });
    }
    let body_snippet = response
        .into_body()
        .with_config()
        .limit(SNIPPET_BYTES)
        .lossy_utf8(true)
        .read_to_string()
        .map(|s| collapse_whitespace(&redact_url(&s)))
        .unwrap_or_default();
    Err(FecApiError::Http {
        status,
        url: redact_url(url),
        body_snippet,
    })
}

/// Two `ureq` error variants quote the request URI in their message
/// (`bad uri: <uri> is missing scheme`, `configured for https only:
/// <uri>`); with a misconfigured `OpenFec::with_base_url` that URI
/// carries the key. Everything else passes through unchanged.
fn redact_transport_error(e: ureq::Error) -> FecApiError {
    FecApiError::Transport(match e {
        ureq::Error::BadUri(s) => ureq::Error::BadUri(redact_url(&s)),
        ureq::Error::RequireHttpsOnly(s) => ureq::Error::RequireHttpsOnly(redact_url(&s)),
        other => other,
    })
}

/// Reads a whole response body with no size limit (raw filings run to
/// 135 MB).
pub(crate) fn read_all(response: http::Response<ureq::Body>) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    response.into_body().into_reader().read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn collapse_whitespace(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Downloads a raw `.fec` filing, cache-first, at [`Endpoints::from_env`].
///
/// [`fetch_filing_bytes_with`] with the endpoints from the environment.
/// Fails with [`FecApiError::Endpoint`] if a `HARDMONEY_*` override is
/// malformed (checked before the cache is consulted, so a bad
/// configuration is never masked by a cache hit), otherwise as
/// `fetch_filing_bytes_with` does.
pub fn fetch_filing_bytes(id: u64, cache: &Cache, url_hint: Option<&str>) -> Result<Vec<u8>> {
    fetch_filing_bytes_with(id, cache, url_hint, &Endpoints::from_env()?)
}

/// Downloads a raw `.fec` filing, cache-first, from the hosts in
/// `endpoints`.
///
/// Order of business:
///
/// 1. If `cache` already holds `<id>.fec`, return it without touching the
///    network.
/// 2. Otherwise GET `url_hint` (openFEC's `fec_url`, or the RSS `<link>`)
///    if one is given, with a production `docquery.fec.gov` host rewritten
///    to `endpoints.docquery_base` ([`Endpoints::rewrite_docquery`]).
///    Without a hint, ask openFEC for the filing's `fec_url`
///    ([`resolve_fec_url_with`]; one or two requests, skipped when there
///    is no API key) and GET that. If the URL is not the document store's
///    and fails with an HTTP error, or the resolution itself fails, fall
///    back to [`Endpoints::docquery_filing`].
/// 3. Store the bytes in the cache (atomically) and return them.
///
/// Fails with [`FecApiError::Http`] if the filing does not exist (the
/// document store answers 404) or if the server returned an HTML page
/// instead of a filing, [`FecApiError::Transport`] on network failure,
/// and [`FecApiError::Io`] if the cache cannot be written.
pub fn fetch_filing_bytes_with(
    id: u64,
    cache: &Cache,
    url_hint: Option<&str>,
    endpoints: &Endpoints,
) -> Result<Vec<u8>> {
    if let Some(bytes) = cache.read_filing(id)? {
        return Ok(bytes);
    }
    let fallback = endpoints.docquery_filing(id);
    let url = match url_hint.map(str::trim).filter(|u| !u.is_empty()) {
        Some(hint) => endpoints.rewrite_docquery(hint),
        // Best effort: a failed lookup is not a failed download.
        None => resolve_fec_url_with(id, endpoints).unwrap_or_else(|_| fallback.clone()),
    };
    let bytes = if url == fallback {
        download_filing(&fallback)?
    } else {
        match download_filing(&url) {
            Ok(bytes) => bytes,
            Err(FecApiError::Http { .. }) => download_filing(&fallback)?,
            Err(e) => return Err(e),
        }
    };
    cache.write_filing(id, &bytes)?;
    Ok(bytes)
}

/// Downloads `filing_id` from `endpoints.docquery_filing(filing_id)`
/// with no cache and no openFEC lookup: one GET of the document store.
///
/// Fails with [`FecApiError::Http`] if the store answers non-2xx or with
/// an HTML page instead of a filing, and [`FecApiError::Transport`] on
/// network failure. This is [`crate::Filing::fetch_bytes`] with explicit
/// endpoints and typed errors.
pub fn download_filing_bytes(filing_id: u64, endpoints: &Endpoints) -> Result<Vec<u8>> {
    download_filing(&endpoints.docquery_filing(filing_id))
}

fn download_filing(url: &str) -> Result<Vec<u8>> {
    let response = get_ok(agent(), url)?;
    let status = response.status().as_u16();
    let bytes = read_all(response)?;
    if looks_like_html(&bytes) {
        return Err(FecApiError::Http {
            status,
            url: redact_url(url),
            body_snippet: collapse_whitespace(&String::from_utf8_lossy(
                bytes
                    .get(..usize::try_from(SNIPPET_BYTES).unwrap_or(400))
                    .unwrap_or(&bytes),
            )),
        });
    }
    Ok(bytes)
}

/// A `.fec` file starts with `HDR` (or `/*` for the pre-3.0 header); an
/// HTML error page served with a 200 does not.
fn looks_like_html(bytes: &[u8]) -> bool {
    let head: Vec<u8> = bytes
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take(15)
        .map(u8::to_ascii_lowercase)
        .collect();
    head.starts_with(b"<!doctype") || head.starts_with(b"<html")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_key_anywhere_in_query() {
        assert_eq!(
            redact_url("https://x/v1/filings/?api_key=SECRET&page=2"),
            "https://x/v1/filings/?api_key=REDACTED&page=2"
        );
        assert_eq!(
            redact_url("https://x/v1/filings/?page=2&api_key=SECRET"),
            "https://x/v1/filings/?page=2&api_key=REDACTED"
        );
        assert_eq!(
            redact_url("https://x/?api_key=SECRET#frag"),
            "https://x/?api_key=REDACTED#frag"
        );
        assert_eq!(redact_url("https://x/?page=1"), "https://x/?page=1");
        assert_eq!(
            redact_url("https://x/?api_key=REDACTED"),
            "https://x/?api_key=REDACTED"
        );
    }

    #[test]
    fn transport_errors_that_quote_the_uri_are_redacted() {
        let e = redact_transport_error(ureq::Error::BadUri(
            "api.open.fec.gov/v1/filings/?api_key=SECRET is missing scheme".to_string(),
        ));
        let text = e.to_string();
        assert!(!text.contains("SECRET"), "{text}");
        assert!(text.contains("api_key=REDACTED"), "{text}");
        let passthrough = redact_transport_error(ureq::Error::HostNotFound);
        assert!(matches!(
            passthrough,
            FecApiError::Transport(ureq::Error::HostNotFound)
        ));
    }

    #[test]
    fn retry_after_seconds_and_dates() {
        assert_eq!(parse_retry_after("120"), Some(Duration::from_secs(120)));
        assert_eq!(parse_retry_after(" 0 "), Some(Duration::ZERO));
        assert_eq!(parse_retry_after(""), None);
        assert_eq!(parse_retry_after("soon"), None);
        assert_eq!(
            parse_retry_after("Wed, 21 Oct 2015 07:28:00 GMT"),
            Some(Duration::ZERO)
        );
        let far = parse_retry_after("Sat, 01 Jan 2101 00:00:00 GMT").unwrap();
        assert!(far > Duration::from_secs(3600));
    }

    #[test]
    fn html_detection() {
        assert!(looks_like_html(b"\n<!DOCTYPE HTML PUBLIC"));
        assert!(looks_like_html(b"<html lang=\"en\">"));
        assert!(!looks_like_html(b"HDR\x1cFEC\x1c8.5"));
        assert!(!looks_like_html(b"/* Header"));
        assert!(!looks_like_html(b""));
    }
}
