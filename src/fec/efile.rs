//! The FEC's electronic-filing feed and daily archives: raw, immediate,
//! no API key.
//!
//! * [`EfileFeed`] polls the RSS feed at [`FEED_URL`], which lists every
//!   e-filing received in the last seven days (roughly 1,000-2,000 items)
//!   within minutes of receipt. Each item names the committee, the form
//!   type, the coverage period, and the raw-filing URL.
//! * [`daily_zip_filings`] reads one of the daily archives at
//!   `https://www.fec.gov/files/bulk-downloads/electronic/YYYYMMDD.zip`
//!   (one per day from 2001-02-01 to yesterday, each holding that day's
//!   e-filings as `<id>.fec`), for backfilling beyond the feed's window.
//!
//! Both describe filings *as received*: before the FEC's processing, so
//! before openFEC's `/filings/` knows about them, and including filings
//! the FEC will later reject.
//!
//! The hosts come from [`Endpoints`]: [`EfileFeed::with_endpoints`] and
//! [`daily_zip_filings_with`] take them explicitly; [`EfileFeed::new`]
//! and [`daily_zip_url`] use the production defaults; [`daily_zip_filings`]
//! reads [`Endpoints::from_env`].

use std::collections::BTreeMap;
use std::io::Read;

use chrono::{DateTime, FixedOffset, NaiveDate};

use super::cache::{Cache, write_atomically};
use super::endpoints::Endpoints;
use super::{FecApiError, Result, agent, get_ok, read_all};

/// The FEC's "all filings" RSS feed: the production default of
/// [`Endpoints::efile_rss_url`].
pub const FEED_URL: &str = Endpoints::DEFAULT_EFILE_RSS_URL;

/// Where the daily archives live in production; the file name is
/// `YYYYMMDD.zip`. [`Endpoints::daily_efile_zip`] builds the URL from the
/// configured base.
pub const DAILY_ZIP_BASE: &str = "https://www.fec.gov/files/bulk-downloads/electronic/";

/// The first day with a daily archive (2001-02-01).
pub const FIRST_DAILY_ZIP: NaiveDate = match NaiveDate::from_ymd_opt(2001, 2, 1) {
    Some(d) => d,
    // Unreachable for a literal valid date; MIN rather than a panic so no
    // panic path exists in library code.
    None => NaiveDate::MIN,
};

/// One entry of the e-file RSS feed.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct FeedItem {
    /// The filing id (`FilingId:` in the description, else from the link).
    pub filing_id: u64,
    /// The form type as filed, with its amendment suffix: `F3XN`, `F3XA`,
    /// `F24N`, ... Empty if the item did not name one.
    pub form_type: String,
    /// `C00123456`-style committee id, when the item carries one.
    pub committee_id: Option<String>,
    /// The committee's name from the item title.
    pub committee_name: Option<String>,
    /// When the FEC published the item (`pubDate`, else `dc:date`).
    pub published: Option<DateTime<FixedOffset>>,
    /// The raw `.fec` URL. The feed says `http://docquery.fec.gov`; the
    /// host is rewritten to the configured document store
    /// ([`Endpoints::rewrite_docquery`]), which at the default base means
    /// `https://docquery.fec.gov` (the store redirects to it anyway).
    pub url: String,
    /// Report type description (`SEPTEMBER MONTHLY`), when given.
    pub report_type: Option<String>,
    /// Coverage period, when the form has one.
    pub coverage_start: Option<NaiveDate>,
    pub coverage_end: Option<NaiveDate>,
}

impl FeedItem {
    /// The form type without its amendment suffix (`F3XA` -> `F3X`), for
    /// matching against a form-type filter.
    #[must_use]
    pub fn base_form_type(&self) -> &str {
        base_form_type(&self.form_type)
    }

    /// True if `filter` names this item's form type either exactly
    /// (`F3XA`) or by base type (`F3X` matches `F3XN`, `F3XA`, `F3XT`).
    #[must_use]
    pub fn matches_form_type(&self, filter: &str) -> bool {
        let f = filter.trim();
        self.form_type.eq_ignore_ascii_case(f) || self.base_form_type().eq_ignore_ascii_case(f)
    }
}

/// `F3XA` -> `F3X`; `F1MN` -> `F1M`; `F99` -> `F99`. Only a single
/// trailing `N`/`A`/`T` is an amendment marker.
#[must_use]
pub fn base_form_type(form_type: &str) -> &str {
    let t = form_type.trim();
    match t.as_bytes().last() {
        Some(b'N' | b'A' | b'T' | b'n' | b'a' | b't') if t.len() > 2 => {
            t.get(..t.len() - 1).unwrap_or(t)
        }
        _ => t,
    }
}

/// A client for the e-file RSS feed.
#[derive(Debug, Clone)]
pub struct EfileFeed {
    url: String,
    /// For rewriting each item's `docquery.fec.gov` link.
    endpoints: Endpoints,
}

impl Default for EfileFeed {
    fn default() -> Self {
        Self::new()
    }
}

impl EfileFeed {
    /// The feed at [`FEED_URL`], with item links on the production
    /// document store. Does not read the environment; see
    /// [`EfileFeed::with_endpoints`].
    #[must_use]
    pub fn new() -> Self {
        Self::with_endpoints(&Endpoints::default())
    }

    /// The feed at [`Endpoints::efile_rss`], with each item's link
    /// rewritten to [`Endpoints::docquery_base`].
    #[must_use]
    pub fn with_endpoints(endpoints: &Endpoints) -> Self {
        EfileFeed {
            url: endpoints.efile_rss().to_string(),
            endpoints: endpoints.clone(),
        }
    }

    /// A feed at another URL (a mirror, or a fixture served locally);
    /// item links point at the production document store.
    pub fn with_url(url: impl Into<String>) -> Self {
        EfileFeed {
            url: url.into(),
            endpoints: Endpoints::default(),
        }
    }

    /// The URL polled.
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// Downloads the feed and returns its items, newest first as the FEC
    /// orders them. Items without a filing id are skipped.
    ///
    /// Fails with [`FecApiError::Http`]/[`FecApiError::Transport`] if the
    /// feed cannot be fetched and [`FecApiError::Rss`] if the body is not
    /// an RSS document.
    pub fn poll(&self) -> Result<Vec<FeedItem>> {
        let response = get_ok(agent(), &self.url)?;
        let bytes = read_all(response)?;
        parse_feed_with(&decode_feed(&bytes), &self.endpoints)
    }
}

/// The feed declares UTF-8 but is served as ISO-8859-1; take UTF-8 when
/// it is valid and fall back to Windows-1252 otherwise, like the parser.
fn decode_feed(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => encoding_rs::WINDOWS_1252.decode(bytes).0.into_owned(),
    }
}

/// Parses the feed's XML into items, with item links on the production
/// document store. Exposed so a captured feed can be parsed without the
/// network. See [`parse_feed_with`].
pub fn parse_feed(xml: &str) -> Result<Vec<FeedItem>> {
    parse_feed_with(xml, &Endpoints::default())
}

/// Parses the feed's XML into items, rewriting each item's
/// `docquery.fec.gov` link to `endpoints.docquery_base`
/// ([`Endpoints::rewrite_docquery`]).
///
/// The FEC's feed is plain RSS 2.0 with fixed element names, so this is
/// a small tolerant extractor rather than a full XML parser: it finds
/// each `<item>`, reads the text of `title`, `link`, `description`,
/// `pubDate`, and `dc:date` (decoding entities and CDATA), and pulls the
/// `CommitteeId | FilingId | FormType | CoverageFrom | CoverageThrough |
/// ReportType` trailer out of the description.
///
/// Fails with [`FecApiError::Rss`] if there is no `<rss>`/`<channel>`
/// element at all (an error page, say). Items without a filing id are
/// skipped, not fatal.
pub fn parse_feed_with(xml: &str, endpoints: &Endpoints) -> Result<Vec<FeedItem>> {
    if !(xml.contains("<rss") || xml.contains("<channel")) {
        let head: String = xml.chars().take(120).collect();
        return Err(FecApiError::Rss(format!(
            "no <rss> or <channel> element; the document starts: {}",
            super::collapse_whitespace(&head)
        )));
    }
    let mut items = Vec::new();
    let mut rest = xml;
    while let Some(start) = find_open_tag(rest, "item") {
        let after_open = &rest[start..];
        let body_start = after_open.find('>').map_or(after_open.len(), |i| i + 1);
        let body = &after_open[body_start..];
        let (item_xml, remaining) = match body.find("</item>") {
            Some(end) => (&body[..end], &body[end + "</item>".len()..]),
            None => (body, ""),
        };
        if let Some(item) = parse_item(item_xml, endpoints) {
            items.push(item);
        }
        rest = remaining;
    }
    Ok(items)
}

fn parse_item(item: &str, endpoints: &Endpoints) -> Option<FeedItem> {
    let title = element_text(item, "title");
    let link = element_text(item, "link").unwrap_or_default();
    let description = element_text(item, "description").unwrap_or_default();
    let fields = trailer_fields(&description);

    let filing_id = fields
        .get("FilingId")
        .and_then(|v| v.parse::<u64>().ok())
        .or_else(|| id_from_url(&link))?;
    let form_type = fields
        .get("FormType")
        .filter(|s| !s.is_empty())
        .cloned()
        .or_else(|| form_type_from_prose(&description))
        .unwrap_or_default();
    let committee_id = fields.get("CommitteeId").filter(|s| !s.is_empty()).cloned();
    let committee_name = title
        .as_deref()
        .map(|t| t.strip_prefix("New filing by ").unwrap_or(t))
        .map(|t| decode_entities_lenient(t.trim()))
        .filter(|s| !s.is_empty());
    let published = element_text(item, "pubDate")
        .and_then(|d| DateTime::parse_from_rfc2822(d.trim()).ok())
        .or_else(|| {
            element_text(item, "dc:date").and_then(|d| DateTime::parse_from_rfc3339(d.trim()).ok())
        });
    let url = if link.trim().is_empty() {
        endpoints.docquery_filing(filing_id)
    } else {
        endpoints.rewrite_docquery(&link)
    };
    Some(FeedItem {
        filing_id,
        form_type,
        committee_id,
        committee_name,
        published,
        url,
        report_type: fields.get("ReportType").filter(|s| !s.is_empty()).cloned(),
        coverage_start: fields.get("CoverageFrom").and_then(|s| parse_mdy(s)),
        coverage_end: fields.get("CoverageThrough").and_then(|s| parse_mdy(s)),
    })
}

/// `CommitteeId: C00961573 | FilingId: 2011915 | FormType: F1N | ...`
/// as a map. The trailer is wrapped in runs of `*`; anything before the
/// first known key (the HTML sentence) is ignored.
fn trailer_fields(description: &str) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    let Some(start) = description
        .find("CommitteeId:")
        .or_else(|| description.find("FilingId:"))
    else {
        return fields;
    };
    let trailer = description[start..].trim_end_matches(|c: char| c == '*' || c.is_whitespace());
    for part in trailer.split('|') {
        if let Some((key, value)) = part.split_once(':') {
            fields.insert(key.trim().to_string(), value.trim().to_string());
        }
    }
    fields
}

/// Fallback when the trailer is missing: "successfully filed  their F3XN
/// SEPTEMBER MONTHLY ..." names the form type after "their".
fn form_type_from_prose(description: &str) -> Option<String> {
    let after = description.split("their ").nth(1)?;
    let token: String = after
        .trim_start()
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric())
        .collect();
    (token.starts_with('F') && token.len() >= 2).then_some(token)
}

/// `.../posted/2011915.fec` -> 2011915.
fn id_from_url(url: &str) -> Option<u64> {
    let file = url.trim().rsplit('/').next()?;
    file.strip_suffix(".fec")
        .unwrap_or(file)
        .parse::<u64>()
        .ok()
}

/// `MM/DD/YYYY` as the feed writes coverage dates.
fn parse_mdy(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%m/%d/%Y").ok()
}

/// Byte offset of `<tag>` or `<tag ...>` in `xml`, ignoring `<tagger>`.
fn find_open_tag(xml: &str, tag: &str) -> Option<usize> {
    let mut from = 0;
    while let Some(rel) = xml[from..].find('<') {
        let pos = from + rel;
        let after = &xml[pos + 1..];
        if let Some(rest) = after.strip_prefix(tag) {
            match rest.chars().next() {
                Some('>') | Some(' ') | Some('\t') | Some('\n') | Some('\r') | Some('/') => {
                    return Some(pos);
                }
                _ => {}
            }
        }
        from = pos + 1;
    }
    None
}

/// The decoded text content of the first `<tag>` element in `xml`, or
/// `None` if there is none (or it is self-closing).
fn element_text(xml: &str, tag: &str) -> Option<String> {
    let start = find_open_tag(xml, tag)?;
    let after = &xml[start..];
    let open_end = after.find('>')?;
    if after[..open_end].ends_with('/') {
        return None;
    }
    let body = &after[open_end + 1..];
    let close = format!("</{tag}>");
    let raw = match body.find(&close) {
        Some(end) => &body[..end],
        None => body,
    };
    Some(xml_unescape(raw))
}

/// Decodes XML character data: CDATA sections verbatim, the five
/// predefined entities, and numeric character references. Unknown or
/// malformed references are kept as written.
fn xml_unescape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(pos) = rest.find("<![CDATA[") {
        out.push_str(&decode_entities_lenient(&rest[..pos]));
        let cdata = &rest[pos + "<![CDATA[".len()..];
        match cdata.find("]]>") {
            Some(end) => {
                out.push_str(&cdata[..end]);
                rest = &cdata[end + "]]>".len()..];
            }
            None => {
                out.push_str(cdata);
                rest = "";
            }
        }
    }
    out.push_str(&decode_entities_lenient(rest));
    out
}

/// Decodes `&amp; &lt; &gt; &quot; &apos; &#NN; &#xHH;`, leaving anything
/// else (including a bare `&`) alone. Applied twice to committee names
/// because the feed double-encodes them (`AMERICA&amp;#39;S`).
pub(crate) fn decode_entities_lenient(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(pos) = rest.find('&') {
        out.push_str(&rest[..pos]);
        let after = &rest[pos..];
        let Some(semi) = after.find(';').filter(|&i| i <= 12) else {
            out.push('&');
            rest = &after[1..];
            continue;
        };
        let entity = &after[1..semi];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity.strip_prefix('#').and_then(|num| {
                let code = match num.strip_prefix(['x', 'X']) {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => num.parse::<u32>().ok(),
                }?;
                char::from_u32(code)
            }),
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &after[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &after[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The URL of the daily archive for `date` on the production host:
/// [`Endpoints::daily_efile_zip`] at the default base.
#[must_use]
pub fn daily_zip_url(date: NaiveDate) -> String {
    Endpoints::default().daily_efile_zip(date)
}

/// [`daily_zip_filings_with`] at [`Endpoints::from_env`]. Fails with
/// [`FecApiError::Endpoint`] if a `HARDMONEY_*` override is malformed.
pub fn daily_zip_filings(date: NaiveDate, cache: &Cache) -> Result<DailyZipFilings> {
    daily_zip_filings_with(date, cache, &Endpoints::from_env()?)
}

/// Downloads (unless cached) the daily e-filing archive for `date` from
/// `endpoints.www_base` and returns an iterator over its filings as
/// `(filing_id, bytes)`.
///
/// The archive is kept at `<cache>/efile/YYYYMMDD.zip`. Entries whose
/// name is not `<digits>.fec` are skipped.
///
/// Fails with [`FecApiError::InvalidQuery`] for a date before
/// 2001-02-01 or after today (UTC), [`FecApiError::Http`] if the FEC has
/// no archive for the date (today's is published tomorrow),
/// [`FecApiError::Zip`] if the file is not a zip, and
/// [`FecApiError::Io`] on cache trouble. Individual entries that fail to
/// read come back as `Err` items; the iterator continues past them.
pub fn daily_zip_filings_with(
    date: NaiveDate,
    cache: &Cache,
    endpoints: &Endpoints,
) -> Result<DailyZipFilings> {
    let today = chrono::Utc::now().date_naive();
    if date < FIRST_DAILY_ZIP || date > today {
        return Err(FecApiError::InvalidQuery(format!(
            "no daily e-filing archive exists for {date}; the FEC publishes one per day from \
             {FIRST_DAILY_ZIP} through yesterday"
        )));
    }
    let path = cache.daily_zip_path(date);
    if !path.exists() {
        let url = endpoints.daily_efile_zip(date);
        let response = get_ok(agent(), &url)?;
        let mut reader = response.into_body().into_reader();
        write_atomically(&path, |file| std::io::copy(&mut reader, file).map(|_| ()))?;
    }
    let file = std::fs::File::open(&path)?;
    let archive = zip::ZipArchive::new(file).map_err(|source| FecApiError::Zip {
        path: path.clone(),
        source,
    })?;
    Ok(DailyZipFilings {
        archive,
        path,
        next: 0,
    })
}

/// Iterator returned by [`daily_zip_filings`].
pub struct DailyZipFilings {
    archive: zip::ZipArchive<std::fs::File>,
    path: std::path::PathBuf,
    next: usize,
}

impl std::fmt::Debug for DailyZipFilings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DailyZipFilings")
            .field("path", &self.path)
            .field("entries", &self.archive.len())
            .field("next", &self.next)
            .finish()
    }
}

impl DailyZipFilings {
    /// Where the archive is cached.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Number of entries in the archive (including any that are not
    /// filings).
    #[must_use]
    pub fn len(&self) -> usize {
        self.archive.len()
    }

    /// True if the archive has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.archive.is_empty()
    }
}

impl Iterator for DailyZipFilings {
    type Item = Result<(u64, Vec<u8>)>;

    fn next(&mut self) -> Option<Self::Item> {
        while self.next < self.archive.len() {
            let index = self.next;
            self.next += 1;
            let mut entry = match self.archive.by_index(index) {
                Ok(e) => e,
                Err(source) => {
                    return Some(Err(FecApiError::Zip {
                        path: self.path.clone(),
                        source,
                    }));
                }
            };
            if !entry.is_file() {
                continue;
            }
            let Some(id) = id_from_url(entry.name()) else {
                continue;
            };
            let mut bytes = Vec::with_capacity(usize::try_from(entry.size()).unwrap_or(0));
            return Some(match entry.read_to_end(&mut bytes) {
                Ok(_) => Ok((id, bytes)),
                Err(e) => Err(FecApiError::Io(e)),
            });
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ITEM: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss xmlns:dc="http://purl.org/dc/elements/1.1/" version="2.0">
  <channel>
    <title>FEC Electronic Filing RSS Feed - ALL</title>
    <item>
      <title>New filing by AMERICA&amp;#39;S CREDIT UNIONS</title>
      <link>http://docquery.fec.gov/dcdev/posted/2011407.fec</link>
      <description>&lt;p&gt;The AMERICA&amp;#39;S CREDIT UNIONS successfully filed  their F5N  with the coverage period of 09/08/2026 to 09/10/2026 and a confirmation ID of FEC-2011407&lt;/p&gt;*********CommitteeId: C90023565 | FilingId: 2011407 | FormType: F5N | CoverageFrom: 09/08/2026 | CoverageThrough: 09/10/2026 | ReportType: *********</description>
      <pubDate>Fri, 11 Sep 2026 14:02:10 GMT</pubDate>
      <guid>http://docquery.fec.gov/dcdev/posted/2011407.fec</guid>
      <dc:date>2026-09-11T14:02:10Z</dc:date>
    </item>
    <item>
      <title><![CDATA[New filing by HEALTH & FITNESS PAC]]></title>
      <link>http://docquery.fec.gov/dcdev/posted/2011914.fec</link>
      <description>no trailer here</description>
    </item>
    <item>
      <title>broken: no id anywhere</title>
      <link>http://docquery.fec.gov/dcdev/</link>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn parses_items_and_skips_the_unusable() {
        let items = parse_feed(ITEM).unwrap();
        assert_eq!(items.len(), 2);
        let a = &items[0];
        assert_eq!(a.filing_id, 2011407);
        assert_eq!(a.form_type, "F5N");
        assert_eq!(a.base_form_type(), "F5");
        assert_eq!(a.committee_id.as_deref(), Some("C90023565"));
        assert_eq!(a.committee_name.as_deref(), Some("AMERICA'S CREDIT UNIONS"));
        assert_eq!(a.url, "https://docquery.fec.gov/dcdev/posted/2011407.fec");
        assert_eq!(
            a.published.unwrap().to_rfc3339(),
            "2026-09-11T14:02:10+00:00"
        );
        assert_eq!(a.report_type, None);
        assert_eq!(
            a.coverage_start,
            Some(NaiveDate::from_ymd_opt(2026, 9, 8).unwrap())
        );
        assert_eq!(
            a.coverage_end,
            Some(NaiveDate::from_ymd_opt(2026, 9, 10).unwrap())
        );
        assert!(a.matches_form_type("F5"));
        assert!(a.matches_form_type("f5n"));
        assert!(!a.matches_form_type("F5A"));

        let b = &items[1];
        assert_eq!(b.filing_id, 2011914);
        assert_eq!(b.form_type, "");
        assert_eq!(b.committee_name.as_deref(), Some("HEALTH & FITNESS PAC"));
        assert_eq!(b.published, None);
    }

    #[test]
    fn rejects_non_rss() {
        let err = parse_feed("<html><body>503 Service Unavailable</body></html>").unwrap_err();
        assert!(matches!(err, FecApiError::Rss(_)), "{err}");
        assert!(err.to_string().contains("503"));
    }

    #[test]
    fn entity_decoding_is_lenient() {
        assert_eq!(decode_entities_lenient("A &amp; B"), "A & B");
        assert_eq!(decode_entities_lenient("A & B"), "A & B");
        assert_eq!(
            decode_entities_lenient("&#39;&#x41;&lt;&gt;&quot;&apos;"),
            "'A<>\"'"
        );
        assert_eq!(
            decode_entities_lenient("&bogus; &#zz; &"),
            "&bogus; &#zz; &"
        );
        assert_eq!(xml_unescape("x<![CDATA[<&>]]>&amp;y"), "x<&>&y");
    }

    #[test]
    fn base_form_types() {
        assert_eq!(base_form_type("F3XA"), "F3X");
        assert_eq!(base_form_type("F3XT"), "F3X");
        assert_eq!(base_form_type("F1MN"), "F1M");
        assert_eq!(base_form_type("F99"), "F99");
        assert_eq!(base_form_type("F24N"), "F24");
        assert_eq!(base_form_type("F1"), "F1");
        assert_eq!(base_form_type(""), "");
    }

    #[test]
    fn open_tag_matching_ignores_prefixes() {
        assert_eq!(find_open_tag("<items><item>", "item"), Some(7));
        assert_eq!(find_open_tag("<item x='1'>", "item"), Some(0));
        assert_eq!(find_open_tag("<itemx>", "item"), None);
        assert_eq!(element_text("<a><title/></a>", "title"), None);
        assert_eq!(
            element_text("<a><title>T &amp; U</title></a>", "title").as_deref(),
            Some("T & U")
        );
    }

    #[test]
    fn daily_zip_rejects_dates_outside_the_archive() {
        let cache = Cache::at(std::env::temp_dir().join("hardmoney-never-created"));
        let endpoints = Endpoints::default();
        let early = NaiveDate::from_ymd_opt(2001, 1, 31).unwrap();
        assert!(matches!(
            daily_zip_filings_with(early, &cache, &endpoints),
            Err(FecApiError::InvalidQuery(_))
        ));
        let future = NaiveDate::from_ymd_opt(2200, 1, 1).unwrap();
        assert!(matches!(
            daily_zip_filings_with(future, &cache, &endpoints),
            Err(FecApiError::InvalidQuery(_))
        ));
        let day = NaiveDate::from_ymd_opt(2026, 9, 6).unwrap();
        assert_eq!(
            daily_zip_url(day),
            "https://www.fec.gov/files/bulk-downloads/electronic/20260906.zip"
        );
        // The legacy constant and the centralised default agree.
        assert_eq!(daily_zip_url(day), format!("{DAILY_ZIP_BASE}20260906.zip"));
        assert_eq!(FEED_URL, EfileFeed::new().url());
    }

    /// Feed-driven downloads follow a mirror: every item link is rebased
    /// onto the configured document store, and an item without a link
    /// gets the configured template.
    #[test]
    fn item_links_follow_the_configured_docquery_base() {
        let mirror = Endpoints::default()
            .with_docquery_base(super::super::Url::parse("https://mirror.example.gov/dq").unwrap());
        let items = parse_feed_with(ITEM, &mirror).unwrap();
        assert_eq!(
            items[0].url,
            "https://mirror.example.gov/dq/dcdev/posted/2011407.fec"
        );
        assert_eq!(
            items[1].url,
            "https://mirror.example.gov/dq/dcdev/posted/2011914.fec"
        );
        let no_link = r#"<rss><channel><item>
            <description>FilingId: 42 | FormType: F3XN</description>
        </item></channel></rss>"#;
        let items = parse_feed_with(no_link, &mirror).unwrap();
        assert_eq!(
            items[0].url,
            "https://mirror.example.gov/dq/dcdev/posted/42.fec"
        );
        let feed = EfileFeed::with_endpoints(&mirror);
        assert_eq!(feed.url(), FEED_URL);
        assert_eq!(feed.endpoints, mirror);
    }
}
