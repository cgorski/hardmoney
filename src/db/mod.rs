//! Shared Postgres schema for `bulk` (the ETL writer) and `api` (the
//! reader). One schema, one source of truth: [`SCHEMA_SQL`] is executed
//! idempotently (`CREATE TABLE IF NOT EXISTS`) by both `hardmoney bulk load`
//! and `hardmoney serve` on startup, so either can be run first.

/// The full DDL for every table `bulk` populates, embedded at compile time
/// from `src/db/schema.sql` (see that file for column-by-column sourcing
/// notes).
pub static SCHEMA_SQL: &str = include_str!("schema.sql");

#[cfg(feature = "bulk")]
/// Applies [`SCHEMA_SQL`] against `pool`. Safe to call repeatedly.
///
/// Uses [`sqlx::raw_sql`], which forwards the whole script to Postgres's
/// simple-query protocol in one round trip -- Postgres itself (not naive
/// string-splitting in this crate) is what correctly treats the file's
/// trailing `DO $$ ... $$` block as a single statement despite its
/// internal semicolons.
pub async fn ensure_schema(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(SCHEMA_SQL).execute(pool).await?;
    Ok(())
}
