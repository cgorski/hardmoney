//! Shared query parameters: pagination and the `cycle` filter.

use serde::Deserialize;

use crate::Cycle;
use crate::api::error::ApiError;

/// `?limit=&offset=`, applied consistently across list endpoints.
///
/// A `Pagination` is the *raw* client input. Call [`Pagination::validate`]
/// to turn it into a [`Validated`] page or a 400: a `limit` outside
/// `1..=MAX_LIMIT` or a negative `offset` is rejected with an explanatory
/// message rather than silently clamped, so a client that asks for
/// `?limit=9999` finds out it is not getting 9999 rows. (Silent clamping
/// is the pattern the FEC's own API team flagged as developer-unfriendly.)
/// The ceiling itself exists so a client cannot force an unbounded scan of
/// a multi-million-row table like `schedule_a`.
///
/// Deliberately *not* used behind `#[serde(flatten)]`: axum's `Query`
/// extractor loses per-field type information once a struct is flattened,
/// so `?limit=2` fails to deserialize into `i64`. Every route's params
/// struct declares `limit`/`offset` directly and builds a `Pagination` via
/// [`Pagination::new`].
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Pagination {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// A page whose bounds have been checked by [`Pagination::validate`]:
/// `limit` is within `1..=Pagination::MAX_LIMIT` and `offset >= 0`. The
/// fields are private so a `Validated` cannot be built any other way; bind
/// [`Validated::limit`] and [`Validated::offset`] straight into `LIMIT $n
/// OFFSET $m`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Validated {
    limit: i64,
    offset: i64,
}

impl Validated {
    /// The row cap, in `1..=Pagination::MAX_LIMIT`.
    #[must_use]
    pub const fn limit(self) -> i64 {
        self.limit
    }

    /// Rows to skip, `>= 0`.
    #[must_use]
    pub const fn offset(self) -> i64 {
        self.offset
    }
}

impl Pagination {
    pub const DEFAULT_LIMIT: i64 = 50;
    pub const MAX_LIMIT: i64 = 500;

    #[must_use]
    pub fn new(limit: Option<i64>, offset: Option<i64>) -> Self {
        Self { limit, offset }
    }

    /// Checks the client's bounds and returns the page to bind, or the
    /// 400 to send back.
    ///
    /// An absent `limit` defaults to [`Pagination::DEFAULT_LIMIT`] and an
    /// absent `offset` to 0. A present `limit` outside `1..=MAX_LIMIT` fails
    /// with `limit must be between 1 and 500`; a negative `offset` fails
    /// with `offset must be >= 0`. Both are [`ApiError::BadRequest`].
    pub fn validate(&self) -> Result<Validated, ApiError> {
        let limit = self.limit.unwrap_or(Self::DEFAULT_LIMIT);
        if !(1..=Self::MAX_LIMIT).contains(&limit) {
            return Err(ApiError::BadRequest(format!(
                "limit must be between 1 and {}",
                Self::MAX_LIMIT
            )));
        }
        let offset = self.offset.unwrap_or(0);
        if offset < 0 {
            return Err(ApiError::BadRequest("offset must be >= 0".to_string()));
        }
        Ok(Validated { limit, offset })
    }
}

/// Validates an optional `?cycle=` query value into `Option<i32>` for
/// binding, rejecting odd/out-of-range years with a 400 instead of
/// silently returning an empty result set.
pub fn cycle_param(raw: Option<i32>) -> Result<Option<i32>, ApiError> {
    match raw {
        None => Ok(None),
        Some(y) => Cycle::new(y)
            .map(|c| Some(i32::from(c)))
            .map_err(|e| ApiError::BadRequest(e.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bad_request(r: Result<Validated, ApiError>) -> String {
        match r {
            Err(ApiError::BadRequest(msg)) => msg,
            other => panic!("expected BadRequest, got {other:?}"),
        }
    }

    #[test]
    fn validate_defaults_to_fifty_and_zero_when_absent() {
        let page = Pagination::new(None, None).validate().unwrap();
        assert_eq!(page.limit(), 50);
        assert_eq!(page.offset(), 0);
    }

    #[test]
    fn validate_passes_in_range_values_through_unchanged() {
        let page = Pagination::new(Some(200), Some(1_000)).validate().unwrap();
        assert_eq!((page.limit(), page.offset()), (200, 1_000));
        // Both ends of the range are inclusive.
        assert_eq!(
            Pagination::new(Some(1), None).validate().unwrap().limit(),
            1
        );
        assert_eq!(
            Pagination::new(Some(500), None).validate().unwrap().limit(),
            500
        );
        assert_eq!(
            Pagination::new(None, Some(0)).validate().unwrap().offset(),
            0
        );
    }

    #[test]
    fn validate_rejects_a_limit_above_the_ceiling_with_400() {
        assert_eq!(
            bad_request(Pagination::new(Some(501), None).validate()),
            "limit must be between 1 and 500"
        );
        assert_eq!(
            bad_request(Pagination::new(Some(1_000_000), None).validate()),
            "limit must be between 1 and 500"
        );
    }

    #[test]
    fn validate_rejects_a_limit_below_one_with_400() {
        assert_eq!(
            bad_request(Pagination::new(Some(0), None).validate()),
            "limit must be between 1 and 500"
        );
        assert_eq!(
            bad_request(Pagination::new(Some(-10), None).validate()),
            "limit must be between 1 and 500"
        );
    }

    #[test]
    fn validate_rejects_a_negative_offset_with_400() {
        assert_eq!(
            bad_request(Pagination::new(None, Some(-5)).validate()),
            "offset must be >= 0"
        );
        // A bad limit is reported before a bad offset.
        assert_eq!(
            bad_request(Pagination::new(Some(0), Some(-5)).validate()),
            "limit must be between 1 and 500"
        );
    }

    #[test]
    fn cycle_param_validates() {
        assert_eq!(cycle_param(None).unwrap(), None);
        assert_eq!(cycle_param(Some(2026)).unwrap(), Some(2026));
        assert!(matches!(
            cycle_param(Some(2027)),
            Err(ApiError::BadRequest(_))
        ));
        assert!(matches!(cycle_param(Some(3)), Err(ApiError::BadRequest(_))));
    }
}
