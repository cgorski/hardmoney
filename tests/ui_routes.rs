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

    // A record whose `table` and form-type token disagree: 400, naming
    // both, never a 500.
    let (status, _, body) = post_json(
        &app,
        "/tools/write",
        &json!({
            "version": "8.5",
            "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } },
            "lines": [ { "table": "SchB", "fields": { "form_type": "SA11AI", "expenditure_amount": "1.00" } } ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = json(&body)["error"].as_str().unwrap().to_string();
    assert_eq!(
        msg,
        "line 3: table 'SchB' does not match form type 'SA11AI', which is a SchA record"
    );

    // The document's `version` and the header's `fec_version_raw` disagree.
    let (status, _, body) = post_json(
        &app,
        "/tools/write",
        &json!({
            "version": "8.5",
            "header": { "fec_version_raw": "6.4" },
            "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let msg = json(&body)["error"].as_str().unwrap().to_string();
    assert!(msg.contains("6.4") && msg.contains("8.5"), "{msg}");
}

/// A JSON document is bounded by what it would *write*, not by its own
/// size: 2,000 sixteen-byte records (`{"table":"SchA"}`) would build
/// 2,000 full-width (45-cell) Schedule A lines, so with a small cap they
/// are refused before any is built -- ahead of every per-line check.
#[tokio::test]
async fn documents_that_would_write_past_the_cap_are_413() {
    let lines: Vec<Value> = (0..2_000).map(|_| json!({ "table": "SchA" })).collect();
    let doc = json!({
        "version": "8.5",
        "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } },
        "lines": lines
    });
    // The JSON itself is under this cap; the written filing is not.
    let cap = 80 * 1024;
    let app = app_with_limit(cap);
    let json_len = serde_json::to_vec(&doc).unwrap().len();
    assert!(json_len < cap, "{json_len}");
    for route in ["/tools/write", "/tools/validate", "/tools/reconcile"] {
        let (status, _, body) = post_json(&app, route, &doc).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{route}");
        let msg = json(&body)["error"].as_str().unwrap().to_string();
        assert!(msg.contains("2001 record(s)"), "{msg}");
        assert!(msg.contains("81920 byte limit"), "{msg}");
    }

    // With room to write it, the same records are checked one by one: a
    // schedule record needs its form-type token (there is no default one).
    let roomy = app_with_limit(1 << 20);
    let (status, _, body) = post_json(&roomy, "/tools/write", &doc).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{}",
        String::from_utf8_lossy(&body)
    );
    assert_eq!(
        json(&body)["error"],
        "line 3: fields.form_type is required for a SchA record (the form-type token as filed, for example SA11AI)"
    );
    let lines: Vec<Value> = (0..2_000)
        .map(|_| json!({ "table": "SchA", "fields": { "form_type": "SA11AI" } }))
        .collect();
    let doc = json!({
        "version": "8.5",
        "summary": { "table": "F3X", "fields": { "form_type": "F3XN" } },
        "lines": lines
    });
    let (status, _, body) = post_json(&roomy, "/tools/write", &doc).await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert!(body.len() > cap, "the written file really is that big");
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

/// In allow-list mode a browser at an allowed origin must be able to send
/// the API key and a JSON body (both need a preflight) and to read the two
/// headers `POST /tools/write` sets. The `CorsLayer` is outermost, so a
/// preflight is answered before the key check and never reaches a handler
/// or the database.
#[tokio::test]
async fn cors_allow_list_admits_the_api_key_and_json_and_exposes_the_write_headers() {
    use hardmoney::api::ApiConfig;

    let pool = sqlx::PgPool::connect_lazy("postgres://nobody@127.0.0.1:1/none").unwrap();
    let bind = "127.0.0.1:0".parse().unwrap();
    let config = ApiConfig::new(bind)
        .cors_origins(vec!["https://example.org".into()])
        .api_key(Some("k".into()))
        .ui(true);
    config.validate().unwrap();
    let app = hardmoney::api::router(pool.clone(), &config);

    let preflight = |origin: &'static str| {
        Request::builder()
            .method("OPTIONS")
            .uri("/tools/validate")
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
            .header(
                header::ACCESS_CONTROL_REQUEST_HEADERS,
                "x-api-key, content-type",
            )
            .body(Body::empty())
            .unwrap()
    };

    let (status, headers, _) = send(&app, preflight("https://example.org")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_str(&headers, "access-control-allow-origin"),
        "https://example.org"
    );
    let allowed_headers = header_str(&headers, "access-control-allow-headers").to_ascii_lowercase();
    assert!(allowed_headers.contains("x-api-key"), "{allowed_headers}");
    assert!(
        allowed_headers.contains("content-type"),
        "{allowed_headers}"
    );
    let methods = header_str(&headers, "access-control-allow-methods");
    assert!(methods.contains("POST"), "{methods}");
    assert!(methods.contains("GET"), "{methods}");

    // An origin off the list gets no `Access-Control-Allow-Origin`, which
    // is the header a browser blocks on (tower-http still lists the
    // allowed headers and methods; without a matching origin they are
    // inert).
    let (_, headers, _) = send(&app, preflight("https://evil.example")).await;
    assert!(headers.get("access-control-allow-origin").is_none());

    // A real response carries the exposed headers (here from /health,
    // which needs neither a key nor a database).
    let (status, headers, _) = send(
        &app,
        Request::builder()
            .uri("/health")
            .header(header::ORIGIN, "https://example.org")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let exposed = header_str(&headers, "access-control-expose-headers").to_ascii_lowercase();
    assert!(exposed.contains("content-disposition"), "{exposed}");
    assert!(
        exposed.contains("x-hardmoney-validation-errors"),
        "{exposed}"
    );

    // Without --ui the API is read-only and POST is not offered.
    let read_only = hardmoney::api::router(
        pool,
        &ApiConfig::new(bind).cors_origins(vec!["https://example.org".into()]),
    );
    let (_, headers, _) = send(&read_only, preflight("https://example.org")).await;
    let methods = header_str(&headers, "access-control-allow-methods");
    assert!(!methods.contains("POST"), "{methods}");
}

/// `router` must not panic on an origin `validate` would reject (the
/// CLI validates first; a library caller might not): the entry is dropped
/// and the rest of the list still works.
#[tokio::test]
async fn cors_router_skips_an_invalid_origin_instead_of_panicking() {
    use hardmoney::api::ApiConfig;

    let pool = sqlx::PgPool::connect_lazy("postgres://nobody@127.0.0.1:1/none").unwrap();
    let config = ApiConfig::new("127.0.0.1:0".parse().unwrap()).cors_origins(vec![
        "*".into(),
        "https://example.org/".into(),
        "https://example.org".into(),
    ]);
    assert!(config.validate().is_err());
    let app = hardmoney::api::router(pool, &config);
    let (status, headers, _) = send(
        &app,
        Request::builder()
            .uri("/health")
            .header(header::ORIGIN, "https://example.org")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        header_str(&headers, "access-control-allow-origin"),
        "https://example.org"
    );
}

/// A one-shot local HTTP server answering `connections` requests with
/// `body`, returning the request lines it saw.
fn serve_bytes(
    body: &'static [u8],
    connections: usize,
) -> (std::net::SocketAddr, std::thread::JoinHandle<Vec<String>>) {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for _ in 0..connections {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = vec![0u8; 16384];
            let n = stream.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..n]).to_string();
            seen.push(request.lines().next().unwrap_or("").to_string());
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(body).unwrap();
        }
        seen
    });
    (addr, handle)
}

/// `GET /tools/fetch/{id}` downloads from the document store the API was
/// configured with (`ApiConfig::endpoints`, here a local server standing
/// in for a mirror), not from `docquery.fec.gov`.
#[tokio::test]
async fn tools_fetch_downloads_from_the_configured_docquery_base() {
    use axum::extract::Extension;
    use hardmoney::fec::{Endpoints, Url};

    let (addr, handle) = serve_bytes(FIXTURE, 1);
    let mirror = Endpoints::default()
        .with_docquery_base(Url::parse(&format!("http://{addr}/mirror")).unwrap());
    // The same layer `hardmoney::api::router` adds from `ApiConfig`.
    let app = Router::new()
        .merge(tools::router(Limits {
            max_body_bytes: 1024 * 1024,
        }))
        .layer(Extension(mirror));

    let (status, _, body) = get(&app, "/tools/fetch/2011827").await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let doc = json(&body);
    assert_eq!(doc["line_count"], FIXTURE_LINES);
    assert_eq!(doc["form_type"], "F3XA");
    let seen = handle.join().unwrap();
    assert_eq!(seen, ["GET /mirror/dcdev/posted/2011827.fec HTTP/1.1"]);
}

/// The tools router pointed at a local document store with `max_body_bytes`
/// as its cap.
fn fetch_app(addr: std::net::SocketAddr, max_body_bytes: usize) -> Router {
    use axum::extract::Extension;
    use hardmoney::fec::{Endpoints, Url};
    let mirror = Endpoints::default()
        .with_docquery_base(Url::parse(&format!("http://{addr}/mirror")).unwrap());
    Router::new()
        .merge(tools::router(Limits { max_body_bytes }))
        .layer(Extension(mirror))
}

/// A filing exactly at the cap is served; one byte over is 413.
#[tokio::test]
async fn tools_fetch_accepts_a_filing_exactly_at_the_cap() {
    let (addr, handle) = serve_bytes(FIXTURE, 1);
    let (status, _, body) = get(&fetch_app(addr, FIXTURE.len()), "/tools/fetch/2011827").await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(json(&body)["line_count"], FIXTURE_LINES);
    handle.join().unwrap();

    let (addr, handle) = serve_bytes(FIXTURE, 1);
    let (status, _, body) = get(&fetch_app(addr, FIXTURE.len() - 1), "/tools/fetch/2011827").await;
    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let message = json(&body)["error"].as_str().unwrap().to_string();
    assert!(message.contains("2011827"), "{message}");
    assert!(
        message.contains(&(FIXTURE.len() - 1).to_string()),
        "{message}"
    );
    assert!(message.contains("hardmoney parse 2011827"), "{message}");
    handle.join().unwrap();
}

/// A server that announces a body far over the cap is refused from its
/// `Content-Length` alone: the handler never reads the body.
#[tokio::test]
async fn tools_fetch_refuses_an_oversized_content_length_without_reading_the_body() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = vec![0u8; 16384];
        let _ = stream.read(&mut buf).unwrap();
        // Announces 100 MB, sends a few bytes, then holds the connection
        // open: a client that waits for the body would hang here.
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: 100000000\r\nConnection: close\r\n\r\nHDR",
            )
            .unwrap();
        // Ends when the client closes (read returns 0 or an error).
        let mut sink = [0u8; 64];
        while matches!(stream.read(&mut sink), Ok(n) if n > 0) {}
    });

    let app = fetch_app(addr, 1024 * 1024);
    let (status, _, body) = tokio::time::timeout(
        std::time::Duration::from_secs(20),
        get(&app, "/tools/fetch/2010101"),
    )
    .await
    .expect("the handler must answer without waiting for the body");
    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let message = json(&body)["error"].as_str().unwrap().to_string();
    assert!(message.contains("100000000 bytes"), "{message}");
    handle.join().unwrap();
}

/// A chunked body of unknown length is cut off one byte past the cap, so
/// a client's choice of filing id can never make the server buffer more
/// than its own limit.
#[tokio::test]
async fn tools_fetch_stops_reading_an_unbounded_body_at_the_cap() {
    use std::io::{Read, Write};
    const CAP: usize = 256 * 1024;
    // The server gives up on its own well past the cap, so a regression
    // fails an assertion instead of hanging the test.
    const SERVER_GIVES_UP_AT: usize = 64 * 1024 * 1024;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut buf = vec![0u8; 16384];
        let _ = stream.read(&mut buf).unwrap();
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        let chunk = vec![b'x'; 64 * 1024];
        let header = format!("{:x}\r\n", chunk.len());
        let mut written = 0usize;
        while written < SERVER_GIVES_UP_AT {
            if stream.write_all(header.as_bytes()).is_err()
                || stream.write_all(&chunk).is_err()
                || stream.write_all(b"\r\n").is_err()
            {
                break;
            }
            written += chunk.len();
        }
        written
    });

    let app = fetch_app(addr, CAP);
    let (status, _, body) = tokio::time::timeout(
        std::time::Duration::from_secs(60),
        get(&app, "/tools/fetch/2010101"),
    )
    .await
    .expect("the handler must give up at the cap");
    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "{}",
        String::from_utf8_lossy(&body)
    );
    let written = handle.join().unwrap();
    // Socket buffers absorb some chunks after the client stops reading;
    // what must not happen is the server streaming on towards its limit.
    assert!(
        written < SERVER_GIVES_UP_AT / 4,
        "the client kept reading: server wrote {written} bytes against a {CAP} byte cap"
    );
}

// ---------------------------------------------------------------------------
// Accessibility invariants of the shell and the assets (WCAG 2.1 AA)
// ---------------------------------------------------------------------------
//
// String-level checks that hold the line on what the Section 508 review in
// `book/src/accessibility.md` verified with axe-core and a keyboard
// walkthrough. They run without a browser, so they cannot prove
// conformance; they catch the regressions that are cheap to catch.

async fn shell_html() -> String {
    let (status, _, body) = get(&app(), "/ui/").await;
    assert_eq!(status, StatusCode::OK);
    String::from_utf8(body).unwrap()
}

fn asset_text(path: &str) -> String {
    String::from_utf8(hardmoney::ui::asset_bytes(path).unwrap().into_owned()).unwrap()
}

fn js_assets() -> Vec<(String, String)> {
    hardmoney::ui::asset_paths()
        .filter(|p| p.ends_with(".js"))
        .map(|p| (p.to_string(), asset_text(&p)))
        .collect()
}

/// 3.1.1 language, 2.4.1 bypass blocks, 2.4.2 title, 1.3.1 landmarks.
#[tokio::test]
async fn shell_has_lang_title_skip_link_and_one_main() {
    let html = shell_html().await;
    let re = |p: &str| regex::Regex::new(p).unwrap();
    assert!(
        re(r#"<html[^>]*\slang="[a-z]{2}"#).is_match(&html),
        "missing <html lang>"
    );
    assert!(
        re(r"<title>[^<]+</title>").is_match(&html),
        "missing non-empty <title>"
    );
    assert!(
        re(r##"<a[^>]*class="skip-link"[^>]*href="#main""##).is_match(&html),
        "missing skip link to #main"
    );
    assert_eq!(
        re(r"<main[\s>]").find_iter(&html).count(),
        1,
        "exactly one <main>"
    );
    assert!(
        re(r#"<main[^>]*\sid="main"[^>]*\stabindex="-1""#).is_match(&html),
        "<main id=main> must be focusable (tabindex=-1) as the skip-link target"
    );
    assert!(
        re(r#"<nav[^>]*\saria-label="#).is_match(&html),
        "<nav> needs a name"
    );
    assert!(html.contains("<header"), "missing <header> landmark");
    // Live regions exist before any message is written into them (4.1.3).
    assert!(re(r#"<[^>]*\brole="status"[^>]*aria-live="polite""#).is_match(&html));
    assert!(re(r#"<[^>]*\brole="alert""#).is_match(&html));
    // The dialog is modal, named, and described (4.1.2).
    assert!(
        re(r#"<dialog[^>]*aria-labelledby="api-key-title"[^>]*aria-modal="true""#).is_match(&html)
            || re(r#"<dialog[^>]*aria-modal="true"[^>]*aria-labelledby="api-key-title""#)
                .is_match(&html),
        "dialog must have aria-labelledby and aria-modal"
    );
}

/// 2.4.3 focus order: no positive tabindex anywhere (shell or templates).
#[tokio::test]
async fn no_positive_tabindex_in_shell_or_scripts() {
    let positive_attr = regex::Regex::new(r#"(?i)tabindex\s*=\s*["']?\s*[1-9]"#).unwrap();
    let positive_prop = regex::Regex::new(r"\.tabIndex\s*=\s*[1-9]").unwrap();
    let html = shell_html().await;
    assert!(
        !positive_attr.is_match(&html),
        "positive tabindex in index.html"
    );
    for (path, js) in js_assets() {
        assert!(
            !positive_attr.is_match(&js),
            "positive tabindex attribute in {path}"
        );
        assert!(
            !positive_prop.is_match(&js),
            "positive tabIndex assignment in {path}"
        );
    }
}

/// 1.1.1 text alternatives: every image or inline SVG the UI could emit is
/// either named or hidden from assistive technology.
#[tokio::test]
async fn images_and_svgs_are_named_or_hidden() {
    let tag = regex::Regex::new(r"(?is)<(img|svg)\b[^>]*>").unwrap();
    let named = regex::Regex::new(
        r#"(?i)\b(alt|aria-label|aria-labelledby)\s*=|aria-hidden="true"|role="presentation""#,
    )
    .unwrap();
    let mut sources = vec![("index.html".to_string(), shell_html().await)];
    sources.extend(js_assets());
    for (path, text) in sources {
        for m in tag.find_iter(&text) {
            assert!(
                named.is_match(m.as_str()),
                "{path}: {} has no accessible name and is not aria-hidden",
                m.as_str()
            );
        }
    }
}

/// 1.3.1 / 4.1.2: every form control in the shell has a label. Inputs need
/// a `<label for>` or an aria name; buttons need text or an aria-label.
#[tokio::test]
async fn shell_controls_are_labelled() {
    let html = shell_html().await;
    let input = regex::Regex::new(r"(?is)<(input|select)\b[^>]*>").unwrap();
    let id = regex::Regex::new(r#"\bid="([^"]+)""#).unwrap();
    let aria = regex::Regex::new(r#"\baria-label(ledby)?="[^"]+""#).unwrap();
    let mut controls = 0;
    for m in input.find_iter(&html) {
        controls += 1;
        let tag = m.as_str();
        let labelled = aria.is_match(tag)
            || id
                .captures(tag)
                .is_some_and(|c| html.contains(&format!("<label for=\"{}\"", &c[1])));
        assert!(labelled, "unlabelled control in index.html: {tag}");
    }
    let button = regex::Regex::new(r"(?is)<button\b([^>]*)>(.*?)</button>").unwrap();
    let strip = regex::Regex::new(r"<[^>]+>").unwrap();
    for c in button.captures_iter(&html) {
        controls += 1;
        let text = strip.replace_all(&c[2], "").trim().to_string();
        assert!(
            !text.is_empty() || aria.is_match(&c[1]),
            "button without a name in index.html: {}",
            &c[0]
        );
    }
    assert!(
        controls >= 5,
        "expected the shell's controls to be checked, saw {controls}"
    );

    // The scripts' own templates: every `<input` they emit is wrapped in a
    // `<label>`, carries an aria-label, or has an id that a `<label for>`
    // in the same file refers to. Inputs built with createElement (kvInput,
    // beginEdit, the column chooser) set their names in code and are
    // covered by the axe scan, not by this check.
    let emitted = regex::Regex::new(r#"<input\b[^>]*>"#).unwrap();
    for (path, js) in js_assets() {
        for m in emitted.find_iter(&js) {
            let tag = m.as_str();
            let before = &js[m.start().saturating_sub(160)..m.start()];
            let wrapped = before
                .rfind("<label")
                .is_some_and(|i| !before[i..].contains("</label>"));
            let ok = wrapped
                || aria.is_match(tag)
                || id
                    .captures(tag)
                    .is_some_and(|c| js.contains(&format!("for=\"{}\"", &c[1])));
            assert!(ok, "{path} emits an input without a label: {tag}");
        }
    }
}

/// 2.4.7 focus visible: the stylesheet never removes the focus outline
/// without providing a :focus-visible replacement, and defines one.
#[tokio::test]
async fn css_keeps_a_visible_focus_indicator() {
    let css = asset_text("app.css");
    let removed = regex::Regex::new(r"outline\s*:\s*(none|0)\b").unwrap();
    let replacement =
        regex::Regex::new(r"(?s):focus-visible\s*\{[^}]*outline\s*:\s*\d+px\s+solid").unwrap();
    assert!(
        replacement.is_match(&css),
        "app.css must style :focus-visible with a solid outline"
    );
    // Allowed only inside a rule that is itself a :focus-visible
    // replacement; the UI currently has none, so any hit is a regression.
    if let Some(m) = removed.find(&css) {
        panic!(
            "app.css removes the focus outline (`{}`) without a replacement",
            m.as_str()
        );
    }
    // 1.4.11: control boundaries use the high-contrast token, not the
    // decorative one (the ratios are computed in tmp/agent-508/contrast.py).
    assert!(
        css.contains("--control-border:"),
        "missing --control-border token"
    );
    let btn = regex::Regex::new(r"(?s)\.btn\s*\{[^}]*border:\s*1px solid var\(--control-border\)")
        .unwrap();
    assert!(btn.is_match(&css), ".btn must use --control-border");
    // 1.4.1: state marks that do not rely on colour alone.
    assert!(
        css.contains(".visually-hidden"),
        "missing .visually-hidden utility"
    );
    assert!(
        css.contains("@media (forced-colors: active)"),
        "missing forced-colors fallback"
    );
}

/// 4.1.2 name/role/value for the custom widgets, 1.4.1 non-colour cues,
/// 2.4.2 per-view titles: the scripts contain the wiring the audit verified.
#[tokio::test]
async fn scripts_carry_the_widget_semantics_the_audit_verified() {
    let rt = asset_text("records-table.js");
    assert!(rt.contains("role=\"grid\""), "records table is a grid");
    assert!(
        rt.contains("aria-rowcount") && rt.contains("aria-rowindex"),
        "virtualised grid reports row counts"
    );
    assert!(
        rt.contains("aria-sort"),
        "sortable headers expose aria-sort"
    );
    assert!(
        rt.contains("has validation error"),
        "error rows carry text, not only colour"
    );
    assert!(
        rt.contains("(edited)"),
        "edited cells carry text, not only colour"
    );
    let tabs = asset_text("tabs.js");
    assert!(
        tabs.contains("role=\"tabpanel\"") || tabs.contains("'tabpanel'"),
        "tab panels are marked"
    );
    assert!(
        tabs.contains("aria-controls") && tabs.contains("aria-selected"),
        "tabs wire aria-controls and aria-selected"
    );
    let app = asset_text("app.js");
    assert!(
        app.contains("document.title ="),
        "the title changes per view"
    );
    assert!(
        app.contains("aria-describedby"),
        "the tooltip is linked to its anchor"
    );
    assert!(
        app.contains("aria-pressed"),
        "the theme toggle exposes its state"
    );
    let wb = asset_text("workbench.js");
    assert!(
        wb.contains("aria-invalid"),
        "form errors mark the field invalid"
    );
    assert!(
        wb.contains("class=\"visually-hidden\"") || wb.contains("novalidate"),
        "custom validation replaces the native bubble"
    );
    for (path, js) in js_assets() {
        // The file input must be focusable: hiding it with display:none
        // took the only mouse-free way to choose a file away from keyboards.
        assert!(!js.contains("type=\"file\" id=\"file-input\" accept=\".fec,text/plain,application/octet-stream\" class=\"hidden\""), "{path}: file input hidden with display:none");
    }
}
