//! ETL from the FEC's bulk-data files (and, for independent expenditures,
//! the FEC's own official `pg_dump` archive) into Postgres.
//!
//! Three complementary paths:
//! - [`source`] + [`loader`]: the pipe-delimited `.zip` files under
//!   `https://www.fec.gov/files/bulk-downloads/{cycle}/` (candidates,
//!   committees, contributions, disbursements, summaries).
//! - [`dump`]: the FEC's own real, weekly-updated `pg_dump` archives
//!   (see [`dump`] docs for which ones are practical to restore).
//! - [`ingest`]: individual raw `.fec` filings, parsed by [`crate::parser`].

pub mod dump;
pub mod error;
pub mod ingest;
pub mod loader;
pub mod preflight;
pub mod source;

pub use error::{BulkError, Result};
pub use ingest::{IngestReport, filing_id_from_path, ingest_filing, ingest_filing_bytes};
pub use loader::{Input, LoadMode, LoadOptions, LoadReport, load};
pub use source::{BulkSource, DateColumn, find as find_source};
