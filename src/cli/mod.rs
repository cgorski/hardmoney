//! Command-line surface. Each subcommand's arguments and implementation
//! live in their own module; this file only declares the tree.

pub mod bulk;
pub mod db_args;
pub mod parse;
pub mod schema;
pub mod serve;

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
enum Command {
    /// Parse a single `.fec` filing and print a JSON summary.
    Parse(parse::ParseArgs),
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
    /// Ingest a single raw `.fec` filing directly.
    BulkLoadFiling(bulk::BulkLoadFilingArgs),
    /// Run the REST API server.
    Serve(serve::ServeArgs),
}

pub async fn run() -> CliResult {
    let cli = Cli::parse();
    match cli.command {
        Command::Parse(a) => parse::run(a),
        Command::SchemaInit(a) => schema::init(a).await,
        Command::SchemaStatus(a) => schema::status(a).await,
        Command::SchemaList(a) => schema::list(a).await,
        Command::SchemaDrop(a) => schema::drop(a).await,
        Command::BulkLoad(a) => bulk::load(a).await,
        Command::BulkLoadAll(a) => bulk::load_all(a).await,
        Command::BulkRestoreDump(a) => bulk::restore_dump(a).await,
        Command::BulkLoadFiling(a) => bulk::load_filing(a).await,
        Command::Serve(a) => serve::run(a).await,
    }
}
