//! `hardmoney` CLI: schema management, bulk-data ETL, filing ingestion,
//! and the REST API server, all backed by the `hardmoney` library.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "hardmoney", version, about = "FEC campaign-finance data: parse .fec filings, load bulk data, serve a REST API")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Parse a single `.fec` filing and print its header/summary as JSON.
    Parse {
        /// Path to a `.fec` file.
        path: PathBuf,
    },
    /// Apply the Postgres schema (idempotent).
    SchemaInit {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
    },
    /// Load one bulk-data source into Postgres.
    BulkLoad {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        /// One of: candidates, committees, candidate_committee_links,
        /// schedule_a, committee_to_committee_transactions,
        /// committee_to_candidate_transactions, disbursements,
        /// candidate_summary, house_senate_summary, pac_party_summary.
        source: String,
        /// 4-digit even-year election cycle, e.g. 2026.
        #[arg(long)]
        cycle: u16,
        /// Load from a local file instead of downloading from fec.gov.
        #[arg(long)]
        file: Option<PathBuf>,
        /// Cap the number of rows read (samples without a full download
        /// for multi-gigabyte sources like `schedule_a`).
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Load every bulk source for one cycle.
    BulkLoadAll {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        #[arg(long)]
        cycle: u16,
        #[arg(long)]
        limit: Option<u64>,
    },
    /// Restore one of the FEC's own official pg_dump archives (schedule_e
    /// and committee_history are small and practical; schedule_a_full and
    /// schedule_b_full are tens of gigabytes and require --allow-large).
    BulkRestoreDump {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        /// One of: schedule_e, committee_history, schedule_a_full, schedule_b_full.
        name: String,
        #[arg(long)]
        allow_large: bool,
        #[arg(long, default_value = "/tmp/hardmoney-dumps")]
        cache_dir: PathBuf,
    },
    /// Ingest a single raw `.fec` filing directly (precise Schedule E
    /// extraction; complements the bulk-loaded, aggregate tables).
    BulkLoadFiling {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        /// A local `.fec` file path, or a numeric filing id to download
        /// from the FEC's own document store (requires the `fetch`
        /// feature, on by default).
        filing: String,
    },
    /// Run the REST API server.
    Serve {
        #[arg(long, env = "DATABASE_URL")]
        database_url: String,
        #[arg(long, default_value = "0.0.0.0:8080")]
        bind: SocketAddr,
    },
}

#[tokio::main]
async fn main() {
    // Best-effort: load a local `.env` file (e.g. DATABASE_URL) if present.
    // Explicit environment variables and `--database-url` still win, since
    // dotenvy never overrides a variable that's already set.
    let _ = dotenvy::dotenv();
    tracing_subscriber::fmt::init();
    let cli = Cli::parse();
    if let Err(e) = run(cli).await {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    match cli.command {
        Command::Parse { path } => {
            let bytes = std::fs::read(&path)?;
            let filing = hardmoney::Filing::parse_bytes(&bytes)?;
            let header: serde_json::Map<String, serde_json::Value> =
                filing.headers.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
            let summary: serde_json::Map<String, serde_json::Value> =
                filing.summary.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect();
            let out = serde_json::json!({
                "form_type": filing.raw_form_type,
                "base_form_type": filing.base_form_type,
                "version": filing.version,
                "is_amendment": filing.is_amendment,
                "line_count": filing.lines.len(),
                "header": header,
                "summary": summary,
            });
            println!("{}", serde_json::to_string_pretty(&out)?);
        }

        Command::SchemaInit { database_url } => {
            let pool = connect(&database_url).await?;
            hardmoney::db::ensure_schema(&pool).await?;
            println!("schema applied");
        }

        Command::BulkLoad { database_url, source, cycle, file, limit } => {
            let pool = connect(&database_url).await?;
            hardmoney::db::ensure_schema(&pool).await?;
            let src = hardmoney::bulk::find_source(&source).ok_or_else(|| format!("unknown source '{source}'"))?;
            let input = match file {
                Some(path) => hardmoney::bulk::Input::LocalFile(path),
                None => hardmoney::bulk::Input::Url(hardmoney::bulk::source::download_url(src, cycle)),
            };
            let report = hardmoney::bulk::load(&pool, src, input, cycle, limit).await?;
            println!("loaded {} rows into {}", report.rows_loaded, report.table);
        }

        Command::BulkLoadAll { database_url, cycle, limit } => {
            let pool = connect(&database_url).await?;
            hardmoney::db::ensure_schema(&pool).await?;
            for src in hardmoney::bulk::source::ALL {
                let url = hardmoney::bulk::source::download_url(src, cycle);
                println!("loading {} from {url} ...", src.name);
                match hardmoney::bulk::load(&pool, src, hardmoney::bulk::Input::Url(url), cycle, limit).await {
                    Ok(report) => println!("  {} rows into {}", report.rows_loaded, report.table),
                    Err(e) => eprintln!("  failed: {e}"),
                }
            }
        }

        Command::BulkRestoreDump { database_url, name, allow_large, cache_dir } => {
            let src = hardmoney::bulk::dump::find(&name).ok_or_else(|| format!("unknown dump '{name}'"))?;
            let path = hardmoney::bulk::dump::restore(&database_url, src, &cache_dir, allow_large).await?;
            println!("restored {} into disclosure.{} (cached at {})", src.name, src.disclosure_table, path.display());
            println!("re-run `schema-init` to (re)create the friendly `independent_expenditures` view over it");
        }

        Command::BulkLoadFiling { database_url, filing } => {
            let pool = connect(&database_url).await?;
            hardmoney::db::ensure_schema(&pool).await?;

            let report = if let Ok(id) = filing.parse::<u64>() {
                #[cfg(feature = "fetch")]
                {
                    let parsed = hardmoney::Filing::fetch(id)?;
                    hardmoney::bulk::ingest_filing(&pool, id as i64, &parsed).await?
                }
                #[cfg(not(feature = "fetch"))]
                {
                    return Err("numeric filing ids require the `fetch` feature".into());
                }
            } else {
                let bytes = std::fs::read(&filing)?;
                let filing_id: i64 = std::path::Path::new(&filing)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .and_then(|s| s.chars().filter(|c| c.is_ascii_digit()).collect::<String>().parse().ok())
                    .unwrap_or(0);
                hardmoney::bulk::ingest_filing_bytes(&pool, filing_id, &bytes).await?
            };

            println!("ingested {} ({} Schedule E lines)", report.form_type, report.schedule_e_lines);
        }

        Command::Serve { database_url, bind } => {
            let pool = connect(&database_url).await?;
            hardmoney::db::ensure_schema(&pool).await?;
            hardmoney::api::serve(pool, bind).await?;
        }
    }
    Ok(())
}

async fn connect(database_url: &str) -> Result<sqlx::PgPool, sqlx::Error> {
    sqlx::postgres::PgPoolOptions::new().max_connections(10).connect(database_url).await
}
