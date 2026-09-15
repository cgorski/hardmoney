//! End-to-end tests against a live Postgres: migrations, namespace
//! isolation, bulk load semantics, filing ingestion, and the REST API.
//!
//! Gated on `HARDMONEY_TEST_DATABASE_URL`. When unset every test here is a
//! no-op that prints a skip notice, so `cargo test` stays green on a
//! machine without Postgres. CI sets it (see `.github/workflows/ci.yml`).
//!
//! Each test works in its own randomly-named namespace and drops it at the
//! end, so tests can run in parallel against one database.

#![cfg(feature = "api")]

use std::path::PathBuf;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use hardmoney::Cycle;
use hardmoney::api::ApiConfig;
use hardmoney::bulk::dump::{self, DumpError, RestoreOptions, TocKind};
use hardmoney::bulk::{self, Input, LoadMode, LoadOptions};
use hardmoney::db::{self, DbConfig, Namespace};
use sqlx::PgPool;
use tower::ServiceExt as _;

fn database_url() -> Option<String> {
    std::env::var("HARDMONEY_TEST_DATABASE_URL").ok()
}

macro_rules! require_db {
    () => {
        match database_url() {
            Some(url) => url,
            None => {
                eprintln!("skipping: HARDMONEY_TEST_DATABASE_URL not set");
                return;
            }
        }
    };
}

struct TestNs {
    pool: PgPool,
    ns: Namespace,
    url: String,
}

impl TestNs {
    async fn new(url: &str, tag: &str) -> Self {
        // Unique per process and per call so parallel tests never collide.
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let ns = Namespace::new(format!("t_{tag}_{}_{seq}", std::process::id())).unwrap();
        let pool = db::connect(&DbConfig::new(url).namespace(ns.clone()))
            .await
            .expect("connect");
        db::migrate(&pool).await.expect("migrate");
        Self {
            pool,
            ns,
            url: url.to_string(),
        }
    }

    async fn drop(self) {
        let admin = db::connect(&DbConfig::new(&self.url)).await.unwrap();
        db::drop_namespace(&admin, &self.ns).await.unwrap();
    }
}

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name)
}

/// Builds a tiny `cn26.zip`-shaped archive with the given candidate rows.
fn candidates_zip(rows: &[(&str, &str)]) -> PathBuf {
    use std::io::Write as _;
    let src = &bulk::source::CANDIDATES;
    let mut body = String::new();
    for (id, name) in rows {
        let mut f: Vec<String> = (0..src.columns.len()).map(|_| String::new()).collect();
        f[0] = id.to_string();
        f[1] = name.to_string();
        body.push_str(&f.join("|"));
        body.push('\n');
    }
    let mut buf = std::io::Cursor::new(Vec::new());
    {
        let mut zw = zip::ZipWriter::new(&mut buf);
        zw.start_file("cn.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zw.write_all(body.as_bytes()).unwrap();
        zw.finish().unwrap();
    }
    // Unique per call: tests run in parallel threads and two of them build
    // 2-row zips; naming by row count made them overwrite each other.
    static ZIP_SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "hardmoney-it-cn-{}-{}.zip",
        std::process::id(),
        ZIP_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ));
    std::fs::write(&path, buf.into_inner()).unwrap();
    path
}

async fn count(pool: &PgPool, sql: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(sql.to_string()))
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn get_json(app: &axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
    let resp = app
        .clone()
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let json = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    (status, json)
}

#[tokio::test]
async fn migrations_are_idempotent_and_namespaces_are_isolated() {
    let url = require_db!();
    let a = TestNs::new(&url, "iso_a").await;
    let b = TestNs::new(&url, "iso_b").await;

    // Re-running migrate is a no-op.
    db::migrate(&a.pool).await.unwrap();
    let st = db::migration_status(&a.pool).await.unwrap();
    assert!(st.is_current());
    assert_eq!(st.applied.len(), db::MIGRATOR.iter().count());

    // Data in `a` is invisible from `b`.
    let zip = candidates_zip(&[("H0AA00001", "ALPHA, A")]);
    let cycle = Cycle::new(2026).unwrap();
    bulk::load(
        &a.pool,
        &bulk::source::CANDIDATES,
        Input::LocalFile(zip.clone()),
        LoadOptions::new(cycle),
    )
    .await
    .unwrap();
    assert_eq!(count(&a.pool, "SELECT count(*) FROM candidates").await, 1);
    assert_eq!(count(&b.pool, "SELECT count(*) FROM candidates").await, 0);

    let namespaces = db::list_namespaces(&a.pool).await.unwrap();
    assert!(namespaces.contains(&a.ns.to_string()));
    assert!(namespaces.contains(&b.ns.to_string()));

    let _ = std::fs::remove_file(zip);
    a.drop().await;
    b.drop().await;
}

#[tokio::test]
async fn replace_reloads_exactly_and_append_conflicts() {
    let url = require_db!();
    let t = TestNs::new(&url, "replace").await;
    let cycle = Cycle::new(2026).unwrap();
    let src = &bulk::source::CANDIDATES;

    let v1 = candidates_zip(&[("H0AA00001", "ALPHA, A"), ("H0AA00002", "BRAVO, B")]);
    let r1 = bulk::load(
        &t.pool,
        src,
        Input::LocalFile(v1.clone()),
        LoadOptions::new(cycle),
    )
    .await
    .unwrap();
    assert_eq!(r1.rows_loaded, 2);
    assert_eq!(r1.rows_replaced, 0);

    // The FEC's next weekly file dropped BRAVO and added CHARLIE.
    let v2 = candidates_zip(&[("H0AA00001", "ALPHA, A"), ("H0AA00003", "CHARLIE, C")]);
    let r2 = bulk::load(
        &t.pool,
        src,
        Input::LocalFile(v2.clone()),
        LoadOptions::new(cycle),
    )
    .await
    .unwrap();
    assert_eq!(r2.rows_loaded, 2);
    assert_eq!(r2.rows_replaced, 2);
    assert_eq!(
        count(
            &t.pool,
            "SELECT count(*) FROM candidates WHERE cand_id = 'H0AA00002'"
        )
        .await,
        0,
        "replace must remove rows the FEC removed"
    );
    assert_eq!(count(&t.pool, "SELECT count(*) FROM candidates").await, 2);

    // A different cycle is untouched by replace.
    let other = Cycle::new(2024).unwrap();
    bulk::load(
        &t.pool,
        src,
        Input::LocalFile(v1.clone()),
        LoadOptions::new(other),
    )
    .await
    .unwrap();
    bulk::load(
        &t.pool,
        src,
        Input::LocalFile(v2.clone()),
        LoadOptions::new(cycle),
    )
    .await
    .unwrap();
    assert_eq!(
        count(
            &t.pool,
            "SELECT count(*) FROM candidates WHERE cycle = 2024"
        )
        .await,
        2
    );

    // Append on existing rows is a clean database error, not a panic.
    let err = bulk::load(
        &t.pool,
        src,
        Input::LocalFile(v2.clone()),
        LoadOptions::new(cycle).mode(LoadMode::Append),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, bulk::BulkError::Database(_)), "{err}");

    // Confirmation guard.
    let err = bulk::load(
        &t.pool,
        src,
        Input::LocalFile(v2.clone()),
        LoadOptions::new(cycle)
            .mode(LoadMode::Replace)
            .confirmed(false)
            .limit(None),
    )
    .await;
    assert!(
        err.is_ok(),
        "2 rows is under the default threshold: {:?}",
        err.err()
    );
    let mut opts = LoadOptions::new(cycle);
    opts.confirm_threshold = 1;
    let err = bulk::load(&t.pool, src, Input::LocalFile(v2.clone()), opts)
        .await
        .unwrap_err();
    assert!(
        matches!(
            err,
            bulk::BulkError::ConfirmationRequired { existing: 2, .. }
        ),
        "{err}"
    );

    // Every successful load is recorded (the append and the refused
    // replace were rolled back / never started).
    assert_eq!(count(&t.pool, "SELECT count(*) FROM loads").await, 5);

    let _ = std::fs::remove_file(v1);
    let _ = std::fs::remove_file(v2);
    t.drop().await;
}

#[tokio::test]
async fn filing_ingest_is_idempotent_and_records_skips() {
    let url = require_db!();
    let t = TestNs::new(&url, "ingest").await;

    let bytes = std::fs::read(fixture("F24N_2011823.fec")).unwrap();
    let id = bulk::filing_id_from_path(&fixture("F24N_2011823.fec")).unwrap();
    assert_eq!(id, 2011823);
    let r1 = bulk::ingest_filing_bytes(&t.pool, id, &bytes, &hardmoney::ParseOptions::LENIENT)
        .await
        .unwrap();
    assert_eq!(r1.schedule_e_lines, 1);
    assert!(r1.skipped.is_empty());
    let r2 = bulk::ingest_filing_bytes(&t.pool, id, &bytes, &hardmoney::ParseOptions::LENIENT)
        .await
        .unwrap();
    assert_eq!(r2.schedule_e_lines, 1);
    assert_eq!(count(&t.pool, "SELECT count(*) FROM filings").await, 1);
    assert_eq!(
        count(&t.pool, "SELECT count(*) FROM schedule_e_lines").await,
        1
    );

    // A filing with an unknown line type: lenient records it, strict fails.
    let junk = format!(
        "{}\nZZZ\u{1c}junk\n",
        String::from_utf8_lossy(&std::fs::read(fixture("F99_2011828.fec")).unwrap()).trim_end()
    );
    let r3 = bulk::ingest_filing_bytes(
        &t.pool,
        999,
        junk.as_bytes(),
        &hardmoney::ParseOptions::LENIENT,
    )
    .await
    .unwrap();
    assert_eq!(r3.skipped.len(), 1);
    assert_eq!(
        count(
            &t.pool,
            "SELECT skipped_lines::bigint FROM filings WHERE filing_id = 999"
        )
        .await,
        1
    );
    let err = bulk::ingest_filing_bytes(
        &t.pool,
        998,
        junk.as_bytes(),
        &hardmoney::ParseOptions::STRICT,
    )
    .await
    .unwrap_err();
    assert!(matches!(err, bulk::BulkError::Parse(_)), "{err}");

    t.drop().await;
}

#[tokio::test]
async fn api_serves_loaded_data_with_hardening() {
    let url = require_db!();
    let t = TestNs::new(&url, "api").await;
    let cycle = Cycle::new(2026).unwrap();

    let zip = candidates_zip(&[("H0AA00001", "ALPHA, A"), ("S0AA00002", "SMITH, JANE")]);
    bulk::load(
        &t.pool,
        &bulk::source::CANDIDATES,
        Input::LocalFile(zip.clone()),
        LoadOptions::new(cycle),
    )
    .await
    .unwrap();
    let bytes = std::fs::read(fixture("F24N_2011823.fec")).unwrap();
    bulk::ingest_filing_bytes(&t.pool, 2011823, &bytes, &hardmoney::ParseOptions::LENIENT)
        .await
        .unwrap();

    let config = ApiConfig::new("127.0.0.1:0".parse().unwrap()).api_key(Some("k".into()));
    let app = hardmoney::api::router(t.pool.clone(), &config);

    let (s, _) = get_json(&app, "/health").await;
    assert_eq!(s, StatusCode::OK);

    let (s, body) = get_json(&app, "/candidates").await;
    assert_eq!(s, StatusCode::UNAUTHORIZED, "{body}");

    let (s, body) = get_json(&app, "/candidates?q=smith&api_key=k").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["cand_name"], "SMITH, JANE");

    let (s, body) = get_json(&app, "/candidates?cycle=2027&api_key=k").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");

    let (s, body) = get_json(&app, "/candidates/H0AA00001?api_key=k").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body[0]["cycle"], 2026);

    let (s, body) = get_json(&app, "/filings/2011823?api_key=k").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["form_type"], "F24N");
    assert_eq!(body["skipped_lines"], 0);

    let (s, body) = get_json(&app, "/filings/2011823/schedule-e?api_key=k").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body[0]["expenditure_amt"], "1074900.00");
    assert_eq!(body[0]["support_oppose_code"], "O");

    // Independent expenditures view is absent unless the FEC dump has been
    // restored into this database: a 503 with guidance, not a 500.
    let (s, body) = get_json(&app, "/independent-expenditures?api_key=k").await;
    assert!(
        s == StatusCode::SERVICE_UNAVAILABLE || s == StatusCode::OK,
        "{s} {body}"
    );
    if s == StatusCode::SERVICE_UNAVAILABLE {
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("bulk-restore-dump")
        );
    }

    let (s, body) = get_json(&app, "/schema?api_key=k").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["namespace"], t.ns.to_string());
    assert_eq!(body["migrations_pending"].as_array().unwrap().len(), 0);
    assert_eq!(body["loads"][0]["source"], "candidates");

    let _ = std::fs::remove_file(zip);
    t.drop().await;
}

// ---------------------------------------------------------------------------
// Amendment chains
// ---------------------------------------------------------------------------

/// A minimal spec-8.5 Form 3 as `.fec` text. `report_id`/`report_number`
/// go in the header (`FEC-<original>` / amendment number); the cover line
/// carries the committee, `12G`, and the coverage dates at their F3 v8.5
/// column positions.
fn f3_text(cover: &str, report_id: &str, report_number: &str) -> String {
    let header = ["HDR", "FEC", "8.5", "X", "1", report_id, report_number].join("\u{1c}");
    // F3 v8.5, 1-based: 1 form_type, 2 filer_committee_id_number,
    // 3 committee_name, 12 report_code, 16 coverage_from_date,
    // 17 coverage_through_date (data/fec-csv-sources/F3.csv).
    let mut cols = vec![""; 17];
    cols[0] = cover;
    cols[1] = "C00554709";
    cols[2] = "COMMITTEE";
    cols[11] = "12G";
    cols[15] = "20161001";
    cols[16] = "20161019";
    format!("{header}\n{}\n", cols.join("\u{1c}"))
}

async fn ingest_text(pool: &PgPool, id: i64, text: &str) -> bulk::IngestReport {
    bulk::ingest_filing_bytes(pool, id, text.as_bytes(), &hardmoney::ParseOptions::STRICT)
        .await
        .unwrap()
}

/// The resolved chain columns of one row, in a shape that is easy to
/// compare against openFEC's `/filings/` fields.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
struct Chain {
    filing_id: i64,
    amendment_version: Option<i32>,
    amendment_chain: Option<Vec<i64>>,
    most_recent: Option<bool>,
    most_recent_filing_id: Option<i64>,
    previous_filing_id: Option<i64>,
    chain_unresolved: bool,
}

impl Chain {
    fn resolved(
        filing_id: i64,
        version: i32,
        chain: &[i64],
        most_recent_id: i64,
        previous_id: i64,
    ) -> Self {
        Self {
            filing_id,
            amendment_version: Some(version),
            amendment_chain: Some(chain.to_vec()),
            most_recent: Some(filing_id == most_recent_id),
            most_recent_filing_id: Some(most_recent_id),
            previous_filing_id: Some(previous_id),
            chain_unresolved: false,
        }
    }

    /// A one-filing chain: an original with no amendments, an F99, or an
    /// amendment whose original is missing (`unresolved`).
    fn alone(filing_id: i64, unresolved: bool) -> Self {
        Self {
            chain_unresolved: unresolved,
            ..Self::resolved(filing_id, 0, &[filing_id], filing_id, filing_id)
        }
    }
}

async fn chains(pool: &PgPool) -> Vec<Chain> {
    sqlx::query_as::<_, Chain>(
        "SELECT filing_id, amendment_version, amendment_chain, most_recent, \
                most_recent_filing_id, previous_filing_id, chain_unresolved \
         FROM filings ORDER BY filing_id",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// openFEC's `/filings/?committee_id=C00554709&report_type=12G&cycle=2016`
/// (tmp/agent-fec-deep/pages/openfec_filings_c00554709.json): original
/// 1118027, amendments 1131084 (v1) and 1151343 (v2).
const OPENFEC_ORIGINAL: i64 = 1118027;
const OPENFEC_V1: i64 = 1131084;
const OPENFEC_V2: i64 = 1151343;

fn openfec_chain() -> Vec<Chain> {
    let all = [OPENFEC_ORIGINAL, OPENFEC_V1, OPENFEC_V2];
    vec![
        Chain::resolved(OPENFEC_ORIGINAL, 0, &all[..1], OPENFEC_V2, OPENFEC_ORIGINAL),
        Chain::resolved(OPENFEC_V1, 1, &all[..2], OPENFEC_V2, OPENFEC_ORIGINAL),
        Chain::resolved(OPENFEC_V2, 2, &all, OPENFEC_V2, OPENFEC_V1),
    ]
}

#[tokio::test]
async fn amendment_chain_matches_openfec_and_is_served_by_the_api() {
    let url = require_db!();
    let t = TestNs::new(&url, "amend").await;

    let r0 = ingest_text(&t.pool, OPENFEC_ORIGINAL, &f3_text("F3N", "", "")).await;
    let r1 = ingest_text(&t.pool, OPENFEC_V1, &f3_text("F3A", "FEC-1118027", "1")).await;
    let r2 = ingest_text(&t.pool, OPENFEC_V2, &f3_text("F3A", "FEC-1118027", "2")).await;
    // Each ingest recomputes the whole chain it belongs to.
    assert_eq!(r0.chain_rows_resolved, Some(1));
    assert_eq!(r1.chain_rows_resolved, Some(2));
    assert_eq!(r2.chain_rows_resolved, Some(3));

    assert_eq!(chains(&t.pool).await, openfec_chain());

    // The ingest-time columns.
    #[derive(Debug, PartialEq, Eq, sqlx::FromRow)]
    struct Filed {
        filing_id: i64,
        report_id: Option<String>,
        report_code: Option<String>,
        amendment_number: Option<i32>,
        coverage_from: Option<chrono::NaiveDate>,
        coverage_through: Option<chrono::NaiveDate>,
    }
    let rows: Vec<Filed> = sqlx::query_as(
        "SELECT filing_id, report_id, report_code, amendment_number, coverage_from, coverage_through \
         FROM filings ORDER BY filing_id",
    )
    .fetch_all(&t.pool)
    .await
    .unwrap();
    let filed = |filing_id, report_id: Option<&str>, amendment_number| Filed {
        filing_id,
        report_id: report_id.map(str::to_string),
        report_code: Some("12G".to_string()),
        amendment_number,
        coverage_from: chrono::NaiveDate::from_ymd_opt(2016, 10, 1),
        coverage_through: chrono::NaiveDate::from_ymd_opt(2016, 10, 19),
    };
    assert_eq!(
        rows,
        vec![
            filed(OPENFEC_ORIGINAL, None, None),
            filed(OPENFEC_V1, Some("FEC-1118027"), Some(1)),
            filed(OPENFEC_V2, Some("FEC-1118027"), Some(2)),
        ]
    );

    // `filings_current` is one row per chain: the latest version.
    assert_eq!(
        count(&t.pool, "SELECT count(*) FROM filings_current").await,
        1
    );
    assert_eq!(
        count(&t.pool, "SELECT filing_id FROM filings_current").await,
        OPENFEC_V2
    );

    // Over HTTP, with openFEC's field names.
    let config = ApiConfig::new("127.0.0.1:0".parse().unwrap()).api_key(Some("k".into()));
    let app = hardmoney::api::router(t.pool.clone(), &config);

    let (s, body) = get_json(&app, "/filings/1151343?api_key=k").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["form_type"], "F3A");
    assert_eq!(body["amendment_indicator"], "A");
    assert_eq!(body["amendment_version"], 2);
    assert_eq!(
        body["amendment_chain"],
        serde_json::json!([1118027, 1131084, 1151343])
    );
    assert_eq!(body["most_recent"], true);
    assert_eq!(body["most_recent_file_number"], 1151343);
    assert_eq!(body["previous_file_number"], 1131084);
    assert_eq!(body["chain_unresolved"], false);
    assert_eq!(body["report_type"], "12G");
    assert_eq!(body["coverage_start_date"], "2016-10-01");
    assert_eq!(body["coverage_end_date"], "2016-10-19");
    assert_eq!(
        body["fec_url"],
        "https://docquery.fec.gov/dcdev/posted/1151343.fec"
    );

    let (s, body) = get_json(&app, "/filings/1118027?api_key=k").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body["amendment_indicator"], "N");
    assert_eq!(body["amendment_version"], 0);
    assert_eq!(body["amendment_chain"], serde_json::json!([1118027]));
    assert_eq!(body["most_recent"], false);
    assert_eq!(body["most_recent_file_number"], 1151343);
    // openFEC: the original's previous_file_number is itself.
    assert_eq!(body["previous_file_number"], 1118027);

    let (s, body) = get_json(
        &app,
        "/filings?committee_id=C00554709&most_recent=true&api_key=k",
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 1);
    assert_eq!(body[0]["filing_id"], 1151343);
    assert_eq!(body[0]["most_recent"], true);

    // Newest first; no filter returns the whole chain.
    let (s, body) = get_json(&app, "/filings?committee_id=C00554709&api_key=k").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    let ids: Vec<i64> = body
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["filing_id"].as_i64().unwrap())
        .collect();
    assert_eq!(ids, vec![1151343, 1131084, 1118027]);

    let (s, body) = get_json(&app, "/filings?most_recent=false&api_key=k").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 2);

    // `form_type` matches a base form (any designator) or an exact token.
    for (q, want) in [("F3", 3), ("f3", 3), ("F3A", 2), ("F3N", 1), ("F3X", 0)] {
        let (s, body) = get_json(&app, &format!("/filings?form_type={q}&api_key=k")).await;
        assert_eq!(s, StatusCode::OK, "{body}");
        assert_eq!(body.as_array().unwrap().len(), want, "form_type={q}");
    }

    let (s, body) = get_json(&app, "/filings?limit=2&offset=1&api_key=k").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 2);
    assert_eq!(body[0]["filing_id"], 1131084);
    let (s, body) = get_json(&app, "/filings?limit=0&api_key=k").await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{body}");
    let (s, body) = get_json(&app, "/filings?committee_id=C00000000&api_key=k").await;
    assert_eq!(s, StatusCode::OK, "{body}");
    assert_eq!(body.as_array().unwrap().len(), 0);

    t.drop().await;
}

#[tokio::test]
async fn amendment_chain_is_order_independent_and_flags_unresolved() {
    let url = require_db!();
    let t = TestNs::new(&url, "amend_rev").await;

    // Amendments first: each stands alone, flagged, until the original
    // arrives.
    ingest_text(&t.pool, OPENFEC_V2, &f3_text("F3A", "FEC-1118027", "2")).await;
    assert_eq!(chains(&t.pool).await, vec![Chain::alone(OPENFEC_V2, true)]);
    ingest_text(&t.pool, OPENFEC_V1, &f3_text("F3A", "FEC-1118027", "1")).await;
    assert_eq!(
        chains(&t.pool).await,
        vec![
            Chain::alone(OPENFEC_V1, true),
            Chain::alone(OPENFEC_V2, true)
        ]
    );
    assert_eq!(
        count(&t.pool, "SELECT count(*) FROM filings_current").await,
        2,
        "an unresolved amendment is the most recent thing we know of"
    );

    // The original lands: the whole chain resolves to the openFEC values.
    let r = ingest_text(&t.pool, OPENFEC_ORIGINAL, &f3_text("F3N", "", "")).await;
    assert_eq!(r.chain_rows_resolved, Some(3));
    assert_eq!(chains(&t.pool).await, openfec_chain());
    assert_eq!(
        count(&t.pool, "SELECT count(*) FROM filings_current").await,
        1
    );

    // Two amendments with the same number: the later filing id wins.
    ingest_text(&t.pool, 1151344, &f3_text("F3A", "FEC-1118027", "2")).await;
    let after = chains(&t.pool).await;
    assert_eq!(after.len(), 4);
    assert_eq!(
        after[3],
        Chain::resolved(
            1151344,
            3,
            &[OPENFEC_ORIGINAL, OPENFEC_V1, OPENFEC_V2, 1151344],
            1151344,
            OPENFEC_V2
        )
    );
    assert_eq!(after[2].most_recent, Some(false));
    assert_eq!(after[0].most_recent_filing_id, Some(1151344));

    // Permanently unresolved: header names a filing we never see, and a
    // header with no `FEC-<n>` at all (the FEC's validator rejects the
    // latter; the parser does not). A real one: F3A_2011812 amends
    // FEC-1997089, which is not among the fixtures.
    let r = ingest_text(&t.pool, 2000001, &f3_text("F3A", "FEC-1999999", "1")).await;
    assert_eq!(r.chain_rows_resolved, Some(1));
    ingest_text(&t.pool, 2000002, &f3_text("F3A", "", "")).await;
    let bytes = std::fs::read(fixture("F3A_2011812.fec")).unwrap();
    bulk::ingest_filing_bytes(&t.pool, 2011812, &bytes, &hardmoney::ParseOptions::LENIENT)
        .await
        .unwrap();
    let all = chains(&t.pool).await;
    assert_eq!(all[4], Chain::alone(2000001, true));
    assert_eq!(all[5], Chain::alone(2000002, true));
    assert_eq!(all[6], Chain::alone(2011812, true));

    // A re-ingest that corrects the header moves the filing between chains
    // and repairs the chain it left.
    ingest_text(&t.pool, 2000001, &f3_text("F3A", "FEC-1118027", "3")).await;
    let moved = chains(&t.pool).await;
    assert_eq!(moved[4].amendment_version, Some(4));
    assert_eq!(moved[4].most_recent, Some(true));
    assert!(!moved[4].chain_unresolved);
    assert_eq!(moved[3].most_recent, Some(false), "1151344 was superseded");
    ingest_text(&t.pool, 2000001, &f3_text("F3A", "FEC-1999999", "1")).await;
    let back = chains(&t.pool).await;
    assert_eq!(back[4], Chain::alone(2000001, true));
    assert_eq!(back[3].most_recent, Some(true), "1151344 is current again");

    // `resolve_all_amendment_chains` reproduces the incremental result
    // from scratch (as after applying migration 0003 to an old namespace).
    let before = chains(&t.pool).await;
    assert_eq!(before.len(), 7);
    sqlx::query(
        "UPDATE filings SET amendment_version = NULL, amendment_chain = NULL, most_recent = NULL, \
         most_recent_filing_id = NULL, previous_filing_id = NULL, chain_unresolved = FALSE",
    )
    .execute(&t.pool)
    .await
    .unwrap();
    assert_eq!(
        count(&t.pool, "SELECT count(*) FROM filings_current").await,
        0
    );
    let n = db::resolve_all_amendment_chains(&t.pool).await.unwrap();
    assert_eq!(n, 7);
    assert_eq!(chains(&t.pool).await, before);

    // ...and it is how a `Defer` batch gets its columns.
    let deferred = bulk::ingest::ingest_filing_bytes_with(
        &t.pool,
        3000001,
        f3_text("F3A", "FEC-1118027", "4").as_bytes(),
        &hardmoney::ParseOptions::STRICT,
        bulk::ingest::ChainResolution::Defer,
    )
    .await
    .unwrap();
    assert_eq!(deferred.chain_rows_resolved, None);
    let stale = chains(&t.pool).await;
    assert_eq!(stale[7].most_recent, None, "deferred: nothing derived yet");
    assert_eq!(
        stale[3].most_recent,
        Some(true),
        "deferred: chain-mates untouched"
    );
    let n = db::resolve_all_amendment_chains(&t.pool).await.unwrap();
    assert_eq!(n, 8);
    let after = chains(&t.pool).await;
    assert_eq!(
        after[7],
        Chain::resolved(
            3000001,
            4,
            &[OPENFEC_ORIGINAL, OPENFEC_V1, OPENFEC_V2, 1151344, 3000001],
            3000001,
            1151344
        )
    );
    assert_eq!(after[3].most_recent, Some(false));
    assert_eq!(after[0].most_recent_filing_id, Some(3000001));
    assert_eq!(&after[4..7], &before[4..7], "unrelated rows are unchanged");

    t.drop().await;
}

#[tokio::test]
async fn f99_and_report_number_zero_originals_stand_alone() {
    let url = require_db!();
    let t = TestNs::new(&url, "amend_f99").await;

    // A Form 99 is never an amendment: chain = [self], most recent.
    let bytes = std::fs::read(fixture("F99_2011828.fec")).unwrap();
    let r = bulk::ingest_filing_bytes(&t.pool, 2011828, &bytes, &hardmoney::ParseOptions::LENIENT)
        .await
        .unwrap();
    assert_eq!(r.chain_rows_resolved, Some(1));
    assert_eq!(chains(&t.pool).await, vec![Chain::alone(2011828, false)]);

    // Real-world quirk: F3XN_2011831's header carries report_number "0" on
    // an ORIGINAL. The original must still be version 0 ahead of an
    // amendment numbered 1 (and would be even if the amendment's number
    // were blank: the head of the chain sorts first by construction).
    let bytes = std::fs::read(fixture("F3XN_2011831.fec")).unwrap();
    bulk::ingest_filing_bytes(&t.pool, 2011831, &bytes, &hardmoney::ParseOptions::LENIENT)
        .await
        .unwrap();
    assert_eq!(
        count(
            &t.pool,
            "SELECT amendment_number::bigint FROM filings WHERE filing_id = 2011831"
        )
        .await,
        0
    );
    let amendment = "HDR\u{1c}FEC\u{1c}8.5\u{1c}X\u{1c}1\u{1c}FEC-2011831\u{1c}\n\
                     F3XA\u{1c}C00140855\u{1c}FirstEnergy Corp Political Action Committee\n";
    ingest_text(&t.pool, 2011999, amendment).await;
    let rows = chains(&t.pool).await;
    assert_eq!(
        rows[1],
        Chain::resolved(2011831, 0, &[2011831], 2011999, 2011831)
    );
    assert_eq!(
        rows[2],
        Chain::resolved(2011999, 1, &[2011831, 2011999], 2011999, 2011831)
    );

    // The F3X's cover-line columns land too (openFEC's report_type and
    // coverage dates).
    let (code, from, through): (Option<String>, Option<chrono::NaiveDate>, Option<chrono::NaiveDate>) =
        sqlx::query_as(
            "SELECT report_code, coverage_from, coverage_through FROM filings WHERE filing_id = 2011831",
        )
        .fetch_one(&t.pool)
        .await
        .unwrap();
    assert_eq!(code.as_deref(), Some("M9"));
    assert_eq!(from, chrono::NaiveDate::from_ymd_opt(2026, 8, 1));
    assert_eq!(through, chrono::NaiveDate::from_ymd_opt(2026, 8, 31));

    t.drop().await;
}

// ---------------------------------------------------------------------------
// The FEC's pg_dump archives
// ---------------------------------------------------------------------------

/// The FEC's real `fec_fitem_sched_e.dump` (43 MB, 13 Sept 2026), kept
/// outside the repository. Absent on CI, so the dump test skips there.
fn local_schedule_e_dump() -> Option<PathBuf> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tmp/agent-db/dumps/schedule_e.dump");
    path.is_file().then_some(path)
}

fn pg_restore_available() -> bool {
    std::process::Command::new("pg_restore")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

async fn relation_exists(pool: &PgPool, name: &str) -> bool {
    let (exists,): (bool,) = sqlx::query_as("SELECT to_regclass($1::text) IS NOT NULL")
        .bind(name)
        .fetch_one(pool)
        .await
        .unwrap();
    exists
}

/// One test, in sequence, because every restore drops and recreates the
/// shared `disclosure.fec_fitem_sched_e` (other tests tolerate the view
/// vanishing under them, but two restores racing would not).
#[tokio::test]
async fn schedule_e_dump_restores_views_indexes_and_compares() {
    let url = require_db!();
    let Some(dump_path) = local_schedule_e_dump() else {
        eprintln!("skipping: tmp/agent-db/dumps/schedule_e.dump not present");
        return;
    };
    if !pg_restore_available() {
        eprintln!("skipping: pg_restore not on PATH");
        return;
    }
    let t = TestNs::new(&url, "dump").await;
    let cache_dir =
        std::env::temp_dir().join(format!("hardmoney-it-dump-cache-{}", std::process::id()));

    // -- Table of contents of the real archive -----------------------------
    let toc = dump::read_toc(&dump_path).unwrap();
    assert!(toc.has_table("fec_fitem_sched_e"), "{toc:?}");
    assert!(toc.archive_created.is_some());
    // One table, its data, its primary key, and the trigger whose function
    // is not in the archive. No per-cycle child tables, no indexes.
    assert_eq!(toc.count(&TocKind::Table), 1);
    assert_eq!(toc.count(&TocKind::TableData), 1);
    assert_eq!(toc.count(&TocKind::Constraint), 1);
    assert_eq!(toc.count(&TocKind::Trigger), 1);
    assert_eq!(toc.count(&TocKind::Index), 0);
    assert!(toc.partitions("fec_fitem_sched_e").is_empty());

    // -- Errors before anything touches the database -----------------------
    let err = dump::restore_with(
        &t.pool,
        &url,
        &dump::SCHEDULE_E,
        &cache_dir,
        &RestoreOptions::new().dump_file(Some(PathBuf::from("/nonexistent/x.dump"))),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DumpError::DumpFileMissing(_)), "{err}");
    let err = dump::restore_with(
        &t.pool,
        &url,
        &dump::SCHEDULE_E,
        &cache_dir,
        &RestoreOptions::new()
            .dump_file(Some(dump_path.clone()))
            .cycles([Cycle::new(2024).unwrap()]),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, DumpError::NotPartitioned { .. }), "{err}");
    let err = dump::restore_with(
        &t.pool,
        &url,
        &dump::SCHEDULE_A,
        &cache_dir,
        &RestoreOptions::new(),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, DumpError::LargeDumpNotAllowed { .. }),
        "{err}"
    );

    // -- Full restore from the local file ----------------------------------
    let from_file = RestoreOptions::new().dump_file(Some(dump_path.clone()));
    let r1 = dump::restore_with(&t.pool, &url, &dump::SCHEDULE_E, &cache_dir, &from_file)
        .await
        .unwrap();
    assert_eq!(r1.table, "fec_fitem_sched_e");
    assert_eq!(r1.tables, vec!["fec_fitem_sched_e"]);
    assert!(r1.cycles.is_empty());
    assert!(!r1.data_only);
    assert!(r1.rows > 100_000, "rows = {}", r1.rows);
    assert_eq!(r1.load_ids.len(), 1);
    assert_eq!(r1.dump_path, dump_path);
    // The expected ignorable error: the trigger function is not in the dump.
    assert!(
        r1.warnings
            .iter()
            .any(|w| w.contains("fec_fitem_sched_e_insert")),
        "{:?}",
        r1.warnings
    );
    assert_eq!(
        count(&t.pool, "SELECT count(*) FROM disclosure.fec_fitem_sched_e").await,
        r1.rows
    );
    assert_eq!(
        count(
            &t.pool,
            "SELECT count(*) FROM pg_index WHERE indrelid = 'disclosure.fec_fitem_sched_e'::regclass AND indisprimary"
        )
        .await,
        1
    );

    // Views in this namespace: the hardmoney-named one and the FEC-named
    // one with `cycle`; none for the dumps that are not restored.
    assert!(relation_exists(&t.pool, "independent_expenditures").await);
    assert!(relation_exists(&t.pool, "dump_schedule_e").await);
    assert!(!relation_exists(&t.pool, "dump_schedule_a").await);
    assert!(!relation_exists(&t.pool, "dump_schedule_b").await);
    assert!(!relation_exists(&t.pool, "dump_committee_history").await);
    assert_eq!(
        count(&t.pool, "SELECT count(*) FROM independent_expenditures").await,
        r1.rows
    );
    let (cycle_type,): (String,) = sqlx::query_as(
        "SELECT data_type::text FROM information_schema.columns \
         WHERE table_schema = current_schema() AND table_name = 'dump_schedule_e' AND column_name = 'cycle'",
    )
    .fetch_one(&t.pool)
    .await
    .unwrap();
    assert_eq!(cycle_type, "integer");
    assert_eq!(
        count(
            &t.pool,
            "SELECT count(*) FROM dump_schedule_e WHERE cycle <> election_cycle::int"
        )
        .await,
        0
    );
    assert!(count(&t.pool, "SELECT count(DISTINCT cycle) FROM dump_schedule_e").await > 10);
    // FEC column names survive in the view.
    assert!(
        count(
            &t.pool,
            "SELECT count(*) FROM dump_schedule_e WHERE exp_amt IS NOT NULL"
        )
        .await
            > 0
    );

    // The restore was recorded in this namespace's `loads`.
    assert_eq!(
        count(
            &t.pool,
            "SELECT count(*) FROM loads WHERE source = 'dump:schedule_e' AND mode = 'restore' AND cycle IS NULL AND source_url IS NULL"
        )
        .await,
        1
    );
    assert_eq!(
        count(
            &t.pool,
            "SELECT row_count FROM loads WHERE source = 'dump:schedule_e'"
        )
        .await,
        r1.rows
    );

    // -- A second restore is a clean refresh -------------------------------
    let r2 = dump::restore_with(&t.pool, &url, &dump::SCHEDULE_E, &cache_dir, &from_file)
        .await
        .unwrap();
    assert_eq!(r2.rows, r1.rows);
    assert!(
        !r2.warnings.iter().any(|w| w.contains("already exists")),
        "{:?}",
        r2.warnings
    );
    assert!(relation_exists(&t.pool, "independent_expenditures").await);
    assert_eq!(
        count(
            &t.pool,
            "SELECT count(*) FROM loads WHERE source = 'dump:schedule_e'"
        )
        .await,
        2
    );

    // -- Data-only restore, then hardmoney's indexes -----------------------
    let r3 = dump::restore_with(
        &t.pool,
        &url,
        &dump::SCHEDULE_E,
        &cache_dir,
        &from_file.clone().data_only(true),
    )
    .await
    .unwrap();
    assert_eq!(r3.rows, r1.rows);
    assert!(r3.data_only);
    assert_eq!(
        r3.indexes_skipped, 0,
        "the Schedule E archive has no INDEX entries"
    );
    assert_eq!(
        count(
            &t.pool,
            "SELECT count(*) FROM pg_index WHERE indrelid = 'disclosure.fec_fitem_sched_e'::regclass"
        )
        .await,
        0
    );
    assert_eq!(
        count(
            &t.pool,
            "SELECT count(*) FROM loads WHERE source = 'dump:schedule_e' AND mode = 'restore-data-only'"
        )
        .await,
        1
    );

    let err = dump::create_indexes(&t.pool, &dump::SCHEDULE_E, &[Cycle::new(2024).unwrap()])
        .await
        .unwrap_err();
    assert!(matches!(err, DumpError::NotPartitioned { .. }), "{err}");
    let err = dump::create_indexes(&t.pool, &dump::COMMITTEE_HISTORY, &[])
        .await
        .unwrap_err();
    assert!(matches!(err, DumpError::TableMissing { .. }), "{err}");

    let ix = dump::create_indexes(&t.pool, &dump::SCHEDULE_E, &[])
        .await
        .unwrap();
    assert_eq!(ix.tables, vec!["fec_fitem_sched_e"]);
    let created: Vec<&str> = ix.created.iter().map(|c| c.name.as_str()).collect();
    for expected in [
        "hm_fec_fitem_sched_e_sub_id_uidx",
        "hm_fec_fitem_sched_e_cmte_dt_idx",
        "hm_fec_fitem_sched_e_cand_idx",
        "hm_fec_fitem_sched_e_file_num_idx",
    ] {
        assert!(created.contains(&expected), "{created:?}");
    }
    // The trigram index depends on pg_trgm being installable here.
    assert_eq!(
        ix.created.len() + ix.skipped.len(),
        dump::SCHEDULE_E.indexes.len(),
        "{ix:?}"
    );
    assert!(ix.existing.is_empty());
    let ix2 = dump::create_indexes(&t.pool, &dump::SCHEDULE_E, &[])
        .await
        .unwrap();
    assert!(ix2.created.is_empty(), "{ix2:?}");
    assert_eq!(ix2.existing.len(), ix.created.len());

    let state = dump::table_state(&t.pool, &dump::SCHEDULE_E).await.unwrap();
    assert!(state.exists);
    assert!(state.partitions.is_empty());
    assert_eq!(state.index_count, i64::try_from(ix.created.len()).unwrap());
    let absent = dump::table_state(&t.pool, &dump::SCHEDULE_B).await.unwrap();
    assert!(!absent.exists);

    // -- A full restore on top of the indexed table skips the duplicate
    //    unique index (the archive's primary key covers sub_id).
    dump::restore_with(&t.pool, &url, &dump::SCHEDULE_E, &cache_dir, &from_file)
        .await
        .unwrap();
    let ix3 = dump::create_indexes(&t.pool, &dump::SCHEDULE_E, &[])
        .await
        .unwrap();
    assert!(
        ix3.skipped
            .iter()
            .any(|s| s.name == "hm_fec_fitem_sched_e_sub_id_uidx"
                && s.reason.contains("primary key")),
        "{ix3:?}"
    );

    let history = dump::restore_history(&t.pool).await.unwrap();
    assert_eq!(history.len(), 4);
    assert!(
        history
            .iter()
            .all(|h| h.dump == "schedule_e" && h.cycle.is_none())
    );
    assert_eq!(history[0].mode, "restore");
    assert_eq!(history[1].mode, "restore-data-only");
    assert!(
        history
            .windows(2)
            .all(|w| w[0].restored_at >= w[1].restored_at)
    );

    // -- Raw filing vs. the FEC's processed rows ---------------------------
    let err = dump::compare_filing(&t.pool, 424242).await.unwrap_err();
    assert!(matches!(err, DumpError::FilingNotIngested(424242)), "{err}");

    let bytes = std::fs::read(fixture("F24N_2011823.fec")).unwrap();
    bulk::ingest_filing_bytes(&t.pool, 2011823, &bytes, &hardmoney::ParseOptions::LENIENT)
        .await
        .unwrap();
    let cmp = dump::compare_filing(&t.pool, 2011823).await.unwrap();
    assert_eq!(cmp.filing_id, 2011823);
    assert_eq!(cmp.form_type, "F24N");
    assert_eq!(cmp.committee_id.as_deref(), Some("C00912865"));
    assert_eq!(cmp.raw_rows, 1);
    assert_eq!(cmp.raw_total.to_string(), "1074900.00");
    assert!(cmp.dump_newest_file_num.is_some());
    // The dump holds no Form 24 rows, and this filing postdates the local
    // archive anyway; either way the accounting must balance.
    assert_eq!(
        cmp.matched() + i64::try_from(cmp.only_raw.len()).unwrap(),
        cmp.raw_rows
    );
    assert_eq!(
        cmp.matched() + i64::try_from(cmp.only_dump.len()).unwrap(),
        cmp.dump_rows
    );
    if cmp.dump_rows == 0 {
        assert_eq!(cmp.only_raw, vec!["SE.4825"]);
        assert!(cmp.amount_mismatches.is_empty());
    }

    let _ = std::fs::remove_dir_all(&cache_dir);
    t.drop().await;
}
