//! Where hardmoney talks to the FEC, in one place.
//!
//! Every URL the crate requests is built by a method on [`Endpoints`] from
//! one of six bases. Each base defaults to the FEC's production address
//! and can be replaced by an environment variable ([`Endpoints::from_env`]),
//! a builder method (`with_*`), or, on the CLI, a flag. A deployment behind
//! a mirror, an egress proxy that rewrites hosts, or the FEC's test
//! environment changes the variables and nothing else.
//!
//! | Field | Variable | Default | Used for |
//! |---|---|---|---|
//! | `www_base` | `HARDMONEY_FEC_WWW_BASE` | `https://www.fec.gov` | bulk zips, `pg_dump` archives, daily e-file zips, data dictionaries |
//! | `openfec_base` | `HARDMONEY_OPENFEC_BASE` | `https://api.open.fec.gov/v1/` | the openFEC API |
//! | `docquery_base` | `HARDMONEY_DOCQUERY_BASE` | `https://docquery.fec.gov` | raw `.fec` filings (`/dcdev/posted/<id>.fec`) |
//! | `efiling_apps_base` | `HARDMONEY_EFILINGAPPS_BASE` | `https://efilingapps.fec.gov` | the e-filing applications host; the RSS feed lives under it |
//! | `efile_rss_url` | `HARDMONEY_EFILE_RSS_URL` | `https://efilingapps.fec.gov/rss/generate?preDefinedFilingType=ALL` | the e-file RSS feed, as a complete URL |
//! | `webcheck_endpoint` | `HARDMONEY_WEBCHECK_ENDPOINT` | `https://efoservices.fec.gov/webcheck` | the WebCheck validator |
//!
//! An override that is not an `http://` or `https://` URL with a host is
//! an error that names the variable ([`EndpointError::InvalidOverride`]);
//! nothing falls back to production silently. An empty or blank variable
//! counts as unset.
//!
//! Nothing in this module reads the environment except [`Endpoints::from_env`]
//! (and [`Endpoints::from_lookup`] with a lookup you supply). Constructors
//! named `new` elsewhere in the crate use [`Endpoints::default`]; functions
//! that return a `Result` and have no `Endpoints` parameter read the
//! environment once and say so in their docs.
//!
//! Proxies are separate from this: `ureq` reads `ALL_PROXY`, `HTTPS_PROXY`,
//! `HTTP_PROXY` (and their lowercase forms) and `NO_PROXY` itself.

use std::fmt;
use std::str::FromStr;

use chrono::NaiveDate;

/// An `http://` or `https://` URL with a host and no whitespace, checked
/// once at construction so every URL built from it is well-formed.
///
/// This is a checked string, not a parsed URL: it keeps the text exactly
/// as given (trailing slash included) and only guarantees the scheme, a
/// non-empty host, and the absence of whitespace, control characters, and
/// a fragment. [`Url::join`] appends a path with exactly one `/` between
/// the two whatever slashes either side has.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Url(String);

impl Url {
    /// Checks `s` (trimmed) and wraps it.
    ///
    /// Fails with [`EndpointError::InvalidUrl`] if `s` is empty, contains
    /// whitespace or a control character, does not start with `http://`
    /// or `https://`, has no host, or has a `#fragment`. A query string
    /// is allowed (the e-file RSS feed has one).
    pub fn parse(s: &str) -> Result<Self, EndpointError> {
        let value = s.trim();
        let invalid = |reason: &'static str| EndpointError::InvalidUrl {
            value: value.to_string(),
            reason,
        };
        if value.is_empty() {
            return Err(invalid("it is empty"));
        }
        if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(invalid("it contains whitespace or a control character"));
        }
        let Some((scheme, rest)) = value.split_once("://") else {
            return Err(invalid("it must start with http:// or https://"));
        };
        if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
            return Err(invalid("it must start with http:// or https://"));
        }
        if value.contains('#') {
            return Err(invalid("it must not have a #fragment"));
        }
        let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
        let authority = rest.get(..authority_end).unwrap_or("");
        // Userinfo precedes the last '@'; a bracketed IPv6 literal may hold
        // colons, otherwise the port follows the first ':'.
        let host_port = authority.rsplit('@').next().unwrap_or(authority);
        let host = if host_port.starts_with('[') {
            host_port
                .find(']')
                .and_then(|end| host_port.get(1..end))
                .unwrap_or("")
        } else {
            host_port.split(':').next().unwrap_or("")
        };
        if host.is_empty() {
            return Err(invalid("it has no host"));
        }
        Ok(Url(value.to_string()))
    }

    /// [`Url::parse`] plus: no `?query`, because paths are appended to a
    /// base and a query in the middle would corrupt them.
    pub fn parse_base(s: &str) -> Result<Self, EndpointError> {
        let url = Self::parse(s)?;
        if url.0.contains('?') {
            return Err(EndpointError::InvalidUrl {
                value: url.0,
                reason: "a base URL must not have a ?query",
            });
        }
        Ok(url)
    }

    /// The URL as text, exactly as given (trimmed).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// `self` with `path` appended and exactly one `/` between them:
    /// trailing slashes on `self` and leading slashes on `path` are
    /// collapsed, so `https://x/` + `/a` and `https://x` + `a` both give
    /// `https://x/a`. An empty `path` gives `https://x/`.
    #[must_use]
    pub fn join(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.0.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }

    /// [`Url::join`] for a `path` known at compile time to contain no
    /// whitespace or fragment, so the result is still a valid `Url`.
    fn joined(&self, path: &'static str) -> Url {
        Url(self.join(path))
    }

    /// A literal known to satisfy [`Url::parse`]; the defaults are pinned
    /// by a test that parses each one.
    fn known(s: &'static str) -> Url {
        Url(s.to_string())
    }
}

impl fmt::Display for Url {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Url {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for Url {
    type Err = EndpointError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

impl From<Url> for String {
    fn from(url: Url) -> Self {
        url.0
    }
}

/// Why a URL or an override was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum EndpointError {
    /// A value given to [`Url::parse`] or a builder is not usable.
    #[error("{value:?} is not a usable URL: {reason}")]
    InvalidUrl {
        /// The text as given (trimmed).
        value: String,
        /// What is wrong with it, as a phrase.
        reason: &'static str,
    },

    /// An environment variable (or the flag that stands in for it) holds
    /// an unusable URL. Names the variable so the operator can fix it.
    #[error("{var} is set to {value:?}, which is not a usable URL: {reason}")]
    InvalidOverride {
        /// The `HARDMONEY_*` variable.
        var: &'static str,
        /// The text as given (trimmed).
        value: String,
        /// What is wrong with it, as a phrase.
        reason: &'static str,
    },
}

impl EndpointError {
    /// The environment variable at fault, for [`EndpointError::InvalidOverride`].
    #[must_use]
    pub fn var(&self) -> Option<&'static str> {
        match self {
            EndpointError::InvalidOverride { var, .. } => Some(var),
            EndpointError::InvalidUrl { .. } => None,
        }
    }

    /// The text that was refused.
    #[must_use]
    pub fn value(&self) -> &str {
        match self {
            EndpointError::InvalidUrl { value, .. }
            | EndpointError::InvalidOverride { value, .. } => value,
        }
    }

    /// What is wrong with it, as a phrase (`it has no host`).
    #[must_use]
    pub fn reason(&self) -> &'static str {
        match self {
            EndpointError::InvalidUrl { reason, .. }
            | EndpointError::InvalidOverride { reason, .. } => reason,
        }
    }

    fn for_var(self, var: &'static str) -> Self {
        match self {
            EndpointError::InvalidUrl { value, reason }
            | EndpointError::InvalidOverride { value, reason, .. } => {
                EndpointError::InvalidOverride { var, value, reason }
            }
        }
    }
}

/// Where hardmoney talks to the FEC. Every field has the FEC's production
/// address as its default and can be overridden by environment variable,
/// builder, or CLI flag. Read once with [`Endpoints::from_env`] and pass
/// by reference.
///
/// The fields are public for inspection; construct through
/// [`Endpoints::default`], [`Endpoints::from_env`], or
/// [`Endpoints::from_lookup`] and adjust with the `with_*` builders, which
/// keep `efile_rss_url` consistent with `efiling_apps_base`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Endpoints {
    /// `https://www.fec.gov`: bulk zips, `pg_dump` archives, daily e-file
    /// zips, data dictionaries.
    pub www_base: Url,
    /// `https://api.open.fec.gov/v1/`: the openFEC API root.
    pub openfec_base: Url,
    /// `https://docquery.fec.gov`: raw filings at `/dcdev/posted/<id>.fec`.
    pub docquery_base: Url,
    /// The e-file RSS feed as a complete URL (it has a query string).
    /// Derived from `efiling_apps_base` unless set explicitly.
    pub efile_rss_url: Url,
    /// `https://efilingapps.fec.gov`: the e-filing applications host.
    pub efiling_apps_base: Url,
    /// `https://efoservices.fec.gov/webcheck`: the WebCheck validator;
    /// `/services/upload` and `/services/validate` are appended.
    pub webcheck_endpoint: Url,
}

impl Default for Endpoints {
    /// The FEC's production addresses.
    fn default() -> Self {
        Endpoints {
            www_base: Url::known(Self::DEFAULT_WWW_BASE),
            openfec_base: Url::known(Self::DEFAULT_OPENFEC_BASE),
            docquery_base: Url::known(Self::DEFAULT_DOCQUERY_BASE),
            efile_rss_url: Url::known(Self::DEFAULT_EFILE_RSS_URL),
            efiling_apps_base: Url::known(Self::DEFAULT_EFILING_APPS_BASE),
            webcheck_endpoint: Url::known(Self::DEFAULT_WEBCHECK_ENDPOINT),
        }
    }
}

impl Endpoints {
    /// Overrides [`Endpoints::www_base`].
    pub const WWW_BASE_VAR: &'static str = "HARDMONEY_FEC_WWW_BASE";
    /// Overrides [`Endpoints::openfec_base`].
    pub const OPENFEC_BASE_VAR: &'static str = "HARDMONEY_OPENFEC_BASE";
    /// Overrides [`Endpoints::docquery_base`].
    pub const DOCQUERY_BASE_VAR: &'static str = "HARDMONEY_DOCQUERY_BASE";
    /// Overrides [`Endpoints::efile_rss_url`] with a complete URL.
    pub const EFILE_RSS_URL_VAR: &'static str = "HARDMONEY_EFILE_RSS_URL";
    /// Overrides [`Endpoints::efiling_apps_base`] (and, unless
    /// [`Endpoints::EFILE_RSS_URL_VAR`] is also set, the feed URL under it).
    pub const EFILING_APPS_BASE_VAR: &'static str = "HARDMONEY_EFILINGAPPS_BASE";
    /// Overrides [`Endpoints::webcheck_endpoint`].
    pub const WEBCHECK_ENDPOINT_VAR: &'static str = "HARDMONEY_WEBCHECK_ENDPOINT";
    /// Every variable [`Endpoints::from_env`] reads.
    pub const VARS: [&'static str; 6] = [
        Self::WWW_BASE_VAR,
        Self::OPENFEC_BASE_VAR,
        Self::DOCQUERY_BASE_VAR,
        Self::EFILE_RSS_URL_VAR,
        Self::EFILING_APPS_BASE_VAR,
        Self::WEBCHECK_ENDPOINT_VAR,
    ];

    /// Production default of [`Endpoints::www_base`].
    pub const DEFAULT_WWW_BASE: &'static str = "https://www.fec.gov";
    /// Production default of [`Endpoints::openfec_base`].
    pub const DEFAULT_OPENFEC_BASE: &'static str = "https://api.open.fec.gov/v1/";
    /// Production default of [`Endpoints::docquery_base`].
    pub const DEFAULT_DOCQUERY_BASE: &'static str = "https://docquery.fec.gov";
    /// Production default of [`Endpoints::efiling_apps_base`].
    pub const DEFAULT_EFILING_APPS_BASE: &'static str = "https://efilingapps.fec.gov";
    /// Production default of [`Endpoints::efile_rss_url`].
    pub const DEFAULT_EFILE_RSS_URL: &'static str =
        "https://efilingapps.fec.gov/rss/generate?preDefinedFilingType=ALL";
    /// Production default of [`Endpoints::webcheck_endpoint`].
    pub const DEFAULT_WEBCHECK_ENDPOINT: &'static str = "https://efoservices.fec.gov/webcheck";

    /// The feed's path and query under [`Endpoints::efiling_apps_base`].
    pub const EFILE_RSS_PATH: &'static str = "rss/generate?preDefinedFilingType=ALL";
    /// The raw-filing directory under [`Endpoints::docquery_base`].
    pub const DOCQUERY_FILING_PATH: &'static str = "dcdev/posted";
    /// The bulk-download directory under [`Endpoints::www_base`].
    pub const BULK_DOWNLOADS_PATH: &'static str = "files/bulk-downloads";
    /// The `pg_dump` directory under [`Endpoints::www_base`].
    pub const DUMPS_PATH: &'static str = "files/bulk-downloads/data-dump/schedules";
    /// The daily e-file archive directory under [`Endpoints::www_base`].
    pub const DAILY_EFILE_PATH: &'static str = "files/bulk-downloads/electronic";

    /// The production addresses, then each `HARDMONEY_*` variable applied.
    ///
    /// Fails with [`EndpointError::InvalidOverride`], naming the variable,
    /// if any set variable is not an `http://`/`https://` URL with a host
    /// (or, for the five bases, carries a query string). Never falls back
    /// to the default for a variable that is set but wrong. Empty or blank
    /// variables count as unset.
    pub fn from_env() -> Result<Self, EndpointError> {
        Self::from_lookup(|var| std::env::var(var).ok())
    }

    /// [`Endpoints::from_env`] with `lookup` in place of the process
    /// environment: it is asked for each of [`Endpoints::VARS`] and answers
    /// `None` for an unset one. This is how the CLI merges flags with
    /// variables and how tests avoid touching the environment.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, EndpointError> {
        let get = |var: &'static str| {
            lookup(var)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let base = |var: &'static str, default: &Url| match get(var) {
            Some(v) => Url::parse_base(&v).map_err(|e| e.for_var(var)),
            None => Ok(default.clone()),
        };
        let defaults = Self::default();
        let efiling_apps_base = base(Self::EFILING_APPS_BASE_VAR, &defaults.efiling_apps_base)?;
        let efile_rss_url = match get(Self::EFILE_RSS_URL_VAR) {
            Some(v) => Url::parse(&v).map_err(|e| e.for_var(Self::EFILE_RSS_URL_VAR))?,
            None => efiling_apps_base.joined(Self::EFILE_RSS_PATH),
        };
        Ok(Endpoints {
            www_base: base(Self::WWW_BASE_VAR, &defaults.www_base)?,
            openfec_base: base(Self::OPENFEC_BASE_VAR, &defaults.openfec_base)?,
            docquery_base: base(Self::DOCQUERY_BASE_VAR, &defaults.docquery_base)?,
            efile_rss_url,
            efiling_apps_base,
            webcheck_endpoint: base(Self::WEBCHECK_ENDPOINT_VAR, &defaults.webcheck_endpoint)?,
        })
    }

    /// Replaces [`Endpoints::www_base`].
    #[must_use]
    pub fn with_www_base(mut self, base: Url) -> Self {
        self.www_base = base;
        self
    }

    /// Replaces [`Endpoints::openfec_base`].
    #[must_use]
    pub fn with_openfec_base(mut self, base: Url) -> Self {
        self.openfec_base = base;
        self
    }

    /// Replaces [`Endpoints::docquery_base`].
    #[must_use]
    pub fn with_docquery_base(mut self, base: Url) -> Self {
        self.docquery_base = base;
        self
    }

    /// Replaces [`Endpoints::efile_rss_url`] with a complete feed URL.
    /// Call this after [`Endpoints::with_efiling_apps_base`] if both are
    /// needed; that builder re-derives the feed URL.
    #[must_use]
    pub fn with_efile_rss_url(mut self, url: Url) -> Self {
        self.efile_rss_url = url;
        self
    }

    /// Replaces [`Endpoints::efiling_apps_base`] and re-derives
    /// [`Endpoints::efile_rss_url`] as [`Endpoints::EFILE_RSS_PATH`] under
    /// it.
    #[must_use]
    pub fn with_efiling_apps_base(mut self, base: Url) -> Self {
        self.efile_rss_url = base.joined(Self::EFILE_RSS_PATH);
        self.efiling_apps_base = base;
        self
    }

    /// Replaces [`Endpoints::webcheck_endpoint`].
    #[must_use]
    pub fn with_webcheck_endpoint(mut self, endpoint: Url) -> Self {
        self.webcheck_endpoint = endpoint;
        self
    }

    /// True if every field is at its production default.
    #[must_use]
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// A bulk-data zip: `{www_base}/files/bulk-downloads/{cycle}/{stem}{yy}.zip`,
    /// with the four-digit cycle in the path and the zero-padded two-digit
    /// year in the file name, as the FEC lays them out (`.../2026/cn26.zip`,
    /// `.../2008/cn08.zip`).
    #[cfg(feature = "bulk")]
    #[must_use]
    pub fn bulk_zip(&self, cycle: crate::Cycle, stem: &str) -> String {
        self.www_base.join(&format!(
            "{}/{cycle}/{stem}{yy:02}.zip",
            Self::BULK_DOWNLOADS_PATH,
            yy = cycle.two_digit()
        ))
    }

    /// A page or file under [`Endpoints::www_base`] by its path
    /// (`files/bulk-downloads/data_dictionaries/cn_header_file.csv`,
    /// `campaign-finance-data/all-candidates-file-description/`).
    #[must_use]
    pub fn bulk_doc(&self, path: &str) -> String {
        self.www_base.join(path)
    }

    /// One of the FEC's `pg_dump` archives by file name
    /// (`fec_fitem_sched_e.dump`).
    #[must_use]
    pub fn dump(&self, file_name: &str) -> String {
        self.www_base
            .join(&format!("{}/{file_name}", Self::DUMPS_PATH))
    }

    /// The `README.txt` that describes the dumps.
    #[must_use]
    pub fn dump_readme(&self) -> String {
        self.dump("README.txt")
    }

    /// The daily e-filing archive for `date`:
    /// `{www_base}/files/bulk-downloads/electronic/YYYYMMDD.zip`.
    #[must_use]
    pub fn daily_efile_zip(&self, date: NaiveDate) -> String {
        self.www_base.join(&format!(
            "{}/{}.zip",
            Self::DAILY_EFILE_PATH,
            date.format("%Y%m%d")
        ))
    }

    /// A raw filing: `{docquery_base}/dcdev/posted/{filing_id}.fec`.
    #[must_use]
    pub fn docquery_filing(&self, filing_id: u64) -> String {
        format!("{}{filing_id}.fec", self.docquery_filing_prefix())
    }

    /// Everything of [`Endpoints::docquery_filing`] before the id:
    /// `{docquery_base}/dcdev/posted/`. For building the URL somewhere the
    /// id is not at hand as a `u64`, such as a SQL expression
    /// (`$1 || filing_id || '.fec'`).
    #[must_use]
    pub fn docquery_filing_prefix(&self) -> String {
        format!("{}/", self.docquery_base.join(Self::DOCQUERY_FILING_PATH))
    }

    /// `url` with a production document-store host (`http://` or
    /// `https://docquery.fec.gov`) replaced by [`Endpoints::docquery_base`];
    /// any other URL is returned trimmed and otherwise unchanged. The
    /// e-file feed and openFEC both hand out production `docquery` URLs, so
    /// this is what makes a mirror work for feed- and API-driven
    /// downloads. With the default base it upgrades `http://` to
    /// `https://`, which the FEC's own redirect would do anyway.
    #[must_use]
    pub fn rewrite_docquery(&self, url: &str) -> String {
        let url = url.trim();
        for prefix in ["https://docquery.fec.gov", "http://docquery.fec.gov"] {
            if let Some(rest) = url.strip_prefix(prefix)
                && (rest.is_empty() || rest.starts_with('/'))
            {
                return self.docquery_base.join(rest);
            }
        }
        url.to_string()
    }

    /// True if `url` is under [`Endpoints::docquery_base`].
    #[must_use]
    pub fn is_docquery(&self, url: &str) -> bool {
        let base = self.docquery_base.as_str().trim_end_matches('/');
        url.strip_prefix(base)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }

    /// An openFEC request: `{openfec_base}/{path}` plus `?{query}` when
    /// `query` is non-empty. `query` is used as given (already encoded).
    #[must_use]
    pub fn openfec(&self, path: &str, query: &str) -> String {
        let mut url = self.openfec_base.join(path);
        if !query.is_empty() {
            url.push('?');
            url.push_str(query);
        }
        url
    }

    /// The e-file RSS feed URL.
    #[must_use]
    pub fn efile_rss(&self) -> &str {
        self.efile_rss_url.as_str()
    }

    /// A WebCheck URL: `{webcheck_endpoint}/{path}` (`services/upload`,
    /// `services/validate`).
    #[must_use]
    pub fn webcheck(&self, path: &str) -> String {
        self.webcheck_endpoint.join(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    /// The defaults are the addresses the crate used before they were
    /// centralised, byte for byte, and each passes the checks it would
    /// face as an override.
    #[test]
    fn defaults_are_the_previous_constants() {
        let e = Endpoints::default();
        assert_eq!(e.www_base.as_str(), "https://www.fec.gov");
        assert_eq!(e.openfec_base.as_str(), "https://api.open.fec.gov/v1/");
        assert_eq!(e.docquery_base.as_str(), "https://docquery.fec.gov");
        assert_eq!(
            e.efile_rss_url.as_str(),
            "https://efilingapps.fec.gov/rss/generate?preDefinedFilingType=ALL"
        );
        assert_eq!(e.efiling_apps_base.as_str(), "https://efilingapps.fec.gov");
        assert_eq!(
            e.webcheck_endpoint.as_str(),
            "https://efoservices.fec.gov/webcheck"
        );
        for base in [
            Endpoints::DEFAULT_WWW_BASE,
            Endpoints::DEFAULT_OPENFEC_BASE,
            Endpoints::DEFAULT_DOCQUERY_BASE,
            Endpoints::DEFAULT_EFILING_APPS_BASE,
            Endpoints::DEFAULT_WEBCHECK_ENDPOINT,
        ] {
            assert_eq!(Url::parse_base(base).unwrap().as_str(), base);
        }
        assert_eq!(
            Url::parse(Endpoints::DEFAULT_EFILE_RSS_URL)
                .unwrap()
                .as_str(),
            Endpoints::DEFAULT_EFILE_RSS_URL
        );
        assert!(e.is_default());
        assert_eq!(Endpoints::from_lookup(|_| None).unwrap(), e);
    }

    #[test]
    fn default_urls_match_what_the_crate_requested_before() {
        let e = Endpoints::default();
        assert_eq!(
            e.docquery_filing(2011915),
            "https://docquery.fec.gov/dcdev/posted/2011915.fec"
        );
        assert_eq!(
            e.docquery_filing_prefix(),
            "https://docquery.fec.gov/dcdev/posted/"
        );
        assert_eq!(
            e.daily_efile_zip(ymd(2026, 9, 6)),
            "https://www.fec.gov/files/bulk-downloads/electronic/20260906.zip"
        );
        assert_eq!(
            e.dump("fec_fitem_sched_e.dump"),
            "https://www.fec.gov/files/bulk-downloads/data-dump/schedules/fec_fitem_sched_e.dump"
        );
        assert_eq!(
            e.dump_readme(),
            "https://www.fec.gov/files/bulk-downloads/data-dump/schedules/README.txt"
        );
        assert_eq!(
            e.bulk_doc("files/bulk-downloads/data_dictionaries/cn_header_file.csv"),
            "https://www.fec.gov/files/bulk-downloads/data_dictionaries/cn_header_file.csv"
        );
        assert_eq!(
            e.openfec("filings/", "api_key=K&page=1"),
            "https://api.open.fec.gov/v1/filings/?api_key=K&page=1"
        );
        assert_eq!(
            e.openfec("/efile/filings/", ""),
            "https://api.open.fec.gov/v1/efile/filings/"
        );
        assert_eq!(
            e.efile_rss(),
            "https://efilingapps.fec.gov/rss/generate?preDefinedFilingType=ALL"
        );
        assert_eq!(
            e.webcheck("/services/upload"),
            "https://efoservices.fec.gov/webcheck/services/upload"
        );
        assert_eq!(
            e.webcheck("services/validate"),
            "https://efoservices.fec.gov/webcheck/services/validate"
        );
    }

    #[cfg(feature = "bulk")]
    #[test]
    fn bulk_zip_pads_the_two_digit_year() {
        use crate::Cycle;
        let e = Endpoints::default();
        assert_eq!(
            e.bulk_zip(Cycle::new(2026).unwrap(), "cn"),
            "https://www.fec.gov/files/bulk-downloads/2026/cn26.zip"
        );
        assert_eq!(
            e.bulk_zip(Cycle::new(2008).unwrap(), "oppexp"),
            "https://www.fec.gov/files/bulk-downloads/2008/oppexp08.zip"
        );
        let mirror = e.with_www_base(Url::parse("https://mirror.example.gov/fec/").unwrap());
        assert_eq!(
            mirror.bulk_zip(Cycle::new(2026).unwrap(), "cn"),
            "https://mirror.example.gov/fec/files/bulk-downloads/2026/cn26.zip"
        );
    }

    #[test]
    fn join_collapses_slashes_both_ways() {
        for base in ["https://x", "https://x/", "https://x//"] {
            let url = Url::parse(base).unwrap();
            for path in ["a/b", "/a/b", "//a/b"] {
                assert_eq!(url.join(path), "https://x/a/b", "{base} + {path}");
            }
            assert_eq!(url.join(""), "https://x/");
        }
        let with_path = Url::parse("https://x/fec/").unwrap();
        assert_eq!(with_path.join("/y.zip"), "https://x/fec/y.zip");
        assert_eq!(
            Url::parse("https://x/fec").unwrap().join("y.zip"),
            "https://x/fec/y.zip"
        );
    }

    #[test]
    fn every_builder_method_works_with_and_without_trailing_slash() {
        for (base, expect) in [
            ("https://m.example.gov", "https://m.example.gov"),
            ("https://m.example.gov/", "https://m.example.gov"),
            (
                "http://m.example.gov:8080/sub/",
                "http://m.example.gov:8080/sub",
            ),
        ] {
            let url = Url::parse_base(base).unwrap();
            let e = Endpoints::default()
                .with_www_base(url.clone())
                .with_openfec_base(url.clone())
                .with_docquery_base(url.clone())
                .with_efiling_apps_base(url.clone())
                .with_webcheck_endpoint(url.clone());
            assert!(!e.is_default());
            assert_eq!(e.docquery_filing(7), format!("{expect}/dcdev/posted/7.fec"));
            assert_eq!(
                e.daily_efile_zip(ymd(2001, 2, 1)),
                format!("{expect}/files/bulk-downloads/electronic/20010201.zip")
            );
            assert_eq!(
                e.dump("x.dump"),
                format!("{expect}/files/bulk-downloads/data-dump/schedules/x.dump")
            );
            assert_eq!(
                e.dump_readme(),
                format!("{expect}/files/bulk-downloads/data-dump/schedules/README.txt")
            );
            assert_eq!(e.bulk_doc("/a/b.csv"), format!("{expect}/a/b.csv"));
            assert_eq!(
                e.openfec("filings/", "q=1"),
                format!("{expect}/filings/?q=1")
            );
            assert_eq!(e.openfec("filings/", ""), format!("{expect}/filings/"));
            assert_eq!(
                e.efile_rss(),
                format!("{expect}/rss/generate?preDefinedFilingType=ALL")
            );
            assert_eq!(
                e.webcheck("services/upload"),
                format!("{expect}/services/upload")
            );
            // An explicit feed URL wins over the derived one, in that order.
            let explicit = e
                .clone()
                .with_efile_rss_url(Url::parse("https://feeds.example.gov/all.rss").unwrap());
            assert_eq!(explicit.efile_rss(), "https://feeds.example.gov/all.rss");
            let rederived = explicit.with_efiling_apps_base(url.clone());
            assert_eq!(
                rederived.efile_rss(),
                format!("{expect}/rss/generate?preDefinedFilingType=ALL")
            );
        }
    }

    #[test]
    fn docquery_rewrite_for_a_mirror_and_for_the_default() {
        let default = Endpoints::default();
        assert_eq!(
            default.rewrite_docquery(" http://docquery.fec.gov/dcdev/posted/2011407.fec "),
            "https://docquery.fec.gov/dcdev/posted/2011407.fec"
        );
        assert_eq!(
            default.rewrite_docquery("https://docquery.fec.gov/dcdev/posted/1.fec"),
            "https://docquery.fec.gov/dcdev/posted/1.fec"
        );
        assert_eq!(
            default.rewrite_docquery("https://other.example.gov/1.fec"),
            "https://other.example.gov/1.fec"
        );
        // A look-alike host is not the document store.
        assert_eq!(
            default.rewrite_docquery("http://docquery.fec.gov.example.com/1.fec"),
            "http://docquery.fec.gov.example.com/1.fec"
        );
        let mirror = Endpoints::default()
            .with_docquery_base(Url::parse("https://mirror.example.gov/docquery/").unwrap());
        assert_eq!(
            mirror.docquery_filing(2011407),
            "https://mirror.example.gov/docquery/dcdev/posted/2011407.fec"
        );
        assert_eq!(
            format!("{}{}.fec", mirror.docquery_filing_prefix(), 2011407),
            mirror.docquery_filing(2011407)
        );
        assert_eq!(
            mirror.rewrite_docquery("http://docquery.fec.gov/dcdev/posted/2011407.fec"),
            "https://mirror.example.gov/docquery/dcdev/posted/2011407.fec"
        );
        assert_eq!(
            mirror.rewrite_docquery("https://docquery.fec.gov"),
            "https://mirror.example.gov/docquery/"
        );
        assert!(mirror.is_docquery("https://mirror.example.gov/docquery/dcdev/posted/1.fec"));
        assert!(!mirror.is_docquery("https://docquery.fec.gov/dcdev/posted/1.fec"));
        assert!(default.is_docquery("https://docquery.fec.gov/dcdev/posted/1.fec"));
        assert!(!default.is_docquery("https://docquery.fec.gov.example.com/x"));
    }

    #[test]
    fn url_validation() {
        let ok = |s: &str| Url::parse(s).unwrap().as_str().to_string();
        assert_eq!(ok(" https://x.gov/ "), "https://x.gov/");
        assert_eq!(ok("HTTP://x.gov"), "HTTP://x.gov");
        assert_eq!(
            ok("http://user:pw@x.gov:8080/a?b=1"),
            "http://user:pw@x.gov:8080/a?b=1"
        );
        assert_eq!(ok("http://[::1]:8080/"), "http://[::1]:8080/");
        assert_eq!(ok("http://127.0.0.1:9/"), "http://127.0.0.1:9/");

        let reason = |s: &str| match Url::parse(s) {
            Err(EndpointError::InvalidUrl { value, reason }) => format!("{value}|{reason}"),
            other => panic!("{s}: {other:?}"),
        };
        assert_eq!(reason(""), "|it is empty");
        assert_eq!(reason("   "), "|it is empty");
        assert_eq!(
            reason("https://x.gov/a b"),
            "https://x.gov/a b|it contains whitespace or a control character"
        );
        assert_eq!(
            reason("https://x.gov/a\tb"),
            "https://x.gov/a\tb|it contains whitespace or a control character"
        );
        assert_eq!(
            reason("x.gov"),
            "x.gov|it must start with http:// or https://"
        );
        assert_eq!(
            reason("ftp://x.gov"),
            "ftp://x.gov|it must start with http:// or https://"
        );
        assert_eq!(reason("https://"), "https://|it has no host");
        assert_eq!(reason("https:///path"), "https:///path|it has no host");
        assert_eq!(reason("https://:8080"), "https://:8080|it has no host");
        assert_eq!(reason("https://user@"), "https://user@|it has no host");
        assert_eq!(
            reason("https://x.gov/#frag"),
            "https://x.gov/#frag|it must not have a #fragment"
        );

        assert!(matches!(
            Url::parse_base("https://x.gov/?q=1"),
            Err(EndpointError::InvalidUrl {
                reason: "a base URL must not have a ?query",
                ..
            })
        ));
        assert_eq!(
            Url::parse_base("https://x.gov/").unwrap().as_str(),
            "https://x.gov/"
        );
        assert_eq!(
            "https://x.gov".parse::<Url>().unwrap().to_string(),
            "https://x.gov"
        );
        assert_eq!(
            String::from(Url::parse("https://x.gov").unwrap()),
            "https://x.gov"
        );
    }

    fn lookup(pairs: &[(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |var: &str| pairs.iter().find(|(k, _)| k == var).map(|(_, v)| v.clone())
    }

    #[test]
    fn from_lookup_applies_each_variable() {
        let e = Endpoints::from_lookup(lookup(&[
            (Endpoints::WWW_BASE_VAR, "https://www.mirror.gov/"),
            (Endpoints::OPENFEC_BASE_VAR, "http://localhost:9999/v1"),
            (Endpoints::DOCQUERY_BASE_VAR, "https://dq.mirror.gov"),
            (
                Endpoints::WEBCHECK_ENDPOINT_VAR,
                "https://wc.mirror.gov/webcheck/",
            ),
        ]))
        .unwrap();
        assert_eq!(e.www_base.as_str(), "https://www.mirror.gov/");
        assert_eq!(e.openfec_base.as_str(), "http://localhost:9999/v1");
        assert_eq!(e.docquery_base.as_str(), "https://dq.mirror.gov");
        assert_eq!(
            e.webcheck_endpoint.as_str(),
            "https://wc.mirror.gov/webcheck/"
        );
        // Untouched fields keep their defaults.
        assert_eq!(
            e.efiling_apps_base.as_str(),
            Endpoints::DEFAULT_EFILING_APPS_BASE
        );
        assert_eq!(e.efile_rss(), Endpoints::DEFAULT_EFILE_RSS_URL);
        assert_eq!(
            e.openfec("filings/", "api_key=K"),
            "http://localhost:9999/v1/filings/?api_key=K"
        );
        assert_eq!(
            e.docquery_filing(5),
            "https://dq.mirror.gov/dcdev/posted/5.fec"
        );
    }

    #[test]
    fn from_lookup_derives_the_feed_from_the_apps_base_unless_given() {
        let derived = Endpoints::from_lookup(lookup(&[(
            Endpoints::EFILING_APPS_BASE_VAR,
            "https://apps.mirror.gov/",
        )]))
        .unwrap();
        assert_eq!(
            derived.efiling_apps_base.as_str(),
            "https://apps.mirror.gov/"
        );
        assert_eq!(
            derived.efile_rss(),
            "https://apps.mirror.gov/rss/generate?preDefinedFilingType=ALL"
        );

        let explicit = Endpoints::from_lookup(lookup(&[
            (Endpoints::EFILING_APPS_BASE_VAR, "https://apps.mirror.gov"),
            (
                Endpoints::EFILE_RSS_URL_VAR,
                "https://feeds.mirror.gov/all.rss?x=1",
            ),
        ]))
        .unwrap();
        assert_eq!(explicit.efile_rss(), "https://feeds.mirror.gov/all.rss?x=1");

        let only_feed = Endpoints::from_lookup(lookup(&[(
            Endpoints::EFILE_RSS_URL_VAR,
            "http://localhost:8000/feed.xml",
        )]))
        .unwrap();
        assert_eq!(only_feed.efile_rss(), "http://localhost:8000/feed.xml");
        assert_eq!(
            only_feed.efiling_apps_base.as_str(),
            Endpoints::DEFAULT_EFILING_APPS_BASE
        );
    }

    #[test]
    fn from_lookup_treats_blank_as_unset_and_trims() {
        let e = Endpoints::from_lookup(lookup(&[
            (Endpoints::WWW_BASE_VAR, ""),
            (Endpoints::OPENFEC_BASE_VAR, "   "),
            (Endpoints::DOCQUERY_BASE_VAR, "  https://dq.mirror.gov  "),
        ]))
        .unwrap();
        assert_eq!(e.www_base.as_str(), Endpoints::DEFAULT_WWW_BASE);
        assert_eq!(e.openfec_base.as_str(), Endpoints::DEFAULT_OPENFEC_BASE);
        assert_eq!(e.docquery_base.as_str(), "https://dq.mirror.gov");
    }

    #[test]
    fn from_lookup_names_the_variable_on_an_invalid_override() {
        for (var, value, reason) in [
            (
                Endpoints::WWW_BASE_VAR,
                "www.mirror.gov",
                "it must start with http:// or https://",
            ),
            (
                Endpoints::OPENFEC_BASE_VAR,
                "https://api.mirror.gov/v1/?key=1",
                "a base URL must not have a ?query",
            ),
            (Endpoints::DOCQUERY_BASE_VAR, "https://", "it has no host"),
            (
                Endpoints::EFILE_RSS_URL_VAR,
                "https://feeds.mirror.gov/a b",
                "it contains whitespace or a control character",
            ),
            (
                Endpoints::EFILING_APPS_BASE_VAR,
                "ftp://apps.mirror.gov",
                "it must start with http:// or https://",
            ),
            (
                Endpoints::WEBCHECK_ENDPOINT_VAR,
                "https://wc.mirror.gov/#x",
                "it must not have a #fragment",
            ),
        ] {
            let err = Endpoints::from_lookup(lookup(&[(var, value)])).unwrap_err();
            assert_eq!(
                err,
                EndpointError::InvalidOverride {
                    var,
                    value: value.to_string(),
                    reason,
                },
                "{var}"
            );
            assert_eq!(err.var(), Some(var));
            assert_eq!(err.value(), value);
            assert_eq!(err.reason(), reason);
            let text = err.to_string();
            assert!(text.contains(var), "{text}");
            assert!(text.contains(reason), "{text}");
        }
        // Setting the feed URL does not excuse a bad apps base.
        let err = Endpoints::from_lookup(lookup(&[
            (Endpoints::EFILING_APPS_BASE_VAR, "nope"),
            (
                Endpoints::EFILE_RSS_URL_VAR,
                "https://feeds.mirror.gov/all.rss",
            ),
        ]))
        .unwrap_err();
        assert_eq!(err.var(), Some(Endpoints::EFILING_APPS_BASE_VAR));
        assert_eq!(Url::parse("nope").unwrap_err().var(), None);
        assert_eq!(Endpoints::VARS.len(), 6);
    }
}
