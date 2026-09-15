//! The browser UI behind `hardmoney serve --ui`: a filing workbench (drop a
//! `.fec`, see its cover page, validation findings, and reconciliation,
//! edit fields, download the result) and a browser for the JSON API.
//!
//! The UI is a static shell (`index.html`) plus hand-written CSS and ES
//! modules under `src/ui/assets/`, embedded in the binary with `rust-embed`
//! so a deployment is one file and nothing is loaded from a CDN. All
//! server-side work happens in [`crate::api::routes::tools`]; this module
//! only serves bytes.
//!
//! | Route | Response |
//! |---|---|
//! | `GET /ui`, `GET /ui/` | the shell |
//! | `GET /ui/assets/{path}` | an embedded asset, or 404 |
//! | `GET /ui/{anything else}` | the shell, so the client-side router owns the path |
//!
//! Assets carry an `ETag` (the SHA-256 of their content) and
//! `Cache-Control: public, no-cache`, so a browser keeps them but
//! revalidates on every load and sees a new build immediately; a matching
//! `If-None-Match` gets a 304. The shell is never cached. These routes sit
//! outside the API-key middleware (the page has to load before the user can
//! enter a key); everything the page then calls is behind it.

use std::borrow::Cow;

use axum::Router;
use axum::body::Body;
use axum::extract::Path;
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "src/ui/assets/"]
struct Assets;

/// The shell's path within the embedded folder.
const INDEX: &str = "index.html";

/// Builds the `/ui` routes. State-free: mount it into any router.
pub fn router<S: Clone + Send + Sync + 'static>() -> Router<S> {
    Router::new()
        .route("/ui", get(index))
        .route("/ui/", get(index))
        .route("/ui/assets/{*path}", get(asset))
        .route("/ui/{*path}", get(index))
}

/// Every embedded asset path, relative to the assets folder
/// (`"app.js"`, `"index.html"`). For tests and diagnostics.
pub fn asset_paths() -> impl Iterator<Item = Cow<'static, str>> {
    Assets::iter()
}

/// The bytes of one embedded asset, if it exists.
#[must_use]
pub fn asset_bytes(path: &str) -> Option<Cow<'static, [u8]>> {
    Assets::get(path).map(|f| f.data)
}

/// The `Content-Type` for an asset, by extension. Everything the UI ships
/// is listed; anything else is served as bytes.
#[must_use]
pub fn content_type(path: &str) -> &'static str {
    let ext = path.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    match ext {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        // Both nibbles are < 16, so `from_digit` always succeeds.
        out.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
        out.push(char::from_digit(u32::from(b & 0x0f), 16).unwrap_or('0'));
    }
    out
}

fn body_from(data: Cow<'static, [u8]>) -> Body {
    match data {
        Cow::Borrowed(b) => Body::from(b),
        Cow::Owned(v) => Body::from(v),
    }
}

/// Security headers for every UI response. The policy allows only
/// same-origin scripts, styles, and fetches, which is all the UI uses.
fn hardening_headers(headers: &mut HeaderMap) {
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(
            "default-src 'self'; script-src 'self'; style-src 'self'; img-src 'self' data:; \
             connect-src 'self'; font-src 'self'; frame-ancestors 'none'; base-uri 'none'",
        ),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
}

async fn index() -> Response {
    let Some(file) = Assets::get(INDEX) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "the UI shell is missing from this build",
        )
            .into_response();
    };
    let mut response = (StatusCode::OK, body_from(file.data)).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    hardening_headers(headers);
    response
}

async fn asset(Path(path): Path<String>, request_headers: HeaderMap) -> Response {
    serve_asset(&path, &request_headers)
}

fn serve_asset(path: &str, request_headers: &HeaderMap) -> Response {
    let Some(file) = Assets::get(path) else {
        return (StatusCode::NOT_FOUND, "no such asset").into_response();
    };
    let etag = format!("\"{}\"", hex(&file.metadata.sha256_hash()));
    let Ok(etag_value) = HeaderValue::from_str(&etag) else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "bad asset hash").into_response();
    };

    let matches = request_headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.split(',').any(|t| t.trim() == etag || t.trim() == "*"));
    let mut response = if matches {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        (StatusCode::OK, body_from(file.data)).into_response()
    };
    let headers = response.headers_mut();
    headers.insert(header::ETAG, etag_value);
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, no-cache"),
    );
    if !matches {
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(content_type(path)),
        );
    }
    hardening_headers(headers);
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeSet;

    use regex::Regex;

    fn text(path: &str) -> String {
        let bytes = asset_bytes(path).unwrap_or_else(|| panic!("{path} is not embedded"));
        String::from_utf8(bytes.into_owned()).unwrap()
    }

    #[test]
    fn the_shell_and_the_entry_script_are_embedded() {
        let paths: BTreeSet<String> = asset_paths().map(Cow::into_owned).collect();
        assert!(paths.contains("index.html"), "{paths:?}");
        assert!(paths.contains("app.js"), "{paths:?}");
        assert!(paths.contains("app.css"), "{paths:?}");
    }

    #[test]
    fn every_asset_is_served_with_a_type_and_an_etag() {
        for path in asset_paths() {
            let resp = serve_asset(&path, &HeaderMap::new());
            assert_eq!(resp.status(), StatusCode::OK, "{path}");
            let ct = resp.headers().get(header::CONTENT_TYPE).unwrap();
            assert_ne!(ct, "application/octet-stream", "{path} has no known type");
            let etag = resp.headers().get(header::ETAG).unwrap().clone();
            assert!(etag.to_str().unwrap().starts_with('"'));

            let mut h = HeaderMap::new();
            h.insert(header::IF_NONE_MATCH, etag);
            let again = serve_asset(&path, &h);
            assert_eq!(again.status(), StatusCode::NOT_MODIFIED, "{path}");
        }
        assert_eq!(
            serve_asset("missing.js", &HeaderMap::new()).status(),
            StatusCode::NOT_FOUND
        );
    }

    /// The shell may only reference embedded assets, and every ES-module
    /// import inside the JS must resolve to an embedded file. A typo here
    /// would otherwise only show up in a browser console.
    #[test]
    fn html_and_js_reference_only_embedded_assets() {
        let paths: BTreeSet<String> = asset_paths().map(Cow::into_owned).collect();
        let html = text("index.html");
        let referenced = Regex::new(r#"/ui/assets/([A-Za-z0-9_./-]+)"#).unwrap();
        let mut seen = 0;
        for cap in referenced.captures_iter(&html) {
            let p = &cap[1];
            assert!(paths.contains(p), "index.html references missing asset {p}");
            seen += 1;
        }
        assert!(seen >= 2, "index.html should reference its CSS and JS");

        let import = Regex::new(r#"(?:from|import)\s+["']\./([A-Za-z0-9_./-]+)["']"#).unwrap();
        for path in paths.iter().filter(|p| p.ends_with(".js")) {
            let js = text(path);
            for cap in import.captures_iter(&js) {
                let p = &cap[1];
                assert!(paths.contains(p), "{path} imports missing module {p}");
            }
        }
    }

    #[test]
    fn no_inline_styles_or_scripts_that_the_csp_would_block() {
        let html = text("index.html");
        assert!(
            !html.contains(" style=\""),
            "inline style attribute in index.html"
        );
        assert!(
            !html.contains("<style"),
            "inline <style> block in index.html"
        );
        assert!(
            !Regex::new(r"<script(?:\s[^>]*)?>\s*[^<\s]")
                .unwrap()
                .is_match(&html),
            "inline <script> body in index.html"
        );
    }

    #[test]
    fn content_types_by_extension() {
        assert_eq!(content_type("app.js"), "text/javascript; charset=utf-8");
        assert_eq!(content_type("a/b.css"), "text/css; charset=utf-8");
        assert_eq!(content_type("index.html"), "text/html; charset=utf-8");
        assert_eq!(content_type("icon.svg"), "image/svg+xml");
        assert_eq!(content_type("noext"), "application/octet-stream");
    }

    #[test]
    fn hex_encodes_lowercase() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
    }
}
