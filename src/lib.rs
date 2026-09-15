//! `hardmoney`: a single Rust crate for FEC campaign-finance data.
//!
//! Four things live here, usable independently or together, as a library
//! or via the `hardmoney` CLI binary:
//!
//! 1. [`parser`] -- a parser *and writer* for raw FEC electronic filings
//!    (`.fec` files), covering every electronic spec era (3.x quoted-CSV,
//!    5.x comma, 6.x-8.5 ASCII-28) with the column layout of every version
//!    compiled in from data, compile-time-checked field access, streaming
//!    for very large files ([`FilingReader`]), the FEC's acceptance rules
//!    ([`Filing::validate`]), and cover-page reconciliation against the
//!    schedules ([`Filing::reconcile`]). Validated against real,
//!    live-downloaded filings (see `tests/real_filings.rs`).
//! 2. [`bulk`] -- a Postgres ETL that loads the FEC's own pre-aggregated
//!    bulk-data downloads (candidates, committees, contributions,
//!    disbursements, ...) into a normalized schema, and can also ingest
//!    individual raw filings via [`parser`] for precision Schedule E
//!    extraction.
//! 3. [`db`] -- the schema those tables live in, with versioned migrations.
//! 4. [`api`] -- an Axum REST API serving the tables `bulk` builds.
//!
//! # Feature flags
//!
//! | Feature | Enables | Default |
//! |---|---|---|
//! | `fetch` | `Filing::fetch` (download a raw filing from docquery.fec.gov) | on |
//! | `serde` | `Serialize`/`Deserialize` on parser types | on (via `bulk`/`api`) |
//! | `bulk` | the [`bulk`] and [`db`] modules (Postgres ETL, needs `sqlx`) | on |
//! | `api` | the [`api`] module (Axum server) | on |
//! | `cli` | the `hardmoney` binary | on |
//!
//! A downstream crate that only wants the parser can depend on
//! `hardmoney = { version = "2", default-features = false, features = ["fetch"] }`
//! to avoid pulling in `sqlx`, `axum`, and `tokio`.
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
