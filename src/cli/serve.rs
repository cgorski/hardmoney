//! `hardmoney serve`: the REST API server.

use std::net::SocketAddr;
use std::time::Duration;

use clap::Args;
use hardmoney::api::ApiConfig;
use hardmoney::db::{self, DbConfig};

use super::db_args::DbArgs;

#[derive(Args, Debug)]
pub struct ServeArgs {
    #[command(flatten)]
    pub db: DbArgs,
    #[arg(long, default_value = "0.0.0.0:8080")]
    pub bind: SocketAddr,
    /// Per-request timeout in seconds. Queries are also bounded server-side
    /// by a Postgres statement_timeout slightly below this.
    #[arg(long, default_value_t = 30)]
    pub timeout_secs: u64,
    /// Allowed CORS origin (repeatable). Default: any origin.
    #[arg(long = "cors-origin")]
    pub cors_origins: Vec<String>,
    /// Require this key in `X-Api-Key` (or `?api_key=`) on every route
    /// except /health.
    #[arg(long, env = "HARDMONEY_API_KEY", hide_env_values = true)]
    pub api_key: Option<String>,
    /// Maximum database connections in the pool.
    #[arg(long, default_value_t = 10)]
    pub max_connections: u32,
    /// Also serve the browser UI at /ui and the filing tools at /tools/*
    /// (parse, validate, reconcile, write). Raises the request body cap
    /// from 64 KiB to 32 MiB so a filing can be uploaded.
    #[arg(long)]
    pub ui: bool,
}

pub async fn run(args: ServeArgs) -> super::CliResult {
    let timeout = Duration::from_secs(args.timeout_secs.max(1));
    let config: DbConfig = args
        .db
        .config()
        .max_connections(args.max_connections)
        // Give Postgres a chance to cancel the query before the HTTP
        // timeout fires, so a slow query doesn't keep running server-side.
        .statement_timeout(
            timeout
                .saturating_sub(Duration::from_secs(2))
                .max(Duration::from_secs(1)),
        );
    let pool = db::connect(&config).await?;
    let status = db::migration_status(&pool).await?;
    if !status.is_current() {
        return Err(format!(
            "namespace '{}' has {} pending migration(s); run `hardmoney schema-init` first",
            args.db.schema,
            status.pending.len()
        )
        .into());
    }
    db::ensure_views(&pool).await?;

    let api = ApiConfig::new(args.bind)
        .request_timeout(timeout)
        .cors_origins(args.cors_origins)
        .api_key(args.api_key)
        .ui(args.ui);
    hardmoney::api::serve(pool, api).await?;
    Ok(())
}
