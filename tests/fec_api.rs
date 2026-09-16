//! The `fec` module: openFEC response decoding from captured responses,
//! RSS feed parsing from a captured feed, query-string building and key
//! redaction, cache path resolution, and `Retry-After` parsing.
//!
//! Network tests are `#[ignore]` and additionally self-skip unless
//! `HARDMONEY_NETWORK_TESTS=1` (they also need an API key in
//! `FEC_API_KEY` or `~/fec_api_key.txt`):
//!
//! ```text
//! HARDMONEY_NETWORK_TESTS=1 cargo test --all-features --test fec_api -- --ignored
//! ```

#![cfg(feature = "fetch")]

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::NaiveDate;
use hardmoney::fec::efile::{EfileFeed, FeedItem, parse_feed};
use hardmoney::fec::{Cache, FecApiError, parse_retry_after, redact_url};

fn fixture(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/openfec")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn network_tests_enabled() -> bool {
    if std::env::var("HARDMONEY_NETWORK_TESTS").as_deref() == Ok("1") {
        true
    } else {
        eprintln!("skipping: set HARDMONEY_NETWORK_TESTS=1 to run network tests");
        false
    }
}

fn ymd(y: i32, m: u32, d: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(y, m, d).unwrap()
}

// ---------------------------------------------------------------------
// openFEC response decoding (needs serde)
// ---------------------------------------------------------------------

#[cfg(feature = "serde")]
mod openfec {
    use super::*;
    use hardmoney::fec::openfec::{
        ApiKey, EfileQuery, EfileRecord, FilingRecord, FilingsQuery, OpenFec, OperationsLogQuery,
        OperationsLogRecord, Page, ProcessingStage, ProcessingStatus,
    };
    use rust_decimal_macros::dec;

    /// A one-shot local HTTP server that answers each request from a
    /// table of `(path prefix, JSON body)` pairs, 404 otherwise, for
    /// `connections` connections. Returns the request lines it saw.
    fn serve_fixtures(
        routes: Vec<(&'static str, String)>,
        connections: usize,
    ) -> (std::net::SocketAddr, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for _ in 0..connections {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = vec![0u8; 16384];
                let n = stream.read(&mut buf).unwrap();
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let line = request.lines().next().unwrap_or("").to_string();
                let path = line.split(' ').nth(1).unwrap_or("");
                let (status, body) = match routes.iter().find(|(p, _)| path.starts_with(p)) {
                    Some((_, body)) => ("200 OK", body.clone()),
                    None => ("404 Not Found", "{\"message\":\"no route\"}".to_string()),
                };
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                )
                .unwrap();
                seen.push(line);
            }
            seen
        });
        (addr, handle)
    }

    #[test]
    fn decodes_the_operations_log() {
        let page: Page<OperationsLogRecord> =
            serde_json::from_str(&fixture("operations_log_c00140855_2026.json")).unwrap();
        assert_eq!(page.pagination.count, 8);
        assert_eq!(page.results.len(), 8);
        // The September monthly, received 2026-09-14: summary loaded the
        // same day, transactions still pending when captured.
        let m9 = page
            .results
            .iter()
            .find(|r| r.report_type.as_deref() == Some("M9"))
            .unwrap();
        assert_eq!(m9.sub_id, Some(4091420261598294066));
        assert_eq!(m9.candidate_committee_id.as_deref(), Some("C00140855"));
        assert_eq!(m9.form_type.as_deref(), Some("F3X"));
        assert_eq!(m9.report_year, Some(2026));
        assert_eq!(m9.amendment_indicator.as_deref(), Some("N"));
        assert_eq!(
            m9.beginning_image_number.as_deref(),
            Some("202609149904202705")
        );
        assert_eq!(m9.receipt_date.unwrap().date(), ymd(2026, 9, 14));
        assert_eq!(m9.coverage_start_date, Some(ymd(2026, 8, 1)));
        assert_eq!(m9.coverage_end_date, Some(ymd(2026, 8, 31)));
        assert_eq!(m9.status_num, Some(1));
        assert_eq!(
            m9.summary_data_complete_date.unwrap().date(),
            ymd(2026, 9, 14)
        );
        assert_eq!(m9.transaction_data_complete_date, None);
        // The August monthly took 14 days to the transaction load.
        let m8 = page
            .results
            .iter()
            .find(|r| r.report_type.as_deref() == Some("M8"))
            .unwrap();
        assert_eq!(m8.receipt_date.unwrap().date(), ymd(2026, 8, 17));
        assert_eq!(m8.transaction_data_complete_date, Some(ymd(2026, 8, 31)));
    }

    #[test]
    fn operations_log_and_batch_queries_encode_their_filters() {
        let q = OperationsLogQuery::new()
            .committee_id("C00140855")
            .report_year(2026)
            .form_type("F3X")
            .beginning_image_numbers(["1", "2"])
            .per_page(100);
        assert_eq!(
            q.query_string().unwrap(),
            "candidate_committee_id=C00140855&form_type=F3X&report_year=2026\
             &beginning_image_number=1&beginning_image_number=2&per_page=100&page=1"
        );
        assert!(
            OperationsLogQuery::new()
                .per_page(0)
                .query_string()
                .is_err()
        );
        assert_eq!(
            FilingsQuery::new()
                .file_numbers([3, 4])
                .file_number(5)
                .sort(None::<String>)
                .query_string()
                .unwrap(),
            "file_number=3&file_number=4&file_number=5&per_page=100&page=1"
        );
        assert_eq!(
            EfileQuery::new()
                .file_number(1)
                .file_numbers([2])
                .sort(None::<String>)
                .query_string()
                .unwrap(),
            "file_number=1&file_number=2&per_page=100&page=1"
        );
    }

    /// `processing_status` joins the three endpoints on file number and
    /// image number: served from the captured responses for FirstEnergy
    /// PAC's August and September 2026 monthlies, plus an id nobody knows.
    #[test]
    fn processing_status_joins_efile_filings_and_the_operations_log() {
        let (addr, handle) = serve_fixtures(
            vec![
                (
                    "/v1/efile/filings/",
                    fixture("efile_filings_by_file_number.json"),
                ),
                ("/v1/filings/", fixture("filings_by_file_number.json")),
                (
                    "/v1/operations-log/",
                    fixture("operations_log_c00140855_2026.json"),
                ),
            ],
            3,
        );
        let client =
            OpenFec::new(ApiKey::new("K").unwrap()).with_base_url(format!("http://{addr}/v1/"));
        let statuses = client
            .processing_status(&[2011831, 2006786, 999_999_999])
            .unwrap();
        let seen = handle.join().unwrap();
        assert_eq!(seen.len(), 3, "{seen:?}");
        assert!(seen[0].starts_with("GET /v1/efile/filings/?api_key=K&file_number=2011831&file_number=2006786&file_number=999999999"), "{}", seen[0]);
        assert!(
            seen[1].starts_with("GET /v1/filings/?api_key=K&file_number=2011831"),
            "{}",
            seen[1]
        );
        assert!(
            seen[2].contains("beginning_image_number=202609149904202705")
                && seen[2].contains("beginning_image_number=202608179899532184"),
            "{}",
            seen[2]
        );
        assert_eq!(statuses.len(), 3);

        let m9: &ProcessingStatus = &statuses[0];
        assert_eq!(m9.filing_id, 2011831);
        assert!(m9.in_efile && m9.in_filings && m9.in_operations_log);
        assert_eq!(m9.committee_id.as_deref(), Some("C00140855"));
        assert_eq!(m9.report_type.as_deref(), Some("M9"));
        assert_eq!(m9.sub_id.as_deref(), Some("4091420261598294066"));
        assert_eq!(m9.cycle, Some(2026));
        // The efile timestamp wins over /filings/' midnight date.
        assert_eq!(m9.received.unwrap().to_string(), "2026-09-14 15:50:19");
        assert_eq!(
            m9.fec_url.as_deref(),
            Some("https://docquery.fec.gov/dcdev/posted/2011831.fec")
        );
        assert_eq!(m9.summary_lag_days(), Some(0));
        assert_eq!(m9.transaction_lag_days(), None);
        assert_eq!(m9.stage(), ProcessingStage::SummaryLoaded);
        assert_eq!(m9.days_pending(ymd(2026, 9, 15)), Some(1));
        assert_eq!(m9.days_pending(ymd(2026, 9, 14)), Some(0));

        let m8 = &statuses[1];
        assert_eq!(m8.filing_id, 2006786);
        assert_eq!(m8.received.unwrap().to_string(), "2026-08-17 11:43:42");
        assert_eq!(m8.summary_data_complete.unwrap().date(), ymd(2026, 8, 17));
        assert_eq!(m8.transaction_data_complete, Some(ymd(2026, 8, 31)));
        assert_eq!(m8.summary_lag_days(), Some(0));
        assert_eq!(m8.transaction_lag_days(), Some(14));
        assert_eq!(m8.stage(), ProcessingStage::TransactionsLoaded);
        assert_eq!(m8.days_pending(ymd(2026, 9, 15)), None);
        assert_eq!(
            m8.ending_image_number.as_deref(),
            Some("202608179899532234")
        );

        let unknown = &statuses[2];
        assert_eq!(unknown.filing_id, 999_999_999);
        assert!(!unknown.in_efile && !unknown.in_filings && !unknown.in_operations_log);
        assert_eq!(unknown.stage(), ProcessingStage::Unknown);
        assert_eq!(unknown.received, None);
        assert_eq!(unknown.summary_lag_days(), None);
        assert_eq!(unknown.days_pending(ymd(2026, 9, 15)), None);

        // The status serialises for the API route and `lag --json`.
        let v: serde_json::Value = serde_json::to_value(m8).unwrap();
        assert_eq!(v["transaction_data_complete"], "2026-08-31");
        assert_eq!(v["in_filings"], true);
    }

    /// `resolve_fec_url` takes the efile URL when there is one and only
    /// then asks `/filings/`.
    #[test]
    fn resolve_fec_url_prefers_the_efile_record() {
        let (addr, handle) = serve_fixtures(
            vec![(
                "/v1/efile/filings/",
                fixture("efile_filings_by_file_number.json"),
            )],
            1,
        );
        let client =
            OpenFec::new(ApiKey::new("K").unwrap()).with_base_url(format!("http://{addr}/v1/"));
        let url = client.resolve_fec_url(2011831).unwrap();
        handle.join().unwrap();
        // The fixture holds two records; the first with a URL is taken.
        assert!(
            url.as_deref()
                .is_some_and(|u| u.starts_with("https://docquery.fec.gov/dcdev/posted/")),
            "{url:?}"
        );

        // Nothing in efile, nothing in filings: None, after two requests.
        let empty = r#"{"results":[],"pagination":{"page":1,"pages":1,"count":0}}"#.to_string();
        let (addr, handle) = serve_fixtures(
            vec![
                ("/v1/efile/filings/", empty.clone()),
                ("/v1/filings/", empty),
            ],
            2,
        );
        let client =
            OpenFec::new(ApiKey::new("K").unwrap()).with_base_url(format!("http://{addr}/v1/"));
        assert_eq!(client.resolve_fec_url(7).unwrap(), None);
        assert_eq!(handle.join().unwrap().len(), 2);
    }

    /// The same local server reached through `Endpoints` instead of
    /// `with_base_url`: the two set the same value, and the request goes
    /// to the configured root.
    #[test]
    fn endpoints_point_the_client_at_a_local_server() {
        use hardmoney::fec::{Endpoints, Url};

        let (addr, handle) = serve_fixtures(
            vec![(
                "/v1/efile/filings/",
                fixture("efile_filings_by_file_number.json"),
            )],
            1,
        );
        let endpoints = Endpoints::default()
            .with_openfec_base(Url::parse(&format!("http://{addr}/v1")).unwrap());
        let client = OpenFec::new(ApiKey::new("K").unwrap()).with_endpoints(&endpoints);
        assert_eq!(client.base_url(), format!("http://{addr}/v1/"));
        assert_eq!(
            client.base_url(),
            OpenFec::new(ApiKey::new("K").unwrap())
                .with_base_url(format!("http://{addr}/v1"))
                .base_url()
        );
        assert_eq!(
            client.url("efile/filings/", "page=1"),
            endpoints.openfec("efile/filings/", "api_key=REDACTED&page=1")
        );
        let url = client.resolve_fec_url(2011831).unwrap();
        let seen = handle.join().unwrap();
        assert!(
            seen[0].starts_with("GET /v1/efile/filings/?api_key=K&"),
            "{seen:?}"
        );
        assert!(
            url.as_deref()
                .is_some_and(|u| u.starts_with("https://docquery.fec.gov/dcdev/posted/")),
            "{url:?}"
        );
    }

    /// A raw-filing download follows the configured document store: the
    /// production `docquery` URL openFEC or the feed hands out is rebased
    /// onto `docquery_base`, and the bytes land in the cache.
    #[test]
    fn fetch_filing_bytes_with_rebases_hints_onto_the_configured_docquery() {
        use hardmoney::fec::{Endpoints, Url, download_filing_bytes, fetch_filing_bytes_with};

        const FEC: &str = "HDR\u{1c}FEC\u{1c}8.5\u{1c}test\r\n";
        let (addr, handle) = serve_fixtures(vec![("/dq/dcdev/posted/", FEC.to_string())], 2);
        let mirror = Endpoints::default()
            .with_docquery_base(Url::parse(&format!("http://{addr}/dq/")).unwrap());
        let root =
            std::env::temp_dir().join(format!("hardmoney-fec-api-mirror-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let cache = Cache::at(&root);

        // The hint names production; the request goes to the mirror.
        let bytes = fetch_filing_bytes_with(
            2011407,
            &cache,
            Some("http://docquery.fec.gov/dcdev/posted/2011407.fec"),
            &mirror,
        )
        .unwrap();
        assert_eq!(bytes, FEC.as_bytes());
        assert_eq!(
            cache.read_filing(2011407).unwrap().as_deref(),
            Some(FEC.as_bytes())
        );
        // Cached now: no request.
        let again = fetch_filing_bytes_with(2011407, &cache, None, &mirror).unwrap();
        assert_eq!(again, FEC.as_bytes());

        // The direct, uncached download uses the template at the mirror.
        assert_eq!(download_filing_bytes(99, &mirror).unwrap(), FEC.as_bytes());

        let seen = handle.join().unwrap();
        assert_eq!(
            seen,
            [
                "GET /dq/dcdev/posted/2011407.fec HTTP/1.1",
                "GET /dq/dcdev/posted/99.fec HTTP/1.1",
            ]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn decodes_an_amendment_chain_page() {
        let page: Page<FilingRecord> =
            serde_json::from_str(&fixture("filings_c00554709.json")).unwrap();
        assert_eq!(page.api_version.as_deref(), Some("1.0"));
        assert_eq!(page.pagination.count, 3);
        assert_eq!(page.pagination.page, 1);
        assert_eq!(page.pagination.pages, 1);
        assert_eq!(page.pagination.per_page, 5);
        assert_eq!(page.pagination.is_count_exact, Some(true));
        assert!(!page.has_more());
        assert_eq!(page.results.len(), 3);

        // Newest (the second amendment) first.
        let latest = &page.results[0];
        assert_eq!(latest.file_number, Some(1151343));
        assert_eq!(latest.fec_file_id.as_deref(), Some("FEC-1151343"));
        assert_eq!(latest.form_type.as_deref(), Some("F3"));
        assert_eq!(latest.report_type.as_deref(), Some("12G"));
        assert_eq!(latest.report_type_full.as_deref(), Some("PRE-GENERAL"));
        assert_eq!(latest.report_year, Some(2016));
        assert_eq!(latest.cycle, Some(2016));
        assert_eq!(latest.committee_id.as_deref(), Some("C00554709"));
        assert_eq!(
            latest.committee_name.as_deref(),
            Some("MARK DESAULNIER FOR CONGRESS")
        );
        assert_eq!(latest.candidate_id, None);
        assert_eq!(latest.amendment_indicator.as_deref(), Some("A"));
        assert_eq!(latest.amendment_chain, vec![1118027, 1131084, 1151343]);
        assert_eq!(latest.amendment_version, Some(2));
        assert_eq!(latest.most_recent, Some(true));
        assert_eq!(latest.most_recent_file_number, Some(1151343));
        assert_eq!(latest.previous_file_number, Some(1131084));
        assert_eq!(latest.is_amended, Some(false));
        assert_eq!(
            latest.receipt_date.unwrap().to_string(),
            "2017-03-04 00:00:00"
        );
        assert_eq!(latest.coverage_start_date, Some(ymd(2016, 10, 1)));
        assert_eq!(latest.coverage_end_date, Some(ymd(2016, 10, 19)));
        assert_eq!(latest.update_date, Some(ymd(2017, 3, 4)));
        assert_eq!(
            latest.fec_url.as_deref(),
            Some("https://docquery.fec.gov/dcdev/posted/1151343.fec")
        );
        assert_eq!(latest.raw_url().as_deref(), latest.fec_url.as_deref());
        assert_eq!(
            latest.csv_url.as_deref(),
            Some("https://docquery.fec.gov/csv/343/1151343.csv")
        );
        assert_eq!(latest.means_filed.as_deref(), Some("e-file"));
        assert_eq!(latest.pages, Some(39));
        // Money is exact decimal, not f64.
        assert_eq!(latest.total_receipts, Some(dec!(25270.16)));
        assert_eq!(latest.total_disbursements, Some(dec!(90886.89)));
        assert_eq!(latest.cash_on_hand_beginning_period, Some(dec!(256061.18)));
        assert_eq!(latest.cash_on_hand_end_period, Some(dec!(190444.45)));
        assert_eq!(latest.debts_owed_by_committee, Some(dec!(0.0)));
        assert_eq!(latest.total_individual_contributions, None);

        let first_amendment = &page.results[1];
        assert_eq!(first_amendment.file_number, Some(1131084));
        assert_eq!(first_amendment.amendment_version, Some(1));
        assert_eq!(first_amendment.most_recent, Some(false));
        assert_eq!(first_amendment.most_recent_file_number, Some(1151343));
        assert_eq!(first_amendment.previous_file_number, Some(1118027));
        assert_eq!(first_amendment.amendment_chain, vec![1118027, 1131084]);

        let original = &page.results[2];
        assert_eq!(original.file_number, Some(1118027));
        assert_eq!(original.amendment_indicator.as_deref(), Some("N"));
        assert_eq!(original.amendment_version, Some(0));
        assert_eq!(original.amendment_chain, vec![1118027]);
        assert_eq!(original.previous_file_number, Some(1118027));
        assert_eq!(
            original.receipt_date.unwrap().to_string(),
            "2016-10-27 00:00:00"
        );
    }

    #[test]
    fn decodes_a_multi_page_most_recent_response() {
        let page: Page<FilingRecord> =
            serde_json::from_str(&fixture("filings_most_recent.json")).unwrap();
        assert_eq!(page.pagination.count, 11);
        assert_eq!(page.pagination.pages, 3);
        assert!(page.has_more());
        assert_eq!(page.results.len(), 5);
        let ids: Vec<u64> = page.results.iter().filter_map(|r| r.filing_id()).collect();
        assert_eq!(ids, [1995828, 1995921, 1983339, 1970745, 1965728]);
        assert!(page.results.iter().all(|r| !r.is_paper()));
        assert!(page.results.iter().all(|r| r.most_recent == Some(true)));
        assert!(page.results.iter().all(|r| r.cycle == Some(2026)));

        // An F1 has no report type or coverage; nulls decode to None.
        let f1 = &page.results[2];
        assert_eq!(f1.form_type.as_deref(), Some("F1"));
        assert_eq!(f1.report_type, None);
        assert_eq!(f1.coverage_start_date, None);
        assert_eq!(f1.total_receipts, None);
        assert_eq!(f1.amendment_version, Some(7));

        let q2 = &page.results[0];
        assert_eq!(q2.report_type.as_deref(), Some("Q2"));
        assert_eq!(q2.total_receipts, Some(dec!(8153416.49)));
        assert_eq!(q2.coverage_start_date, Some(ymd(2026, 4, 1)));
        assert_eq!(q2.coverage_end_date, Some(ymd(2026, 6, 30)));
    }

    #[test]
    fn decodes_the_efile_endpoint() {
        let page: Page<EfileRecord> = serde_json::from_str(&fixture("efile_filings.json")).unwrap();
        assert_eq!(page.pagination.count, 586073);
        assert_eq!(page.pagination.is_count_exact, Some(false));
        assert_eq!(page.pagination.pages, 195358);
        assert_eq!(page.results.len(), 3);
        let r = &page.results[0];
        assert_eq!(r.file_number, Some(2011929));
        assert_eq!(r.filing_id(), Some(2011929));
        assert_eq!(r.fec_file_id.as_deref(), Some("FEC-2011929"));
        assert_eq!(r.committee_id.as_deref(), Some("C00946749"));
        assert_eq!(
            r.committee_name.as_deref(),
            Some("DEMOCRATIC COUNTY EXECUTIVE COMMITTEE OF PHILADELPHIA")
        );
        assert_eq!(r.form_type.as_deref(), Some("F99"));
        assert_eq!(r.receipt_date.unwrap().to_string(), "2026-09-14 22:50:35");
        assert_eq!(r.filed_date, Some(ymd(2026, 9, 14)));
        assert_eq!(r.load_timestamp.unwrap().to_string(), "2026-09-14 22:50:41");
        assert_eq!(r.coverage_start_date, None);
        assert_eq!(r.amendment_chain, vec![2011929]);
        assert_eq!(r.amendment_number, Some(0));
        assert_eq!(r.amended_by, None);
        assert_eq!(r.most_recent, Some(true));
        assert_eq!(r.most_recent_filing, Some(2011929));
        assert_eq!(
            r.fec_url.as_deref(),
            Some("https://docquery.fec.gov/dcdev/posted/2011929.fec")
        );
        assert_eq!(
            r.pdf_url.as_deref(),
            Some("https://docquery.fec.gov/pdf/943/202609149904205943/202609149904205943.pdf")
        );
    }

    #[test]
    fn unknown_fields_and_missing_fields_are_tolerated() {
        let page: Page<FilingRecord> = serde_json::from_str(
            r#"{"results":[{"file_number":1,"brand_new_column":{"x":[1,2]}},{}],"pagination":{"page":1,"pages":1}}"#,
        )
        .unwrap();
        assert_eq!(page.results.len(), 2);
        assert_eq!(page.results[0].file_number, Some(1));
        assert_eq!(page.results[1], FilingRecord::default());
        assert_eq!(page.results[1].raw_url(), None);
        assert_eq!(page.api_version, None);
        assert_eq!(page.pagination.count, 0);

        // The live API sends `amendment_chain: null` for some filings
        // (seen on C00554709's older reports); null lists are empty.
        let r: FilingRecord = serde_json::from_str(
            r#"{"file_number": 9, "amendment_chain": null, "most_recent": null, "receipt_date": null}"#,
        )
        .unwrap();
        assert_eq!(r.amendment_chain, Vec::<i64>::new());
        assert_eq!(r.most_recent, None);
        let e: EfileRecord =
            serde_json::from_str(r#"{"file_number": 9, "amendment_chain": null}"#).unwrap();
        assert_eq!(e.amendment_chain, Vec::<i64>::new());

        // fec_url absent but file_number present: the docquery URL.
        let r: FilingRecord = serde_json::from_str(r#"{"file_number": 42}"#).unwrap();
        assert_eq!(r.filing_id(), Some(42));
        assert_eq!(
            r.raw_url().as_deref(),
            Some("https://docquery.fec.gov/dcdev/posted/42.fec")
        );
    }

    /// openFEC numbers paper filings negatively, and a chain that began
    /// on paper carries the negative id (C00554709's 2013-2020 F3 chain
    /// starts at -6899830, as returned live on 2026-09-15).
    #[test]
    fn paper_filings_have_negative_numbers_and_no_raw_fec() {
        let r: FilingRecord = serde_json::from_str(
            r#"{"file_number": 1703302, "form_type": "F3", "means_filed": "e-file",
                "amendment_chain": [-6899830, 900330, 987585, 1059222, 1301108, 1587559, 1703302],
                "most_recent_file_number": 1703302, "previous_file_number": 1587559}"#,
        )
        .unwrap();
        assert_eq!(r.filing_id(), Some(1703302));
        assert!(!r.is_paper());
        assert_eq!(r.amendment_chain[0], -6899830);
        assert_eq!(r.amendment_chain.len(), 7);

        let paper: FilingRecord = serde_json::from_str(
            r#"{"file_number": -6899830, "form_type": "F3", "means_filed": "paper"}"#,
        )
        .unwrap();
        assert_eq!(paper.file_number, Some(-6899830));
        assert_eq!(paper.filing_id(), None);
        assert!(paper.is_paper());
        assert_eq!(paper.raw_url(), None);

        let rfq: FilingRecord = serde_json::from_str(
            r#"{"file_number": null, "form_type": "FRQ", "means_filed": "paper", "amendment_chain": null}"#,
        )
        .unwrap();
        assert_eq!(rfq.filing_id(), None);
        assert!(rfq.is_paper());
        assert!(rfq.amendment_chain.is_empty());
    }

    #[test]
    fn query_string_is_deterministic_and_encoded() {
        let q = FilingsQuery::new()
            .committee_id("C00554709")
            .form_type("F3X")
            .cycle(2026)
            .most_recent(true)
            .min_receipt_date(ymd(2026, 1, 1))
            .per_page(100)
            .page(2);
        assert_eq!(
            q.query_string().unwrap(),
            "committee_id=C00554709&form_type=F3X&cycle=2026&most_recent=true\
             &min_receipt_date=2026-01-01&sort=-receipt_date&per_page=100&page=2"
        );
        let e = EfileQuery::new()
            .committee_id("C00946749")
            .file_number(2011929)
            .max_receipt_date(ymd(2026, 9, 14))
            .sort(Some("receipt_date"))
            .per_page(3);
        assert_eq!(
            e.query_string().unwrap(),
            "committee_id=C00946749&file_number=2011929&max_receipt_date=2026-09-14\
             &sort=receipt_date&per_page=3&page=1"
        );
        assert_eq!(
            FilingsQuery::new()
                .report_type("Q2 & more")
                .sort(None::<&str>)
                .per_page(1)
                .query_string()
                .unwrap(),
            "report_type=Q2%20%26%20more&per_page=1&page=1"
        );
    }

    #[test]
    fn query_validation_errors_are_typed() {
        for bad in [
            FilingsQuery::new().cycle(2027),
            FilingsQuery::new().cycle(1900),
            FilingsQuery::new().cycle(2102),
            FilingsQuery::new().per_page(0),
            FilingsQuery::new().per_page(101),
            FilingsQuery::new().page(0),
        ] {
            let err = bad.query_string().unwrap_err();
            assert!(
                matches!(err, FecApiError::InvalidQuery(_)),
                "{bad:?}: {err}"
            );
        }
        assert!(FilingsQuery::new().cycle(1976).query_string().is_ok());
        assert!(FilingsQuery::new().cycle(2100).query_string().is_ok());
        assert!(matches!(
            EfileQuery::new().per_page(1000).query_string(),
            Err(FecApiError::InvalidQuery(_))
        ));
    }

    #[test]
    fn url_puts_the_key_first_and_never_leaks_it() {
        const KEY: &str = "sEcReTkEy1234567890";
        let key = ApiKey::new(KEY).unwrap();
        assert_eq!(format!("{key:?}"), "ApiKey(REDACTED)");
        assert_eq!(key.to_string(), "REDACTED");

        let client = OpenFec::new(key.clone());
        let q = FilingsQuery::new().committee_id("C00554709").per_page(5);
        assert_eq!(
            client.url("filings/", &q.query_string().unwrap()),
            "https://api.open.fec.gov/v1/filings/?api_key=REDACTED\
             &committee_id=C00554709&sort=-receipt_date&per_page=5&page=1"
        );
        assert_eq!(
            client.url("/efile/filings/", ""),
            "https://api.open.fec.gov/v1/efile/filings/?api_key=REDACTED"
        );
        let debug = format!("{client:?}");
        assert!(!debug.contains(KEY), "{debug}");
        assert!(debug.contains("ApiKey(REDACTED)"), "{debug}");

        let proxied = OpenFec::new(key).with_base_url("http://localhost:9999/v1");
        assert_eq!(proxied.base_url(), "http://localhost:9999/v1/");
        assert_eq!(
            proxied.url("filings/", "page=1"),
            "http://localhost:9999/v1/filings/?api_key=REDACTED&page=1"
        );
        assert!(!proxied.url("filings/", "page=1").contains(KEY));
    }

    #[test]
    fn api_key_rejects_garbage() {
        assert!(matches!(ApiKey::new(""), Err(FecApiError::InvalidQuery(_))));
        assert!(matches!(
            ApiKey::new("key with space"),
            Err(FecApiError::InvalidQuery(_))
        ));
        assert!(matches!(
            ApiKey::new("key\n"),
            Err(FecApiError::InvalidQuery(_))
        ));
        assert!(ApiKey::new("DEMO_KEY").is_ok());
    }

    #[test]
    fn http_errors_from_a_local_server_are_typed_and_redacted() {
        // A tiny HTTP server that answers every request with 404 --
        // enough to exercise the status path without the network.
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            let body = "{\"message\": \"nothing at all here\"}";
            write!(
                stream,
                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
            request
        });

        let client = OpenFec::new(ApiKey::new("LOCALKEY").unwrap())
            .with_base_url(format!("http://{addr}/v1/"));
        let err = client
            .filings(&FilingsQuery::new().committee_id("C1"))
            .unwrap_err();
        let request = handle.join().unwrap();
        // The key really was sent ...
        assert!(request.contains("api_key=LOCALKEY"), "{request}");
        // ... and never comes back out.
        match &err {
            FecApiError::Http {
                status,
                url,
                body_snippet,
            } => {
                assert_eq!(*status, 404);
                assert!(url.contains("api_key=REDACTED"), "{url}");
                assert!(!url.contains("LOCALKEY"), "{url}");
                assert!(
                    body_snippet.contains("nothing at all here"),
                    "{body_snippet}"
                );
            }
            other => panic!("expected Http, got {other:?}"),
        }
        let text = err.to_string();
        assert!(!text.contains("LOCALKEY"), "{text}");
        assert!(text.contains("HTTP 404"), "{text}");
    }

    #[test]
    fn rate_limit_is_surfaced_after_the_retry_budget() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = std::thread::spawn(move || {
            let mut hits = 0;
            for _ in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf).unwrap();
                write!(
                    stream,
                    "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                )
                .unwrap();
                hits += 1;
            }
            hits
        });
        let client = OpenFec::new(ApiKey::new("K").unwrap())
            .with_base_url(format!("http://{addr}/v1/"))
            .retry_policy(1, Duration::from_secs(5));
        let err = client.filings(&FilingsQuery::new()).unwrap_err();
        assert_eq!(handle.join().unwrap(), 2, "one attempt plus one retry");
        assert!(
            matches!(
                err,
                FecApiError::RateLimited {
                    retry_after: Some(d)
                } if d == Duration::ZERO
            ),
            "{err:?}"
        );
        assert!(err.to_string().contains("1,000 requests per hour"));
    }

    #[test]
    fn missing_key_names_where_it_looked() {
        // Serialise env mutation: tests run in parallel and this one
        // touches process-wide state, so keep it short and restore.
        let saved_key = std::env::var_os("FEC_API_KEY");
        let saved_home = std::env::var_os("HOME");
        let empty_home =
            std::env::temp_dir().join(format!("hardmoney-no-key-home-{}", std::process::id()));
        std::fs::create_dir_all(&empty_home).unwrap();
        // SAFETY: single-threaded within this test; restored below.
        unsafe {
            std::env::remove_var("FEC_API_KEY");
            std::env::set_var("HOME", &empty_home);
        }
        let result = ApiKey::from_env();
        unsafe {
            match &saved_key {
                Some(v) => std::env::set_var("FEC_API_KEY", v),
                None => std::env::remove_var("FEC_API_KEY"),
            }
            match &saved_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
        let _ = std::fs::remove_dir_all(&empty_home);
        match result {
            Err(FecApiError::MissingApiKey { looked_in }) => {
                assert_eq!(looked_in[0], "FEC_API_KEY");
                assert!(looked_in[1].ends_with("fec_api_key.txt"), "{looked_in:?}");
            }
            other => panic!("expected MissingApiKey, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------
    // Network
    // -----------------------------------------------------------------

    /// Lists C00554709's Form 3 filings for 2016 and checks the
    /// 1118027 -> 1131084 -> 1151343 amendment chain. Historical data;
    /// the chain has been closed since March 2017.
    #[test]
    #[ignore = "network: HARDMONEY_NETWORK_TESTS=1 plus an API key"]
    fn network_lists_desaulnier_2016_pre_general_chain() {
        if !network_tests_enabled() {
            return;
        }
        let api = OpenFec::from_env().expect("API key in FEC_API_KEY or ~/fec_api_key.txt");
        let query = FilingsQuery::new()
            .committee_id("C00554709")
            .form_type("F3")
            .cycle(2016)
            .report_type("12G")
            .per_page(20);
        let page = api.filings(&query).unwrap();
        assert!(!page.has_more(), "{:?}", page.pagination);
        let by_id = |id: i64| {
            page.results
                .iter()
                .find(|r| r.file_number == Some(id))
                .unwrap_or_else(|| panic!("filing {id} not in results: {:?}", page.results))
        };
        let original = by_id(1118027);
        let first = by_id(1131084);
        let latest = by_id(1151343);
        for r in [original, first, latest] {
            assert_eq!(r.most_recent_file_number, Some(1151343));
            assert_eq!(r.committee_id.as_deref(), Some("C00554709"));
            assert_eq!(r.coverage_start_date, Some(ymd(2016, 10, 1)));
            assert_eq!(r.coverage_end_date, Some(ymd(2016, 10, 19)));
        }
        // `amendment_chain` is the chain *up to* each filing.
        assert_eq!(original.amendment_chain, vec![1118027]);
        assert_eq!(original.amendment_indicator.as_deref(), Some("N"));
        assert_eq!(original.amendment_version, Some(0));
        assert_eq!(original.most_recent, Some(false));
        assert_eq!(first.amendment_chain, vec![1118027, 1131084]);
        assert_eq!(first.amendment_version, Some(1));
        assert_eq!(first.previous_file_number, Some(1118027));
        assert_eq!(first.most_recent, Some(false));
        assert_eq!(latest.amendment_chain, vec![1118027, 1131084, 1151343]);
        assert_eq!(latest.amendment_version, Some(2));
        assert_eq!(latest.previous_file_number, Some(1131084));
        assert_eq!(latest.most_recent, Some(true));
        assert_eq!(latest.total_receipts, Some(dec!(25270.16)));

        // The auto-paginating iterator agrees with the single page.
        let all: Vec<FilingRecord> = api
            .filings_all(&query.clone().per_page(2))
            .collect::<Result<_, _>>()
            .unwrap();
        let mut ids: Vec<i64> = all.iter().filter_map(|r| r.file_number).collect();
        ids.sort_unstable();
        let mut expected: Vec<i64> = page.results.iter().filter_map(|r| r.file_number).collect();
        expected.sort_unstable();
        assert_eq!(ids, expected);
    }

    /// FirstEnergy PAC's August 2026 monthly (2006786) was fully processed
    /// on 2026-08-31, 14 days after receipt; those dates do not change.
    /// Also checks the live `fec_url` resolution and the docquery fallback
    /// in `hardmoney::fec::resolve_fec_url`.
    #[test]
    #[ignore = "network: HARDMONEY_NETWORK_TESTS=1 plus an API key"]
    fn network_processing_status_of_a_settled_filing() {
        if !network_tests_enabled() {
            return;
        }
        let api = OpenFec::from_env().expect("API key in FEC_API_KEY or ~/fec_api_key.txt");
        let statuses = api.processing_status(&[2006786]).unwrap();
        let s = &statuses[0];
        assert!(s.in_efile && s.in_filings && s.in_operations_log, "{s:?}");
        assert_eq!(s.committee_id.as_deref(), Some("C00140855"));
        assert_eq!(s.received.unwrap().date(), ymd(2026, 8, 17));
        assert_eq!(s.summary_lag_days(), Some(0));
        assert_eq!(s.transaction_lag_days(), Some(14));
        assert_eq!(s.stage(), ProcessingStage::TransactionsLoaded);

        let url = api.resolve_fec_url(2006786).unwrap();
        assert_eq!(
            url.as_deref(),
            Some("https://docquery.fec.gov/dcdev/posted/2006786.fec")
        );
        assert_eq!(
            hardmoney::fec::resolve_fec_url(2006786).unwrap(),
            "https://docquery.fec.gov/dcdev/posted/2006786.fec"
        );
        // An id no endpoint knows resolves to the template.
        assert_eq!(
            hardmoney::fec::resolve_fec_url(999_999_999).unwrap(),
            hardmoney::fec::docquery_url(999_999_999)
        );

        // A committee's whole cycle in a handful of requests.
        let all = api
            .committee_processing_status("C00140855", 2026, Some("F3X"))
            .unwrap();
        assert!(all.len() >= 8, "{}", all.len());
        assert!(all.iter().all(|s| s.in_filings));
        let m8 = all.iter().find(|s| s.filing_id == 2006786).unwrap();
        assert_eq!(m8.transaction_data_complete, Some(ymd(2026, 8, 31)));
    }
}

// ---------------------------------------------------------------------
// RSS feed
// ---------------------------------------------------------------------

#[test]
fn parses_the_captured_feed() {
    let items = parse_feed(&fixture("rss_sample.xml")).unwrap();
    assert_eq!(items.len(), 10);

    let first = &items[0];
    assert_eq!(first.filing_id, 2011915);
    assert_eq!(first.form_type, "F1N");
    assert_eq!(first.base_form_type(), "F1");
    assert_eq!(first.committee_id.as_deref(), Some("C00961573"));
    assert_eq!(first.committee_name.as_deref(), Some("Vital Objective PAC"));
    assert_eq!(
        first.url,
        "https://docquery.fec.gov/dcdev/posted/2011915.fec"
    );
    assert_eq!(
        first.published.unwrap().to_rfc3339(),
        "2026-09-15T00:13:52+00:00"
    );
    assert_eq!(first.report_type, None);
    assert_eq!(first.coverage_start, None);

    let monthly = &items[1];
    assert_eq!(monthly.filing_id, 2011914);
    assert_eq!(monthly.form_type, "F3XN");
    assert_eq!(monthly.committee_id.as_deref(), Some("C00625988"));
    assert_eq!(monthly.committee_name.as_deref(), Some("FUTURE FORUM PAC"));
    assert_eq!(monthly.report_type.as_deref(), Some("SEPTEMBER MONTHLY"));
    assert_eq!(monthly.coverage_start, Some(ymd(2026, 8, 1)));
    assert_eq!(monthly.coverage_end, Some(ymd(2026, 8, 31)));
    assert!(monthly.matches_form_type("F3X"));
    assert!(monthly.matches_form_type("F3XN"));
    assert!(!monthly.matches_form_type("F3XA"));
    assert!(!monthly.matches_form_type("F3"));

    let forms: Vec<&str> = items.iter().map(|i| i.form_type.as_str()).collect();
    assert_eq!(
        forms,
        [
            "F1N", "F3XN", "F1N", "F3XA", "F24N", "F3XA", "F3XA", "F99", "F99", "F3XN"
        ]
    );
    // Ids descend (newest first) and are all distinct.
    let ids: Vec<u64> = items.iter().map(|i| i.filing_id).collect();
    assert!(ids.windows(2).all(|w| w[0] > w[1]), "{ids:?}");
    // Every item carries a committee id and a timestamp.
    assert!(
        items
            .iter()
            .all(|i| i.committee_id.is_some() && i.published.is_some())
    );
}

#[test]
fn feed_parse_failures_are_typed() {
    let err = parse_feed("<!DOCTYPE html><html><body>maintenance</body></html>").unwrap_err();
    assert!(matches!(err, FecApiError::Rss(_)), "{err:?}");
    assert!(err.to_string().contains("maintenance"), "{err}");
    // An RSS document with no items is fine: an empty feed, not an error.
    assert!(
        parse_feed("<rss><channel><title>x</title></channel></rss>")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn feed_client_defaults_to_the_fec_url() {
    let feed = EfileFeed::new();
    assert_eq!(
        feed.url(),
        "https://efilingapps.fec.gov/rss/generate?preDefinedFilingType=ALL"
    );
    assert_eq!(EfileFeed::default().url(), feed.url());
    assert_eq!(EfileFeed::with_url("http://x/feed").url(), "http://x/feed");
}

/// Every host the crate used before `Endpoints` existed is the default it
/// has now, as seen through the public constants and helpers that kept
/// their names.
#[test]
fn endpoint_defaults_pin_the_previous_constants() {
    use hardmoney::fec::efile::{DAILY_ZIP_BASE, FEED_URL, daily_zip_url};
    use hardmoney::fec::{Endpoints, docquery_url};
    use hardmoney::parser::webcheck::{DEFAULT_ENDPOINT, WebCheck};

    let e = Endpoints::default();
    assert_eq!(e.efile_rss(), FEED_URL);
    assert_eq!(
        e.daily_efile_zip(ymd(2026, 9, 6)),
        format!("{DAILY_ZIP_BASE}20260906.zip")
    );
    assert_eq!(
        e.daily_efile_zip(ymd(2026, 9, 6)),
        daily_zip_url(ymd(2026, 9, 6))
    );
    assert_eq!(e.docquery_filing(2011915), docquery_url(2011915));
    assert_eq!(
        docquery_url(2011915),
        "https://docquery.fec.gov/dcdev/posted/2011915.fec"
    );
    assert_eq!(e.webcheck_endpoint.as_str(), DEFAULT_ENDPOINT);
    assert_eq!(
        WebCheck::with_endpoints(&e).upload_url(),
        WebCheck::new().upload_url()
    );
    #[cfg(feature = "serde")]
    assert_eq!(
        e.openfec_base.as_str(),
        hardmoney::fec::openfec::OpenFec::BASE_URL
    );
    assert_eq!(
        e.dump_readme(),
        "https://www.fec.gov/files/bulk-downloads/data-dump/schedules/README.txt"
    );
    assert!(e.is_default());
}

/// A feed client built from mirrored endpoints polls the mirror's feed
/// and rebases each item's link; the lookup-based constructor is how the
/// CLI merges flags and variables without touching the process
/// environment.
#[test]
fn feed_client_follows_endpoints_from_a_lookup() {
    use hardmoney::fec::efile::parse_feed_with;
    use hardmoney::fec::{EndpointError, Endpoints};

    let vars = [
        (Endpoints::EFILING_APPS_BASE_VAR, "https://apps.mirror.gov"),
        (Endpoints::DOCQUERY_BASE_VAR, "https://dq.mirror.gov/store/"),
    ];
    let lookup = |var: &str| {
        vars.iter()
            .find(|(k, _)| *k == var)
            .map(|(_, v)| v.to_string())
    };
    let endpoints = Endpoints::from_lookup(lookup).unwrap();
    let feed = EfileFeed::with_endpoints(&endpoints);
    assert_eq!(
        feed.url(),
        "https://apps.mirror.gov/rss/generate?preDefinedFilingType=ALL"
    );
    let items = parse_feed_with(&fixture("rss_sample.xml"), &endpoints).unwrap();
    assert_eq!(items.len(), 10);
    assert!(
        items.iter().all(|i| i.url
            == format!(
                "https://dq.mirror.gov/store/dcdev/posted/{}.fec",
                i.filing_id
            )),
        "{:?}",
        items.iter().map(|i| i.url.as_str()).collect::<Vec<_>>()
    );

    // A malformed override fails before any request, naming the variable.
    let err = Endpoints::from_lookup(|var| {
        (var == Endpoints::DOCQUERY_BASE_VAR).then(|| "dq.mirror.gov".to_string())
    })
    .unwrap_err();
    assert!(matches!(
        &err,
        EndpointError::InvalidOverride { var, .. } if *var == Endpoints::DOCQUERY_BASE_VAR
    ));
    assert_eq!(
        err.to_string(),
        "HARDMONEY_DOCQUERY_BASE is set to \"dq.mirror.gov\", which is not a usable URL: it must start with http:// or https://"
    );
}

/// Polls the live feed once, as `hardmoney efile watch --once` does, and
/// checks that at least one item parses with an id and a form type.
#[test]
#[ignore = "network: HARDMONEY_NETWORK_TESTS=1"]
fn network_feed_poll_yields_items() {
    if !network_tests_enabled() {
        return;
    }
    let items: Vec<FeedItem> = EfileFeed::new().poll().unwrap();
    assert!(!items.is_empty(), "the feed always has the last 7 days");
    let first = &items[0];
    assert!(first.filing_id > 2_000_000, "{first:?}");
    assert!(first.form_type.starts_with('F'), "{first:?}");
    assert!(
        first.url.starts_with("https://docquery.fec.gov/"),
        "{first:?}"
    );
    assert!(items.iter().all(|i| i.filing_id > 0));
    eprintln!("{} items; newest: {first:?}", items.len());
}

// ---------------------------------------------------------------------
// Cache and Retry-After
// ---------------------------------------------------------------------

#[test]
fn cache_paths_and_env_override() {
    let root = PathBuf::from("/tmp/hm-test-root");
    let cache = Cache::at(&root);
    assert_eq!(cache.root(), root.as_path());
    assert_eq!(cache.filing_path(2011915), root.join("filings/2011915.fec"));
    assert_eq!(
        cache.daily_zip_path(ymd(2026, 9, 6)),
        root.join("efile/20260906.zip")
    );
    assert_eq!(cache.seen_path(), root.join("efile-seen.txt"));
    assert_eq!(Cache::ENV_VAR, "HARDMONEY_CACHE_DIR");

    // from_env honours HARDMONEY_CACHE_DIR; without it, ends in /hardmoney.
    let saved = std::env::var_os("HARDMONEY_CACHE_DIR");
    // SAFETY: restored below; the only other reader of this variable in
    // this test binary is this test.
    unsafe { std::env::set_var("HARDMONEY_CACHE_DIR", "/tmp/hm-from-env") };
    let from_env = Cache::from_env();
    unsafe { std::env::remove_var("HARDMONEY_CACHE_DIR") };
    let default = Cache::from_env();
    unsafe {
        match saved {
            Some(v) => std::env::set_var("HARDMONEY_CACHE_DIR", v),
            None => std::env::remove_var("HARDMONEY_CACHE_DIR"),
        }
    }
    assert_eq!(from_env.root(), Path::new("/tmp/hm-from-env"));
    assert_eq!(
        default.root().file_name().and_then(|n| n.to_str()),
        Some("hardmoney")
    );
    assert!(
        default
            .root()
            .parent()
            .is_some_and(|p| p.ends_with(".cache") || std::env::var_os("XDG_CACHE_HOME").is_some()),
        "{}",
        default.root().display()
    );
}

#[test]
fn cache_round_trips_filings_and_seen_ids() {
    let root = std::env::temp_dir().join(format!("hardmoney-fec-api-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let cache = Cache::at(&root);
    assert_eq!(cache.read_filing(7).unwrap(), None);
    let written = cache.write_filing(7, b"HDR\x1cFEC\x1c8.5").unwrap();
    assert_eq!(written, cache.filing_path(7));
    assert_eq!(
        cache.read_filing(7).unwrap().as_deref(),
        Some(&b"HDR\x1cFEC\x1c8.5"[..])
    );
    cache.append_seen([3, 1, 2]).unwrap();
    assert_eq!(
        cache.read_seen().unwrap().into_iter().collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let info = cache.info().unwrap();
    assert_eq!(info.filings, 1);
    assert_eq!(info.filing_bytes, 11);
    assert_eq!(info.seen_ids, 3);
    assert_eq!(info.total_bytes(), 11);
    let before = cache.clear(true).unwrap();
    assert_eq!(before.filings, 1);
    let after = cache.info().unwrap();
    assert_eq!(after.root, root);
    assert_eq!(
        (
            after.filings,
            after.filing_bytes,
            after.daily_zips,
            after.seen_ids
        ),
        (0, 0, 0, 0)
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn retry_after_parsing() {
    assert_eq!(parse_retry_after("30"), Some(Duration::from_secs(30)));
    assert_eq!(parse_retry_after("  7  "), Some(Duration::from_secs(7)));
    assert_eq!(parse_retry_after("0"), Some(Duration::ZERO));
    assert_eq!(parse_retry_after(""), None);
    assert_eq!(parse_retry_after("-5"), None);
    assert_eq!(parse_retry_after("1.5"), None);
    assert_eq!(parse_retry_after("later"), None);
    // An HTTP-date in the past is "now".
    assert_eq!(
        parse_retry_after("Fri, 31 Dec 1999 23:59:59 GMT"),
        Some(Duration::ZERO)
    );
    // One in the future is the time until then.
    let far = parse_retry_after("Thu, 01 Jan 2099 00:00:00 GMT").unwrap();
    assert!(far > Duration::from_secs(365 * 24 * 3600), "{far:?}");
}

#[test]
fn url_redaction() {
    assert_eq!(
        redact_url("https://api.open.fec.gov/v1/filings/?api_key=abc123&page=1"),
        "https://api.open.fec.gov/v1/filings/?api_key=REDACTED&page=1"
    );
    assert_eq!(
        redact_url("https://x/?a=1&api_key=abc123"),
        "https://x/?a=1&api_key=REDACTED"
    );
    assert_eq!(redact_url("https://x/?a=1"), "https://x/?a=1");
}

#[test]
fn error_messages_are_actionable() {
    let e = FecApiError::MissingApiKey {
        looked_in: vec!["FEC_API_KEY".into(), "/home/me/fec_api_key.txt".into()],
    };
    let text = e.to_string();
    assert!(text.contains("FEC_API_KEY"), "{text}");
    assert!(text.contains("/home/me/fec_api_key.txt"), "{text}");
    assert!(text.contains("api.data.gov/signup"), "{text}");

    let e = FecApiError::RateLimited {
        retry_after: Some(Duration::from_secs(42)),
    };
    assert!(e.to_string().contains("42 second(s)"));
    let e = FecApiError::RateLimited { retry_after: None };
    assert!(e.to_string().contains("retry later"));

    let e = FecApiError::Http {
        status: 503,
        url: "https://x/?api_key=REDACTED".into(),
        body_snippet: String::new(),
    };
    assert_eq!(
        e.to_string(),
        "https://x/?api_key=REDACTED returned HTTP 503"
    );
}
