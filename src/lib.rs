//! `hardmoney`: FEC campaign-finance data as a Rust library, a CLI, and a
//! Python package.
//!
//! The crate has five parts. Each can be used on its own.
//!
//! 1. [`parser`]: reads and writes FEC electronic filings (`.fec` files) in
//!    every spec version since 2001 (3.x quoted CSV, 5.x comma, 6.x-8.5
//!    ASCII-28). The column layout of every version is compiled in from
//!    data; field access is checked against the table at compile time;
//!    [`FilingReader`] streams files of any size; [`Filing::validate`]
//!    applies the FEC's acceptance rules; [`Filing::reconcile`] checks a
//!    cover page against its schedules. Tested against real filings
//!    (`tests/real_filings.rs`).
//! 2. [`fec`]: the FEC's openFEC API, the e-file RSS feed, the daily
//!    e-filing archives, and a download cache.
//! 3. [`export`]: a filing's records as CSV, JSON Lines, Parquet, or SQLite.
//! 4. [`bulk`] and [`db`]: a Postgres ETL for the FEC's bulk downloads and
//!    for individual filings, with versioned migrations and isolated
//!    namespaces.
//! 5. [`api`]: an Axum REST API over the loaded tables.
//!
//! The Python package in `python/` wraps the parser, writer, validator,
//! and reconciler.
//!
//! # Feature flags
//!
//! | Feature | Enables | Default |
//! |---|---|---|
//! | `fetch` | [`Filing::fetch`], the [`fec`] module, `validate --oracle` | on |
//! | `serde` | `Serialize` on parser types; the openFEC client | on (via `bulk`/`api`) |
//! | `export` | the [`export`] module (Parquet, SQLite) | on |
//! | `bulk` | the [`bulk`] and [`db`] modules (Postgres ETL, `sqlx`) | on |
//! | `api` | the [`api`] module (Axum server) | on |
//! | `cli` | the `hardmoney` binary | on |
//!
//! A crate that only needs the parser can depend on
//! `hardmoney = { version = "2", default-features = false, features = ["fetch"] }`,
//! which leaves out `sqlx`, `axum`, `tokio`, `arrow`, and `rusqlite`.
//!
//! # Quick start (library)
//!
//! ```no_run
//! use hardmoney::{Filing, ScheduleA};
//!
//! let bytes = std::fs::read("filing.fec").unwrap();
//! let filing = Filing::parse_bytes(&bytes).unwrap();
//! println!("{} ({})", filing.raw_form_type, filing.version);
//! for a in filing.views::<ScheduleA>() {
//!     println!("{:?}: {:?}", a.contributor_name, a.contribution_amount);
//! }
//! ```

pub mod parser;

#[cfg(feature = "bulk")]
pub mod bulk;

#[cfg(feature = "bulk")]
pub mod db;

#[cfg(feature = "api")]
pub mod api;

#[cfg(feature = "api")]
pub mod ui;

#[cfg(feature = "export")]
pub mod export;

#[cfg(feature = "fetch")]
pub mod fec;

#[cfg(feature = "bulk")]
pub mod cycle;

#[cfg(feature = "bulk")]
pub use cycle::Cycle;

// Ergonomic re-exports at the crate root so `use hardmoney::Filing` reads
// naturally.
pub use parser::{
    EntityType, FecError, Field, Filing, FilingReader, Form3XSummary, Header, Lenient,
    ParseOptions, ParsedLine, Reconciliation, Result, ScheduleA, ScheduleB, ScheduleE, SkippedLine,
    SpecVersion, SupportOppose, Table, Typed, TypedView, TypedViewError, Validation,
    parse_fec_date, parse_money,
};
