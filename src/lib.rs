//! `hardmoney`: a single Rust crate for FEC campaign-finance data.
//!
//! Three things live here, usable independently or together, as a library
//! or via the `hardmoney` CLI binary:
//!
//! 1. [`parser`] -- a parser for raw FEC electronic filings (`.fec` files),
//!    a Rust port of [nyt-pyfec](https://github.com/newsdev/nyt-pyfec) kept
//!    current through FEC electronic filing spec 8.5, extended with
//!    ergonomic typed views and validated against real, live-downloaded
//!    filings (see `tests/real_filings.rs`).
//! 2. [`bulk`] -- a Postgres ETL that loads the FEC's own pre-aggregated
//!    bulk-data CSV/pipe-delimited downloads (candidates, committees,
//!    contributions, disbursements, ...) into a normalized schema, and can
//!    also ingest individual raw filings via [`parser`] for
//!    precision Schedule E extraction.
//! 3. [`api`] -- an Axum REST API serving the tables `bulk` builds.
//!
//! # Feature flags
//!
//! | Feature | Enables | Default |
//! |---|---|---|
//! | `fetch` | `Filing::fetch` (download a raw filing from docquery.fec.gov) | on |
//! | `serde` | `Serialize`/`Deserialize` on parser types | on (via `bulk`/`api`) |
//! | `bulk` | the [`bulk`] module (Postgres ETL, needs `sqlx`) | on |
//! | `api` | the [`api`] module (Axum server) | on |
//! | `cli` | the `hardmoney` binary | on |
//!
//! A downstream crate that only wants the parser can depend on
//! `hardmoney = { version = "0.1", default-features = false, features = ["fetch"] }`
//! to avoid pulling in `sqlx`, `axum`, and `tokio`.
//!
//! # Quick start (library)
//!
//! ```no_run
//! use hardmoney::parser::Filing;
//!
//! let bytes = std::fs::read("filing.fec").unwrap();
//! let filing = Filing::parse_bytes(&bytes).unwrap();
//! println!("{} ({})", filing.raw_form_type, filing.version);
//! ```

pub mod parser;

#[cfg(feature = "bulk")]
pub mod bulk;

#[cfg(feature = "bulk")]
pub mod db;

#[cfg(feature = "api")]
pub mod api;

// Ergonomic re-exports at the crate root, mirroring the old `fec_parser`
// crate's top-level API so `use hardmoney::Filing` reads naturally.
pub use parser::{FecError, Filing, HeaderMap, ParsedLine, Result};
pub use parser::{
    Form3XSummary, ScheduleA, ScheduleB, ScheduleE, TypedViewError, parse_fec_date,
    parse_money_cents,
};
