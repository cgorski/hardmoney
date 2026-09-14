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
pub use ingest::{ingest_filing, ingest_filing_bytes, IngestReport};
pub use loader::{load, Input, LoadReport};
pub use source::{find as find_source, BulkSource};
