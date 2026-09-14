//! `hardmoney::parser`: a Rust parser for FEC electronic filing (`.fec`) files,
//! kept current through FEC spec 8.5, and a Rust port of
//! [nyt-pyfec](https://github.com/newsdev/nyt-pyfec)'s parsing logic --
//! extended, not just translated. See the crate README for the full
//! methodology writeup (sourcing, real-data validation, and the
//! ergonomic/typed design decisions).
//!
//! # Quick start
//!
//! ```no_run
//! use hardmoney::parser::Filing;
//!
//! let bytes = std::fs::read("filing.fec").unwrap();
//! let filing = Filing::parse_bytes(&bytes).unwrap();
//!
//! println!("{} ({})", filing.raw_form_type, filing.version);
//! for line in &filing.lines {
//!     if line.table == "SchA" {
//!         println!("contribution: {:?}", line.get("contribution_amount"));
//!     }
//! }
//! ```
//!
//! # Modules
//!
//! * [`filing`] -- top-level [`Filing::parse`] entry point; header +
//!   summary-line + every body line, dispatched and parsed.
//! * [`typed`] -- ergonomic, strongly-typed views (`ScheduleA`,
//!   `ScheduleB`, `ScheduleE`, `Form3XSummary`) over the raw
//!   `IndexMap<String, String>` line output, with exact-cents money
//!   parsing and real `NaiveDate`s instead of raw strings.
//! * [`header`] -- the first physical line of a filing (software name/
//!   version, report id).
//! * [`form`] -- form-type string -> format-table dispatch.
//! * [`line`] -- version-bucketed column-position lookup for one format
//!   table.
//! * [`format_data`] -- the embedded fec-csv-sources format tables
//!   themselves.
//! * [`error`] -- crate-wide error type.

pub mod error;
pub mod filing;
pub mod format_data;
pub mod form;
pub mod header;
pub mod line;
pub mod typed;
pub mod utils;

pub use error::{FecError, Result};
pub use filing::{Filing, ParsedLine};
pub use header::HeaderMap;
pub use typed::{Form3XSummary, ScheduleA, ScheduleB, ScheduleE, TypedViewError, parse_fec_date, parse_money_cents};
