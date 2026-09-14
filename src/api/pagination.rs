use serde::Deserialize;

/// Shared `?limit=&offset=` query params, applied consistently across list
/// endpoints. `limit` is clamped to `[1, 500]` so a client can't
/// accidentally (or deliberately) force an unbounded scan of a
/// multi-million-row table like `schedule_a`.
///
/// Deliberately *not* meant to be used behind `#[serde(flatten)]`: axum's
/// `Query` extractor (via `serde_html_form`) loses per-field type
/// information once a struct is flattened into another, so a query string
/// like `?limit=2` fails to deserialize into `i64` at all ("invalid type:
/// string \"2\", expected i64"). Every route's params struct instead
/// declares `limit`/`offset` directly and builds a `Pagination` from them
/// with [`Pagination::new`].
#[derive(Deserialize)]
pub struct Pagination {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

impl Pagination {
    pub fn new(limit: Option<i64>, offset: Option<i64>) -> Self {
        Self { limit, offset }
    }

    pub fn limit(&self) -> i64 {
        self.limit.unwrap_or(50).clamp(1, 500)
    }

    pub fn offset(&self) -> i64 {
        self.offset.unwrap_or(0).max(0)
    }
}
