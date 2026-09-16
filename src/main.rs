//! `hardmoney` CLI: parse filings, manage the Postgres schema, run the
//! bulk-data ETL, ingest filings, and serve the REST API -- all backed by
//! the `hardmoney` library.

mod cli;

#[tokio::main]
async fn main() {
    // Best-effort: load `.env` from the current directory (e.g. DATABASE_URL)
    // if present. Only that one file: `dotenvy::dotenv()` would also search
    // every parent directory, so a `.env` left in `~` or a checkout above
    // the working folder could silently repoint DATABASE_URL or the
    // `HARDMONEY_*` FEC endpoints. Explicit environment variables and flags
    // still win, since dotenvy never overrides a variable that's already set.
    let _ = dotenvy::from_path(".env");
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,sqlx=warn")),
        )
        .with_writer(std::io::stderr)
        .init();

    if let Err(e) = cli::run().await {
        eprintln!("error: {e}");
        let mut source = std::error::Error::source(&*e);
        while let Some(s) = source {
            eprintln!("  caused by: {s}");
            source = s.source();
        }
        std::process::exit(1);
    }
}
