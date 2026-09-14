//! ETL from the FEC's bulk-data files (and, for independent expenditures,
//! the FEC's own official `pg_dump` archive) into Postgres.
//!
//! Two complementary paths:
//! - [`source`] + [`loader`]: the pipe-delimited `.zip` files under
//!   `https://www.fec.gov/files/bulk-downloads/{cycle}/` (candidates,
//!   committees, contributions, disbursements, summaries).
//! - [`dump`]: the FEC's own real, weekly-updated `pg_dump` archives
//!   (see [`dump`] docs for which ones are practical to restore).

pub mod dump;
pub mod error;
pub mod ingest;
pub mod loader;
pub mod source;

pub use error::{BulkError, Result};
pub use ingest::{IngestReport, ingest_filing, ingest_filing_bytes};
pub use loader::{Input, LoadReport, load};
pub use source::{BulkSource, find as find_source};
