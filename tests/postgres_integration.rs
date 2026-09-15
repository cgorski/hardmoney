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
    let path = std::env::temp_dir().join(format!(
        "hardmoney-it-cn-{}-{}.zip",
        std::process::id(),
        rows.len()
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
    assert!(err.is_ok(), "2 rows is under the default threshold");
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
