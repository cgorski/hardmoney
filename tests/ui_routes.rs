//! The browser UI's routes (`/ui`, `/ui/assets/*`) and the database-free
//! filing tools (`/tools/*`), exercised through the routers without a
//! Postgres connection: both routers are state-free and are mounted here
//! exactly as `hardmoney::api::router` mounts them behind `--ui`.

#![cfg(feature = "api")]

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use hardmoney::api::routes::tools::{self, Limits};
use serde_json::{Value, json};
use tower::ServiceExt as _;

const FIXTURE: &[u8] = include_bytes!("fixtures/F3XA_2011827.fec");
/// Body lines in the fixture: one SA11AI, four SB21B, one SE.
const FIXTURE_LINES: usize = 6;

fn app() -> Router {
    app_with_limit(1024 * 1024)
}

fn app_with_limit(max_body_bytes: usize) -> Router {
    Router::new()
        .merge(hardmoney::ui::router())
        .merge(tools::router(Limits { max_body_bytes }))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, header::HeaderMap, Vec<u8>) {
    let resp = app.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = axum::body::to_bytes(resp.into_body(), 64 << 20)
        .await
        .unwrap();
    (status, headers, bytes.to_vec())
}

async fn get(app: &Router, uri: &str) -> (StatusCode, header::HeaderMap, Vec<u8>) {
    send(
        app,
        Request::builder().uri(uri).body(Body::empty()).unwrap(),
    )
    .await
}

async fn post_bytes(
    app: &Router,
    uri: &str,
    body: Vec<u8>,
) -> (StatusCode, header::HeaderMap, Vec<u8>) {
    send(
        app,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(body))
            .unwrap(),
    )
    .await
}

async fn post_json(
    app: &Router,
    uri: &str,
    body: &Value,
) -> (StatusCode, header::HeaderMap, Vec<u8>) {
    send(
        app,
        Request::builder()
            .method("POST")
            .uri(uri)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap(),
    )
    .await
}

fn json(bytes: &[u8]) -> Value {
    serde_json::from_slice(bytes)
        .unwrap_or_else(|e| panic!("not JSON ({e}): {}", String::from_utf8_lossy(bytes)))
}

async fn parsed_fixture(app: &Router) -> Value {
    let (status, _, body) = post_bytes(app, "/tools/parse", FIXTURE.to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    json(&body)
}

fn header_str<'a>(headers: &'a header::HeaderMap, name: &str) -> &'a str {
    headers
        .get(name)
        .unwrap_or_else(|| panic!("missing header {name}"))
        .to_str()
        .unwrap()
}

// ---------------------------------------------------------------------------
// Static shell and assets
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ui_shell_is_served_for_the_root_and_for_client_side_routes() {
    let app = app();
    for uri in ["/ui", "/ui/", "/ui/data", "/ui/data/committees/C00944124"] {
        let (status, headers, body) = get(&app, uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}");
        assert_eq!(
            header_str(&headers, "content-type"),
            "text/html; charset=utf-8",
            "{uri}"
        );
        assert!(headers.contains_key("content-security-policy"), "{uri}");
        let html = String::from_utf8(body).unwrap();
        assert!(html.contains("<!doctype html>"), "{uri}");
        assert!(html.contains("/ui/assets/app.js"), "{uri}");
    }
}

#[tokio::test]
async fn ui_assets_are_served_with_types_and_etags() {
    let app = app();
    let (status, headers, body) = get(&app, "/ui/assets/app.js").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_str(&headers, "content-type"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(header_str(&headers, "cache-control"), "public, no-cache");
    let etag = header_str(&headers, "etag").to_string();
    assert!(etag.starts_with('"') && etag.ends_with('"'), "{etag}");
    assert!(String::from_utf8(body).unwrap().contains("import"));

    let (status, _, body) = send(
        &app,
        Request::builder()
            .uri("/ui/assets/app.js")
            .header(header::IF_NONE_MATCH, etag)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_MODIFIED);
    assert!(body.is_empty());

    let (status, headers, _) = get(&app, "/ui/assets/app.css").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_str(&headers, "content-type"),
        "text/css; charset=utf-8"
    );

    let (status, _, _) = get(&app, "/ui/assets/does-not-exist.js").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // Every embedded asset is reachable at its path.
    for path in hardmoney::ui::asset_paths() {
        let (status, _, body) = get(&app, &format!("/ui/assets/{path}")).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(
            body,
            hardmoney::ui::asset_bytes(&path).unwrap().into_owned(),
            "{path} bytes differ"
        );
    }
}

// ---------------------------------------------------------------------------
// /tools/parse
// ---------------------------------------------------------------------------

#[tokio::test]
async fn parse_returns_lines_validation_and_reconciliation() {
    let app = app();
    let doc = parsed_fixture(&app).await;

    assert_eq!(doc["form_type"], "F3XA");
    assert_eq!(doc["base_form_type"], "F3X");
    assert_eq!(doc["is_amendment"], true);
    assert_eq!(doc["amends_filing"], 1_991_972);
    assert_eq!(doc["version"], "8.5");
    assert_eq!(doc["header"]["soft_name"], "FECfile");
    assert_eq!(doc["line_count"], FIXTURE_LINES);
    assert_eq!(doc["lines"].as_array().unwrap().len(), FIXTURE_LINES);
    assert_eq!(doc["skipped"], json!([]));
    assert_eq!(doc["tables"], json!({ "SchA": 1, "SchB": 4, "SchE": 1 }));

    let summary = &doc["summary"];
    assert_eq!(summary["table"], "F3X");
    assert_eq!(summary["line_no"], 2);
    assert_eq!(summary["fields"]["filer_committee_id_number"], "C00944124");
    assert_eq!(summary["fields"]["committee_name"], "REVIVE OREGON");

    let first = &doc["lines"][0];
    assert_eq!(first["line_no"], 3);
    assert_eq!(first["table"], "SchA");
    assert_eq!(first["raw_form_type"], "SA11AI");
    assert_eq!(first["fields"]["contribution_amount"], "20000.00");
    // Every field value is a string.
    for (name, v) in first["fields"].as_object().unwrap() {
        assert!(v.is_string(), "{name} is {v}");
    }

    let findings = doc["validation"]["findings"].as_array().unwrap();
    let errors = findings.iter().filter(|f| f["severity"] == "error").count();
    assert_eq!(errors, 0, "{findings:#?}");

    let checks = doc["reconciliation"]["checks"].as_array().unwrap();
    assert!(!checks.is_empty());
    assert!(doc["reconcile_error"].is_null());
    assert_eq!(doc["reconciliation"]["form"], "F3X");
    let receipts = checks
        .iter()
        .find(|c| c["field"] == "col_a_individuals_itemized" && c["column"] == "A")
        .expect("11(a)(i) check");
    assert_eq!(receipts["reported"], "20000.00");
    assert_eq!(receipts["expected"], "20000.00");
    assert_eq!(receipts["delta"], "0.00");
    assert_eq!(receipts["lines_summed"], 1);
}

#[tokio::test]
async fn parse_rejects_garbage_and_empty_bodies_with_400_json() {
    let app = app();
    let (status, _, body) = post_bytes(&app, "/tools/parse", b"not a filing".to_vec()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = json(&body)["error"].as_str().unwrap().to_string();
    assert!(msg.starts_with("could not parse filing"), "{msg}");

    let (status, _, body) = post_bytes(&app, "/tools/parse", Vec::new()).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(json(&body)["error"].as_str().unwrap().contains("cover"));
}

#[tokio::test]
async fn oversized_bodies_are_refused_with_413() {
    let app = app_with_limit(512);
    let (status, _, body) = post_bytes(&app, "/tools/parse", FIXTURE.to_vec()).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let msg = json(&body)["error"].as_str().unwrap().to_string();
    assert!(msg.contains("512 byte limit"), "{msg}");
    assert!(msg.contains("CLI"), "{msg}");

    let big = json!({ "version": "8.5", "summary": { "table": "F3X", "fields": { "form_type": "F3XN", "committee_name": "x".repeat(2000) } } });
    let (status, _, _) = post_json(&app, "/tools/validate", &big).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
}

/// The configured cap is the only cap: a document between axum's 2 MiB
/// extractor default and the configured limit must be accepted.
#[tokio::test]
async fn bodies_up_to_the_configured_cap_are_accepted() {
    let app = app_with_limit(8 * 1024 * 1024);
    let mut lines = Vec::new();
    for i in 0..12_000 {
        lines.push(json!({ "table": "SchA", "fields": {
            "form_type": "SA11AI", "filer_committee_id_number": "C00123456",
            "transaction_id": format!("A{i}"), "contribution_amount": "1.00",
            "contributor_last_name": "x".repeat(60), "contributor_first_name": "y".repeat(60),
            "contributor_street_1": "z".repeat(60),
        } }));
    }
    let doc = json!({
        "version": "8.5",
        "summary": { "table": "F3X", "fields": { "form_type": "F3XN", "filer_committee_id_number": "C00123456" } },
        "lines": lines,
    });
    let body = serde_json::to_vec(&doc).unwrap();
    assert!(body.len() > 2 * 1024 * 1024, "{} bytes", body.len());

    let (status, headers, bytes) = post_json(&app, "/tools/write", &doc).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    // The synthetic lines leave required fields blank; the count is still
    // reported, and the file is still written.
    let errors: usize = header_str(&headers, "x-hardmoney-validation-errors")
        .parse()
        .unwrap();
    assert!(errors > 0);
    assert_eq!(String::from_utf8_lossy(&bytes).lines().count(), 12_002);

    let (status, _, _) = post_bytes(&app, "/tools/parse", bytes).await;
    assert_eq!(status, StatusCode::OK);
}

// ---------------------------------------------------------------------------
// /tools/write, /tools/validate, /tools/reconcile
// ---------------------------------------------------------------------------

#[tokio::test]
async fn write_round_trips_the_parsed_document() {
    let app = app();
    let doc = parsed_fixture(&app).await;

    // The parse output is a valid document as-is (extra keys are ignored).
    let (status, headers, bytes) = post_json(&app, "/tools/write", &doc).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(
        header_str(&headers, "content-type"),
        "application/octet-stream"
    );
    assert_eq!(
        header_str(&headers, "content-disposition"),
        "attachment; filename=\"F3XA-C00944124.fec\""
    );
    assert_eq!(header_str(&headers, "x-hardmoney-validation-errors"), "0");

    // Re-parsing the written bytes gives the same lines and fields.
    let (status, _, body) = post_bytes(&app, "/tools/parse", bytes.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let again = json(&body);
    assert_eq!(again["line_count"], FIXTURE_LINES);
    assert_eq!(again["header"], doc["header"]);
    assert_eq!(again["summary"]["fields"], doc["summary"]["fields"]);
    for (a, b) in again["lines"]
        .as_array()
        .unwrap()
        .iter()
        .zip(doc["lines"].as_array().unwrap())
    {
        assert_eq!(a["table"], b["table"]);
        assert_eq!(a["raw_form_type"], b["raw_form_type"]);
        assert_eq!(a["fields"], b["fields"]);
    }
    // And the library agrees byte for byte with its own writer.
    let direct = hardmoney::Filing::parse_bytes(FIXTURE).unwrap().to_fec();
    assert_eq!(bytes, direct);
}

#[tokio::test]
async fn an_edit_in_the_document_changes_the_written_file_and_the_checks() {
    let app = app();
    let mut doc = parsed_fixture(&app).await;

    // Change a contributor's city and the reported 11(a)(i) total.
    doc["lines"][0]["fields"]["contributor_city"] = json!("PORTLAND");
    doc["summary"]["fields"]["col_a_individuals_itemized"] = json!("19999.00");

    let (status, headers, bytes) = post_json(&app, "/tools/write", &doc).await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.contains("\u{1c}PORTLAND\u{1c}"), "edited city missing");
    assert!(
        !text.contains("\u{1c}SANDY\u{1c}"),
        "original city still present"
    );
    assert!(text.contains("\u{1c}19999.00\u{1c}"));
    // Still no validation errors: the edits are well-formed values.
    assert_eq!(header_str(&headers, "x-hardmoney-validation-errors"), "0");

    // Reconcile now reports the 11(a)(i) mismatch.
    let (status, _, body) = post_json(&app, "/tools/reconcile", &doc).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let rec = json(&body);
    let check = rec["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["field"] == "col_a_individuals_itemized" && c["column"] == "A")
        .unwrap();
    assert_eq!(check["reported"], "19999.00");
    assert_eq!(check["expected"], "20000.00");
    assert_eq!(check["delta"], "-1.00");
}

#[tokio::test]
async fn validate_reports_a_bad_filer_id_by_rule() {
    let app = app();
    let mut doc = parsed_fixture(&app).await;
    doc["summary"]["fields"]["filer_committee_id_number"] = json!("NOTANID");

    let (status, _, body) = post_json(&app, "/tools/validate", &doc).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let v = json(&body);
    let finding = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["rule"] == "filer_id_format")
        .unwrap_or_else(|| panic!("no filer_id_format finding in {v:#}"));
    assert_eq!(finding["severity"], "error");
    assert_eq!(finding["line_no"], 2);
    assert_eq!(finding["field"], "filer_committee_id_number");

    // The written file carries the error count for the client.
    let (status, headers, _) = post_json(&app, "/tools/write", &doc).await;
    assert_eq!(status, StatusCode::OK);
    let n: usize = header_str(&headers, "x-hardmoney-validation-errors")
        .parse()
        .unwrap();
    assert!(n >= 1);
}

#[tokio::test]
async fn a_minimal_hand_written_document_writes_and_numbers_its_lines() {
    let app = app();
    let doc = json!({
        "version": "8.5",
        "header": { "soft_name": "hardmoney-ui-test", "soft_ver": "1" },
        "summary": { "table": "F3X", "fields": { "form_type": "F3XN", "filer_committee_id_number": "C00123456", "committee_name": "TEST PAC" } },
        "lines": [
            { "table": "SchA", "fields": { "form_type": "SA11AI", "filer_committee_id_number": "C00123456", "contribution_amount": "250.00" } },
            { "table": "SchB", "fields": { "form_type": "SB21B", "filer_committee_id_number": "C00123456", "expenditure_amount": "10.00" } }
        ]
    });
    let (status, headers, bytes) = post_json(&app, "/tools/write", &doc).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&bytes)
    );
    assert_eq!(
        header_str(&headers, "content-disposition"),
        "attachment; filename=\"F3XN-C00123456.fec\""
    );
    let text = String::from_utf8(bytes).unwrap();
    assert!(text.starts_with("HDR\u{1c}FEC\u{1c}8.5\u{1c}hardmoney-ui-test\u{1c}1\u{1c}"));
    assert_eq!(text.lines().count(), 4);

    // Validation numbers the body lines from 3 when the document gives none.
    let (status, _, body) = post_json(&app, "/tools/validate", &doc).await;
    assert_eq!(status, StatusCode::OK);
    let v = json(&body);
    let lines: Vec<u64> = v["findings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|f| f["line_no"].as_u64())
        .collect();
    assert!(lines.iter().all(|&l| (1..=4).contains(&l)), "{lines:?}");
}

#[tokio::test]
async fn malformed_documents_are_400_with_a_message() {
    let app = app();

    // Not JSON at all.
    let (status, _, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/tools/validate")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{not json"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = json(&body)["error"].as_str().unwrap().to_string();
    assert!(msg.to_lowercase().contains("json"), "{msg}");

    // JSON, but missing required keys: the message names the first one.
    let (status, _, body) = post_json(&app, "/tools/validate", &json!({ "summary": {} })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = json(&body)["error"].as_str().unwrap().to_string();
    assert!(msg.contains("missing field `table`"), "{msg}");
    let (status, _, body) = post_json(
        &app,
        "/tools/validate",
        &json!({ "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } } }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = json(&body)["error"].as_str().unwrap().to_string();
    assert!(msg.contains("missing field `version`"), "{msg}");

    // Wrong content type.
    let (status, _, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri("/tools/validate")
            .header(header::CONTENT_TYPE, "text/plain")
            .body(Body::from("{}"))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert!(json(&body)["error"].is_string());

    // Consistent JSON, inconsistent filing: an unknown field on line 12.
    let (status, _, body) = post_json(
        &app,
        "/tools/write",
        &json!({
            "version": "8.5",
            "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } },
            "lines": [ { "table": "SchA", "line_no": 12, "fields": { "form_type": "SA11AI", "nope": "1" } } ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = json(&body)["error"].as_str().unwrap().to_string();
    assert!(msg.starts_with("line 12:"), "{msg}");
    assert!(msg.contains("nope"), "{msg}");

    // A form without reconciliation rules.
    let (status, _, body) = post_json(
        &app,
        "/tools/reconcile",
        &json!({ "version": "8.5", "summary": { "table": "F99", "fields": { "form_type": "F99", "filer_committee_id_number": "C00123456" } } }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        json(&body)["error"]
            .as_str()
            .unwrap()
            .contains("F3X, F3, F3P")
    );
}

// ---------------------------------------------------------------------------
// /tools/spec
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spec_returns_the_layout_with_field_specs() {
    let app = app();
    let (status, _, body) = get(&app, "/tools/spec/SchA?version=8.5").await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let spec = json(&body);
    assert_eq!(spec["table"], "SchA");
    assert_eq!(spec["version"], "8.5");
    assert_eq!(spec["bundled_spec_version"], "8.5");
    let fields = spec["fields"].as_array().unwrap();
    let amount = fields
        .iter()
        .find(|f| f["name"] == "contribution_amount")
        .unwrap();
    assert_eq!(amount["spec"]["kind"], "amount");
    assert!(amount["column"].as_u64().unwrap() > 0);
    assert!(
        fields
            .iter()
            .any(|f| f["name"] == "form_type" && f["column"] == 0)
    );

    // Default version is the bundled one; table names are case-insensitive.
    let (status, _, body) = get(&app, "/tools/spec/scha").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json(&body)["version"], "8.5");

    let (status, _, body) = get(&app, "/tools/spec/NotATable").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(json(&body)["error"].as_str().unwrap().contains("NotATable"));

    let (status, _, _) = get(&app, "/tools/spec/F3X?version=1.0").await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let (status, _, _) = get(&app, "/tools/spec/F3X?version=abc").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

// ---------------------------------------------------------------------------
// The full API router with `ui` on
// ---------------------------------------------------------------------------

/// Building the complete router only needs a pool handle, not a live
/// connection: `PgPool::connect_lazy` does not touch the network. This
/// checks the wiring (`ui` mounts the routes; the key guards the tools but
/// not the shell) without a database.
#[tokio::test]
async fn full_router_mounts_ui_and_tools_only_when_enabled() {
    use hardmoney::api::ApiConfig;

    let pool = sqlx::PgPool::connect_lazy("postgres://nobody@127.0.0.1:1/none").unwrap();
    let bind = "127.0.0.1:0".parse().unwrap();

    let off = hardmoney::api::router(pool.clone(), &ApiConfig::new(bind));
    let (status, _, _) = get(&off, "/ui").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _, _) = post_bytes(&off, "/tools/parse", FIXTURE.to_vec()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    let config = ApiConfig::new(bind).api_key(Some("k".into())).ui(true);
    assert_eq!(config.max_body_bytes, ApiConfig::UI_MAX_BODY_BYTES);
    let on = hardmoney::api::router(pool, &config);

    // The shell and assets load without a key.
    let (status, _, _) = get(&on, "/ui").await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, _) = get(&on, "/ui/assets/app.js").await;
    assert_eq!(status, StatusCode::OK);

    // The tools do not.
    let (status, _, body) = post_bytes(&on, "/tools/parse", FIXTURE.to_vec()).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(json(&body)["error"], "missing or invalid API key");

    let (status, _, body) = send(
        &on,
        Request::builder()
            .method("POST")
            .uri("/tools/parse")
            .header("x-api-key", "k")
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(FIXTURE))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(json(&body)["line_count"], FIXTURE_LINES);

    let (status, _, _) = get(&on, "/tools/spec/SchA?api_key=k").await;
    assert_eq!(status, StatusCode::OK);
}
