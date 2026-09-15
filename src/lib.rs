//! `hardmoney`: a single Rust crate for FEC campaign-finance data.
//!
//! Four things live here, usable independently or together, as a library
//! or via the `hardmoney` CLI binary:
//!
//! 1. [`parser`] -- a parser for raw FEC electronic filings (`.fec` files),
//!    covering every electronic spec era (3.x quoted-CSV, 5.x comma, 6.x-8.5
//!    ASCII-28) with correct version-bucketed field positions per form, plus
//!    ergonomic typed views. Validated against real, live-downloaded
//!    filings (see `tests/real_filings.rs`).
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
//! `hardmoney = { version = "1", default-features = false, features = ["fetch"] }`
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

#[cfg(feature = "bulk")]
pub mod cycle;

#[cfg(feature = "bulk")]
pub use cycle::Cycle;

// Ergonomic re-exports at the crate root so `use hardmoney::Filing` reads
// naturally.
pub use parser::{
    EntityType, FecError, Field, Filing, Form3XSummary, Header, Lenient, ParseOptions, ParsedLine,
    Result, ScheduleA, ScheduleB, ScheduleE, SkippedLine, SpecVersion, SupportOppose, Table, Typed,
    TypedView, TypedViewError, parse_fec_date, parse_money,
};
