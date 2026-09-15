//! Command-line surface. Each subcommand's arguments and implementation
//! live in their own module; this file only declares the tree.

pub mod bulk;
pub mod db_args;
pub mod dumps;
pub mod efile;
pub mod export;
pub mod filings;
pub mod parse;
pub mod query;
pub mod reconcile;
pub mod schema;
pub mod serve;
pub mod shared;
pub mod spec;
pub mod validate;
pub mod write;

use clap::{Parser, Subcommand};

pub type CliResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

#[derive(Parser)]
#[command(
    name = "hardmoney",
    version,
    about = "FEC campaign-finance data: parse .fec filings, load bulk data, serve a REST API",
    after_help = "Database commands read DATABASE_URL (or --database-url) and an optional \
                  HARDMONEY_SCHEMA (or --schema) naming an isolated namespace inside that \
                  database. Include a username in the URL: postgres://user@host/db"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
#[allow(clippy::large_enum_variant)] // clap subcommand tree; parsed once, boxing gains nothing
enum Command {
    /// Parse a single `.fec` filing and print a JSON summary.
    Parse(parse::ParseArgs),
    /// Parse a `.fec` filing and write it back out in canonical form.
    Write(write::WriteArgs),
    /// Recompute a report's cover-page totals from its schedules and show
    /// every line that disagrees.
    Reconcile(reconcile::ReconcileArgs),
    /// Check a .fec file against the FEC's acceptance rules.
    Validate(validate::ValidateArgs),
    /// Export a .fec filing's records as CSV, JSON Lines, Parquet, or SQLite.
    Export(export::ExportArgs),
    /// Create or upgrade the Postgres schema in the target namespace.
    SchemaInit(schema::SchemaInitArgs),
    /// Show migration state and recorded loads for the target namespace.
    SchemaStatus(schema::SchemaStatusArgs),
    /// List hardmoney namespaces (schemas) in the database.
    SchemaList(schema::SchemaListArgs),
    /// Drop a namespace and all data in it.
    SchemaDrop(schema::SchemaDropArgs),
    /// Load one bulk-data source into Postgres.
    BulkLoad(bulk::BulkLoadArgs),
    /// Load every bulk source for one cycle.
    BulkLoadAll(bulk::BulkLoadAllArgs),
    /// Restore one of the FEC's own official pg_dump archives.
    BulkRestoreDump(bulk::BulkRestoreDumpArgs),
    /// Show the FEC's dump files: remote size and date, cache, database state, restores.
    BulkDumpInfo(bulk::BulkDumpInfoArgs),
    /// Create hardmoney's indexes on a restored dump table.
    BulkDumpIndex(bulk::BulkDumpIndexArgs),
    /// Compare an ingested filing's raw Schedule E lines with the FEC's processed dump rows.
    BulkDumpCompare(bulk::BulkDumpCompareArgs),
    /// Ingest a single raw `.fec` filing directly.
    BulkLoadFiling(bulk::BulkLoadFilingArgs),
    /// Recompute every ingested filing's amendment chain (after a batch of `bulk-load-filing --no-resolve`).
    BulkResolveChains(bulk::BulkResolveChainsArgs),
    /// Import the FEC's Postgres dump files (guided).
    Dumps(dumps::DumpsArgs),
    /// Find filings via the FEC's API and fetch, validate, or ingest them.
    Filings(filings::FilingsArgs),
    /// Follow the FEC's electronic filing feed.
    Efile(efile::EfileArgs),
    /// Run the REST API server.
    Serve(serve::ServeArgs),
    /// Search loaded data from the terminal (same results as the REST API).
    Query(query::QueryArgs),
    /// Export or diff the machine-readable FEC format specification.
    Spec(spec::SpecArgs),
}

pub async fn run() -> CliResult {
    let cli = Cli::parse();
    match cli.command {
        Command::Parse(a) => parse::run(a),
        Command::Write(a) => write::run(a),
        Command::Reconcile(a) => reconcile::run(a),
        Command::Validate(a) => validate::run(a),
        Command::Export(a) => export::run(a),
        Command::SchemaInit(a) => schema::init(a).await,
        Command::SchemaStatus(a) => schema::status(a).await,
        Command::SchemaList(a) => schema::list(a).await,
        Command::SchemaDrop(a) => schema::drop(a).await,
        Command::BulkLoad(a) => bulk::load(a).await,
        Command::BulkLoadAll(a) => bulk::load_all(a).await,
        Command::BulkRestoreDump(a) => bulk::restore_dump(a).await,
        Command::BulkDumpInfo(a) => bulk::dump_info(a).await,
        Command::BulkDumpIndex(a) => bulk::dump_index(a).await,
        Command::BulkDumpCompare(a) => bulk::dump_compare(a).await,
        Command::BulkLoadFiling(a) => bulk::load_filing(a).await,
        Command::BulkResolveChains(a) => bulk::resolve_chains(a).await,
        Command::Dumps(a) => dumps::run(a).await,
        Command::Filings(a) => filings::run(a).await,
        Command::Efile(a) => efile::run(a).await,
        Command::Serve(a) => serve::run(a).await,
        Command::Query(a) => query::run(a).await,
        Command::Spec(a) => spec::run(a),
    }
}
