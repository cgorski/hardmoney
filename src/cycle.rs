//! [`Cycle`]: a two-year federal election cycle, identified by its even year.
//!
//! The FEC organizes bulk data by cycle (`2026` covers 2025-2026). Before
//! this type existed the crate passed cycles around as `u16` in the loader,
//! `i32` in the schema and API, and `String` in URLs -- and `--cycle 2027`
//! silently downloaded a 404. `Cycle` validates once at the boundary and
//! converts losslessly to whatever each layer needs.

use std::fmt;
use std::str::FromStr;

/// A two-year election cycle, e.g. `Cycle::new(2026)`.
///
/// Invariants: even, and within the range the FEC actually publishes
/// (1976 through a generous future bound).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(try_from = "i32", into = "i32"))]
pub struct Cycle(u16);

/// Why a value is not a valid [`Cycle`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CycleError {
    #[error("cycle {0} is not an even year (cycles are the even year of a two-year period)")]
    Odd(i64),
    #[error("cycle {0} is outside the FEC's published range ({min}-{max})", min = Cycle::MIN.0, max = Cycle::MAX.0)]
    OutOfRange(i64),
    #[error("cycle '{0}' is not a number")]
    NotANumber(String),
}

impl Cycle {
    /// The first cycle with FEC electronic bulk data.
    pub const MIN: Cycle = Cycle(1976);
    /// A generous upper bound; the FEC pre-creates directories a few cycles
    /// ahead (2028 and 2030 existed in September 2026).
    pub const MAX: Cycle = Cycle(2100);

    /// Validates `year` as a cycle.
    pub fn new(year: impl Into<i64>) -> Result<Self, CycleError> {
        let y = year.into();
        if y < i64::from(Self::MIN.0) || y > i64::from(Self::MAX.0) {
            return Err(CycleError::OutOfRange(y));
        }
        if y % 2 != 0 {
            return Err(CycleError::Odd(y));
        }
        // Range check above guarantees this fits.
        Ok(Cycle(
            u16::try_from(y).map_err(|_| CycleError::OutOfRange(y))?,
        ))
    }

    /// The cycle containing `year` (an odd year rounds up to its cycle).
    pub fn containing(year: i32) -> Result<Self, CycleError> {
        let y = i64::from(year);
        Self::new(if y % 2 == 0 { y } else { y.saturating_add(1) })
    }

    /// The even year as a `u16`.
    pub const fn year(self) -> u16 {
        self.0
    }

    /// The two-digit year the FEC uses in bulk filenames (`cn26.zip`).
    pub const fn two_digit(self) -> u8 {
        (self.0 % 100) as u8
    }

    /// The previous cycle, if within range.
    pub fn prev(self) -> Option<Self> {
        Self::new(i64::from(self.0) - 2).ok()
    }

    /// The next cycle, if within range.
    pub fn next(self) -> Option<Self> {
        Self::new(i64::from(self.0) + 2).ok()
    }
}

impl fmt::Display for Cycle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl FromStr for Cycle {
    type Err = CycleError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let y: i64 = s
            .trim()
            .parse()
            .map_err(|_| CycleError::NotANumber(s.to_string()))?;
        Cycle::new(y)
    }
}

impl TryFrom<i32> for Cycle {
    type Error = CycleError;
    fn try_from(v: i32) -> Result<Self, Self::Error> {
        Cycle::new(v)
    }
}

impl TryFrom<u16> for Cycle {
    type Error = CycleError;
    fn try_from(v: u16) -> Result<Self, Self::Error> {
        Cycle::new(v)
    }
}

impl From<Cycle> for i32 {
    fn from(c: Cycle) -> i32 {
        i32::from(c.0)
    }
}

impl From<Cycle> for u16 {
    fn from(c: Cycle) -> u16 {
        c.0
    }
}

impl From<Cycle> for i64 {
    fn from(c: Cycle) -> i64 {
        i64::from(c.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_even_years_in_range() {
        assert_eq!(Cycle::new(2026).unwrap().year(), 2026);
        assert_eq!(Cycle::new(1976).unwrap(), Cycle::MIN);
        assert_eq!("2024".parse::<Cycle>().unwrap().two_digit(), 24);
        assert_eq!(Cycle::new(2008).unwrap().two_digit(), 8);
    }

    #[test]
    fn rejects_odd_out_of_range_and_garbage() {
        assert_eq!(Cycle::new(2027), Err(CycleError::Odd(2027)));
        assert_eq!(Cycle::new(1974), Err(CycleError::OutOfRange(1974)));
        assert_eq!(Cycle::new(2102), Err(CycleError::OutOfRange(2102)));
        assert_eq!(Cycle::new(-2), Err(CycleError::OutOfRange(-2)));
        assert!(matches!(
            "abc".parse::<Cycle>(),
            Err(CycleError::NotANumber(_))
        ));
    }

    #[test]
    fn containing_rounds_odd_years_up() {
        assert_eq!(Cycle::containing(2025).unwrap().year(), 2026);
        assert_eq!(Cycle::containing(2026).unwrap().year(), 2026);
    }

    #[test]
    fn neighbours_and_conversions() {
        let c = Cycle::new(2026).unwrap();
        assert_eq!(c.prev().unwrap().year(), 2024);
        assert_eq!(c.next().unwrap().year(), 2028);
        assert_eq!(Cycle::MAX.next(), None);
        assert_eq!(Cycle::MIN.prev(), None);
        assert_eq!(i32::from(c), 2026);
        assert_eq!(u16::from(c), 2026);
        assert_eq!(c.to_string(), "2026");
    }
}
