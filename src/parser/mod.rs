//! `hardmoney::parser`: a Rust parser for FEC electronic filing (`.fec`) files,
//! kept current through FEC spec 8.5. Originally a port of
//! [nyt-pyfec](https://github.com/newsdev/nyt-pyfec)'s parsing logic --
//! extended, not just translated. See the crate README for the full
//! methodology writeup (sourcing, real-data validation, and the
//! ergonomic/typed design decisions).
//!
//! # Quick start
//!
//! ```no_run
//! use hardmoney::parser::{Filing, ScheduleA, Table};
//!
//! let bytes = std::fs::read("filing.fec").unwrap();
//! let filing = Filing::parse_bytes(&bytes).unwrap();
//!
//! println!("{} ({})", filing.raw_form_type, filing.version);
//! for line in filing.lines_for(Table::SchA) {
//!     println!("contribution: {:?}", line.get("contribution_amount"));
//! }
//! // Or, typed:
//! for a in filing.views::<ScheduleA>() {
//!     println!("{:?} gave {:?}", a.contributor_name, a.contribution_amount);
//! }
//! ```
//!
//! # Strict vs. lenient
//!
//! [`Filing::parse`] is strict: one unparseable body line fails the whole
//! filing. [`Filing::parse_with`] + [`ParseOptions::LENIENT`] skips such
//! lines and returns a [`Lenient<Filing>`] that must be opened with
//! [`Lenient::into_parts`] (value + skipped lines) or
//! [`Lenient::into_strict`] (error if anything was skipped).
//!
//! # Modules
//!
//! * [`filing`] -- top-level [`Filing::parse`] entry point; header +
//!   summary-line + every body line, dispatched and parsed.
//! * [`typed`] -- ergonomic, strongly-typed views ([`ScheduleA`],
//!   [`ScheduleB`], [`ScheduleE`], [`Form3XSummary`]) over the raw
//!   `IndexMap<String, String>` line output, with exact-cents money
//!   parsing and real `NaiveDate`s instead of raw strings.
//! * [`header`] -- the first physical line of a filing (software name/
//!   version, report id).
//! * [`form`] -- form-type token -> [`Table`] dispatch.
//! * [`mod@line`] -- version-bucketed column-position lookup for one format
//!   table.
//! * [`format_data`] -- the [`Table`] enum and embedded fec-csv-sources
//!   format tables.
//! * [`error`] -- [`FecError`].

pub mod error;
pub mod filing;
pub mod form;
pub mod format_data;
pub mod header;
pub mod line;
pub mod typed;
pub mod utils;

pub use error::{FecError, Result};
pub use filing::{
    Filing, Lenient, OnUnparseableLine, ParseOptions, ParsedLine, SkipReason, SkippedLine,
};
pub use format_data::{Table, UnknownTable};
pub use header::HeaderMap;
pub use typed::{
    EntityType, Form3XSummary, ScheduleA, ScheduleB, ScheduleE, SupportOppose, TypedView,
    TypedViewError, parse_fec_date, parse_money,
};
