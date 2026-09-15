//! Shared `--database-url` / `--schema` arguments and pool construction.

use clap::Args;
use hardmoney::db::{DbConfig, Namespace};

#[derive(Args, Debug, Clone)]
pub struct DbArgs {
    /// Postgres connection URL. Include a username: postgres://user@host/db
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: String,

    /// Namespace (Postgres schema) to operate in. Each namespace is a fully
    /// isolated set of hardmoney tables: use one per cycle, per snapshot,
    /// per investigation, or per CI run. Default: public.
    #[arg(long, env = "HARDMONEY_SCHEMA", default_value = "public")]
    pub schema: Namespace,
}

impl DbArgs {
    pub fn config(&self) -> DbConfig {
        DbConfig::new(&self.database_url).namespace(self.schema.clone())
    }

    /// Connects and refuses to proceed if the namespace's schema is behind
    /// the embedded migrations (so a load can never write into a stale
    /// table layout).
    pub async fn connect_current(&self) -> super::CliResult<sqlx::PgPool> {
        let pool = hardmoney::db::connect(&self.config()).await?;
        let status = hardmoney::db::migration_status(&pool).await?;
        if !status.is_current() {
            return Err(format!(
                "namespace '{}' has {} pending migration(s); run `hardmoney schema-init` first",
                self.schema,
                status.pending.len()
            )
            .into());
        }
        Ok(pool)
    }
}
