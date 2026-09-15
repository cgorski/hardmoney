//! `hardmoney::parser`: a Rust parser (and writer) for FEC electronic
//! filing (`.fec`) files, covering every electronic spec version from 3.x
//! (2001) through 8.5.
//!
//! # Quick start
//!
//! ```no_run
//! use hardmoney::parser::{Filing, ScheduleA, Table};
//! use hardmoney::parser::tables::{f3x, markers::F3X};
//!
//! let bytes = std::fs::read("filing.fec").unwrap();
//! let filing = Filing::parse_bytes(&bytes).unwrap();
//!
//! println!("{} ({})", filing.raw_form_type, filing.version);
//! // Untyped: any field by name.
//! for line in filing.lines_for(Table::SchA) {
//!     println!("contribution: {:?}", line.get("contribution_amount"));
//! }
//! // Compile-time-checked: only F3X fields accepted for an F3X cover page.
//! if let Ok(cover) = filing.summary_as::<F3X>() {
//!     println!("total receipts: {:?}", cover.money(f3x::COL_A_TOTAL_RECEIPTS));
//! }
//! // Domain views with parsed money, dates, and code enums.
//! for a in filing.views::<ScheduleA>() {
//!     println!("{:?} gave {:?}", a.contributor_name, a.contribution_amount);
//! }
//! ```
//!
//! # Layers
//!
//! * [`schema`] -- the static description of the format: [`SpecVersion`],
//!   per-version [`Layout`]s, the FEC's [`FieldSpec`]s, and the
//!   [`Field`]/[`Typed`] machinery for compile-time-checked access.
//! * [`tables`] -- generated at build time: the [`Table`] enum and one
//!   module per table with its marker type, field constants, layouts, and
//!   specs.
//! * [`filing`] -- [`Filing::parse`]: header + cover line + every body
//!   line, dispatched and parsed into [`ParsedLine`]s; strict or lenient.
//! * [`typed`] -- domain views ([`ScheduleA`], [`ScheduleB`],
//!   [`ScheduleE`], [`Form3XSummary`]) with exact-cents money, real dates,
//!   and code enums.
//! * [`header`] -- the typed [`Header`] record.
//! * [`form`] -- form-type token -> [`Table`] dispatch.
//! * [`error`] -- [`FecError`].
//!
//! # Strict vs. lenient
//!
//! [`Filing::parse`] is strict: one unparseable body line fails the whole
//! filing. [`Filing::parse_with`] + [`ParseOptions::LENIENT`] skips such
//! lines and returns a [`Lenient<Filing>`] that must be opened with
//! [`Lenient::into_parts`] (value + skipped lines) or
//! [`Lenient::into_strict`] (error if anything was skipped).

pub mod error;
pub mod filing;
pub mod form;
pub mod header;
pub mod reconcile;
pub mod schema;
pub mod stream;
pub mod tables;
pub mod typed;
pub mod utils;
pub mod validate;
#[cfg(feature = "fetch")]
pub mod webcheck;
pub mod writer;

pub use error::{FecError, Result};
pub use filing::{
    Filing, Lenient, OnUnparseableLine, ParseOptions, ParsedLine, SkipReason, SkippedLine,
};
pub use header::Header;
pub use reconcile::{
    Column, LineCheck, LineRule, ReconcileError, Reconciliation, Relation, ScheduleSum, Source,
    Term,
};
pub use schema::{
    Field, FieldDef, FieldKind, FieldSpec, InvalidSpecVersion, Layout, Requirement, SpecVersion,
    TableMarker, Typed,
};
pub use stream::{FilingReader, Preamble};
pub use tables::{BUNDLED_SPEC_VERSION, Table};
pub use typed::{
    EntityType, Form3XSummary, ScheduleA, ScheduleB, ScheduleE, SupportOppose, TypedView,
    TypedViewError, parse_fec_date, parse_money,
};
pub use validate::{Finding, Rule, Severity, Validation};
