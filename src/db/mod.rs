//! Postgres schema management shared by `bulk` (the ETL writer) and `api`
//! (the reader).
//!
//! # Migrations
//!
//! The schema lives in `migrations/*.sql` and is applied with
//! [`sqlx::migrate!`], which records each file in `_sqlx_migrations` and
//! refuses to run if an applied file has been edited. Adding a column,
//! changing a type, or redefining an index is a *new* numbered file --
//! never an edit to an old one. (`CREATE TABLE IF NOT EXISTS` scripts, which
//! this replaced, silently no-op on all three of those changes.)
//!
//! # Namespaces
//!
//! Every table is unqualified, so it lives in whatever schema is first on
//! the connection's `search_path`. [`connect`] sets that from a
//! [`Namespace`], which is how one database hosts many independent
//! "sessions" of FEC data -- a 2024 load and a 2026 load, a full load and a
//! `--limit` sample, two weekly snapshots for A/B comparison, a CI run --
//! with zero query changes: the API, loader, and migrations all just see
//! their own tables. Each namespace has its own `_sqlx_migrations`, so
//! namespaces can be at different schema versions.
//!
//! The FEC's own `pg_dump` archives always restore into the fixed
//! `disclosure` schema (their DDL hard-codes it); that is reference data
//! shared across namespaces, and [`ensure_views`] wires the friendly
//! `independent_expenditures` view over it inside each namespace.

use sqlx::PgPool;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};

/// The embedded migration set from `migrations/`.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// A Postgres schema name used as an isolated hardmoney session.
///
/// Validated to a conservative identifier grammar (`[a-z_][a-z0-9_]*`, max
/// 63 bytes) so it can be interpolated into `CREATE SCHEMA` / `search_path`
/// without quoting concerns.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Namespace(String);

/// Why a string is not a valid [`Namespace`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum NamespaceError {
    #[error(
        "invalid namespace '{0}': must match [a-z_][a-z0-9_]* and be at most 63 bytes (Postgres schema name)"
    )]
    Invalid(String),
    #[error("namespace '{0}' is reserved")]
    Reserved(String),
}

impl Namespace {
    /// The default namespace when none is configured.
    pub const PUBLIC: &'static str = "public";

    pub fn new(name: impl Into<String>) -> Result<Self, NamespaceError> {
        let name = name.into();
        let mut chars = name.chars();
        let valid_start = chars
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c == '_');
        let valid_rest = chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_');
        if !valid_start || !valid_rest || name.len() > 63 {
            return Err(NamespaceError::Invalid(name));
        }
        if name.starts_with("pg_") || name == "information_schema" || name == "disclosure" {
            return Err(NamespaceError::Reserved(name));
        }
        Ok(Namespace(name))
    }

    pub fn public() -> Self {
        Namespace(Self::PUBLIC.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_public(&self) -> bool {
        self.0 == Self::PUBLIC
    }
}

impl Default for Namespace {
    fn default() -> Self {
        Self::public()
    }
}

impl std::fmt::Display for Namespace {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::str::FromStr for Namespace {
    type Err = NamespaceError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Namespace::new(s)
    }
}

/// Connection settings for a hardmoney database session.
#[derive(Debug, Clone)]
pub struct DbConfig {
    pub url: String,
    pub namespace: Namespace,
    pub max_connections: u32,
    /// Per-statement timeout applied to every connection in the pool.
    /// `None` for bulk-load pools, which legitimately run long `COPY`s.
    pub statement_timeout: Option<std::time::Duration>,
}

impl DbConfig {
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            namespace: Namespace::public(),
            max_connections: 10,
            statement_timeout: None,
        }
    }

    pub fn namespace(mut self, ns: Namespace) -> Self {
        self.namespace = ns;
        self
    }

    pub fn max_connections(mut self, n: u32) -> Self {
        self.max_connections = n;
        self
    }

    pub fn statement_timeout(mut self, d: std::time::Duration) -> Self {
        self.statement_timeout = Some(d);
        self
    }
}

/// Opens a pool whose every connection has `search_path` set to the
/// configured namespace (plus `public`, so the `disclosure`-schema view and
/// any extensions resolve). The namespace's schema is created if missing.
pub async fn connect(config: &DbConfig) -> Result<PgPool, sqlx::Error> {
    let mut options: PgConnectOptions = config.url.parse()?;
    let mut session_options: Vec<(&str, String)> = vec![(
        "search_path",
        format!("{},public", config.namespace.as_str()),
    )];
    if let Some(t) = config.statement_timeout {
        session_options.push(("statement_timeout", format!("{}ms", t.as_millis())));
    }
    options = options.options(session_options);

    let pool = PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(std::time::Duration::from_secs(30))
        .connect_with(options)
        .await?;

    if !config.namespace.is_public() {
        // Validated identifier; safe to interpolate.
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE SCHEMA IF NOT EXISTS {}",
            config.namespace.as_str()
        )))
        .execute(&pool)
        .await?;
    }
    Ok(pool)
}

/// Applies any pending migrations in the pool's namespace, then (re)creates
/// the dependent views. Safe to call repeatedly.
pub async fn migrate(pool: &PgPool) -> Result<(), sqlx::Error> {
    MIGRATOR.run(pool).await?;
    ensure_views(pool).await
}

/// How far the pool's namespace is from the embedded migration set.
pub async fn migration_status(pool: &PgPool) -> Result<MigrationStatus, sqlx::Error> {
    // Schema-qualify explicitly. With `search_path = <ns>,public`, an
    // unqualified `_sqlx_migrations` in a brand-new namespace would resolve
    // to `public._sqlx_migrations` and report the *other* namespace's state
    // -- which would let a bulk load COPY into public's tables.
    let (exists,): (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables \
                        WHERE table_schema = current_schema() AND table_name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?;
    let applied: Vec<i64> = if exists {
        let rows: Vec<(i64,)> =
            sqlx::query_as("SELECT version FROM _sqlx_migrations WHERE success ORDER BY version")
                .fetch_all(pool)
                .await?;
        rows.into_iter().map(|(v,)| v).collect()
    } else {
        Vec::new()
    };
    let pending: Vec<i64> = MIGRATOR
        .iter()
        .map(|m| m.version)
        .filter(|v| !applied.contains(v))
        .collect();
    Ok(MigrationStatus { applied, pending })
}

/// Result of [`migration_status`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationStatus {
    pub applied: Vec<i64>,
    pub pending: Vec<i64>,
}

impl MigrationStatus {
    pub fn is_current(&self) -> bool {
        self.pending.is_empty()
    }
}

/// Creates the `independent_expenditures` view over the FEC's own
/// `disclosure.fec_fitem_sched_e` (restored by `hardmoney bulk-restore-dump
/// schedule_e`) if that table exists; otherwise a no-op. Idempotent: uses
/// `DROP VIEW` + `CREATE VIEW` because Postgres refuses `CREATE OR REPLACE`
/// when a column's type changes.
///
/// Lives outside the migration set because it depends on out-of-band state
/// (whether the dump has been restored) and must be re-run after a restore.
pub async fn ensure_views(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(INDEPENDENT_EXPENDITURES_VIEW_SQL)
        .execute(pool)
        .await?;
    Ok(())
}

const INDEPENDENT_EXPENDITURES_VIEW_SQL: &str = r#"
DO $$
BEGIN
    IF to_regclass('disclosure.fec_fitem_sched_e') IS NOT NULL THEN
        EXECUTE 'DROP VIEW IF EXISTS independent_expenditures';
        EXECUTE '
            CREATE VIEW independent_expenditures AS
            SELECT
                sub_id::bigint     AS sub_id,
                file_num::bigint   AS file_num,
                cmte_id,
                cmte_nm            AS committee_name,
                pye_nm             AS payee_name,
                s_o_cand_id        AS candidate_id,
                s_o_cand_nm        AS candidate_name,
                s_o_cand_office    AS candidate_office,
                s_o_cand_office_st AS candidate_office_state,
                s_o_ind            AS support_oppose_code,
                s_o_ind_desc       AS support_oppose_desc,
                exp_amt            AS expenditure_amt,
                exp_dt             AS expenditure_date,
                exp_desc           AS expenditure_description,
                catg_cd_desc       AS category_desc,
                election_tp,
                rpt_yr,
                election_cycle,
                filing_form,
                image_num,
                dissem_dt          AS disseminated_at
            FROM disclosure.fec_fitem_sched_e
        ';
    END IF;
END
$$;
"#;

/// Lists hardmoney namespaces: every schema containing a `_sqlx_migrations`
/// table.
pub async fn list_namespaces(pool: &PgPool) -> Result<Vec<String>, sqlx::Error> {
    let rows: Vec<(String,)> = sqlx::query_as(
        "SELECT table_schema::text FROM information_schema.tables \
         WHERE table_name = '_sqlx_migrations' ORDER BY 1",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows.into_iter().map(|(s,)| s).collect())
}

/// Drops a namespace and everything in it. Refuses `public`.
pub async fn drop_namespace(pool: &PgPool, ns: &Namespace) -> Result<(), sqlx::Error> {
    if ns.is_public() {
        return Err(sqlx::Error::Protocol(
            "refusing to drop the public schema; drop the database instead".into(),
        ));
    }
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP SCHEMA IF EXISTS {} CASCADE",
        ns.as_str()
    )))
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_grammar() {
        assert!(Namespace::new("snap_2026_09_14").is_ok());
        assert!(Namespace::new("_x").is_ok());
        assert!(Namespace::new("public").is_ok());
        for bad in [
            "",
            "2026",
            "Snap",
            "a-b",
            "a b",
            "a;drop",
            "pg_temp",
            "disclosure",
        ] {
            assert!(Namespace::new(bad).is_err(), "{bad}");
        }
        assert!(Namespace::new("a".repeat(64)).is_err());
        assert!(Namespace::new("a".repeat(63)).is_ok());
    }

    #[test]
    fn migrator_has_the_expected_files() {
        let versions: Vec<i64> = MIGRATOR.iter().map(|m| m.version).collect();
        assert_eq!(versions, vec![1, 2]);
    }
}
