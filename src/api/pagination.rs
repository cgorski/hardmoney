//! Shared query parameters: pagination and the `cycle` filter.

use serde::Deserialize;

use crate::Cycle;
use crate::api::error::ApiError;

/// `?limit=&offset=`, applied consistently across list endpoints. `limit`
/// is clamped to `[1, MAX_LIMIT]` so a client can't force an unbounded scan
/// of a multi-million-row table like `schedule_a`.
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

impl Pagination {
    pub const DEFAULT_LIMIT: i64 = 50;
    pub const MAX_LIMIT: i64 = 500;

    pub fn new(limit: Option<i64>, offset: Option<i64>) -> Self {
        Self { limit, offset }
    }

    pub fn limit(&self) -> i64 {
        self.limit
            .unwrap_or(Self::DEFAULT_LIMIT)
            .clamp(1, Self::MAX_LIMIT)
    }

    pub fn offset(&self) -> i64 {
        self.offset.unwrap_or(0).max(0)
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

    #[test]
    fn defaults_to_fifty_when_limit_absent() {
        assert_eq!(Pagination::new(None, None).limit(), 50);
    }

    #[test]
    fn clamps_limit_above_the_ceiling() {
        assert_eq!(Pagination::new(Some(1_000_000), None).limit(), 500);
    }

    #[test]
    fn clamps_limit_below_the_floor_of_one() {
        assert_eq!(Pagination::new(Some(0), None).limit(), 1);
        assert_eq!(Pagination::new(Some(-10), None).limit(), 1);
    }

    #[test]
    fn passes_through_an_in_range_limit_unchanged() {
        assert_eq!(Pagination::new(Some(200), None).limit(), 200);
    }

    #[test]
    fn defaults_offset_to_zero_and_rejects_negative_offsets() {
        assert_eq!(Pagination::new(None, None).offset(), 0);
        assert_eq!(Pagination::new(None, Some(-5)).offset(), 0);
        assert_eq!(Pagination::new(None, Some(200)).offset(), 200);
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
