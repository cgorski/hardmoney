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
#[cfg(feature = "serde")]
pub mod openfec;

use std::io::Read;
use std::sync::OnceLock;
use std::time::Duration;

use ureq::http;

pub use cache::{Cache, CacheInfo};
pub use efile::{DailyZipFilings, EfileFeed, FeedItem, daily_zip_filings};
#[cfg(feature = "serde")]
pub use openfec::{
    ApiKey, EfileQuery, EfileRecord, FilingRecord, FilingsIter, FilingsQuery, OpenFec, Page,
    Pagination,
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

/// The raw-filing URL on the FEC's document store for a filing id.
///
/// openFEC's `fec_url` is this same URL; [`fetch_filing_bytes`] uses it
/// when no other URL is known.
#[must_use]
pub fn docquery_url(filing_id: u64) -> String {
    format!("https://docquery.fec.gov/dcdev/posted/{filing_id}.fec")
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
    let response = agent.get(url).call()?;
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

/// Downloads a raw `.fec` filing, cache-first.
///
/// Order of business:
///
/// 1. If `cache` already holds `<id>.fec`, return it without touching the
///    network.
/// 2. Otherwise GET `url_hint` (openFEC's `fec_url`, or the RSS `<link>`)
///    if one is given; if that is a different URL from the document
///    store's and it fails with an HTTP error, fall back to
///    [`docquery_url`].
/// 3. Store the bytes in the cache (atomically) and return them.
///
/// Fails with [`FecApiError::Http`] if the filing does not exist (the
/// document store answers 404) or if the server returned an HTML page
/// instead of a filing, [`FecApiError::Transport`] on network failure,
/// and [`FecApiError::Io`] if the cache cannot be written.
pub fn fetch_filing_bytes(id: u64, cache: &Cache, url_hint: Option<&str>) -> Result<Vec<u8>> {
    if let Some(bytes) = cache.read_filing(id)? {
        return Ok(bytes);
    }
    let fallback = docquery_url(id);
    let bytes = match url_hint.filter(|u| !u.trim().is_empty()) {
        Some(hint) if hint != fallback => match download_filing(hint) {
            Ok(bytes) => bytes,
            Err(FecApiError::Http { .. }) => download_filing(&fallback)?,
            Err(e) => return Err(e),
        },
        _ => download_filing(&fallback)?,
    };
    cache.write_filing(id, &bytes)?;
    Ok(bytes)
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
