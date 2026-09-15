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

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{PgConnection, PgPool};

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

/// Creates this namespace's views over the FEC's own dump tables in
/// `disclosure` (restored by `hardmoney bulk-restore-dump`), each only if
/// its table exists; otherwise a no-op for that view:
///
/// | view | over | columns |
/// |---|---|---|
/// | `independent_expenditures` | `fec_fitem_sched_e` | hardmoney names (`payee_name`, `expenditure_amt`, ...) |
/// | `dump_schedule_a` | `fec_fitem_sched_a` | the FEC's names, plus `cycle` (`two_year_transaction_period::int`) |
/// | `dump_schedule_b` | `fec_fitem_sched_b` | the FEC's names, plus `cycle` |
/// | `dump_schedule_e` | `fec_fitem_sched_e` | the FEC's names, plus `cycle` (`election_cycle::int`) |
/// | `dump_committee_history` | `ofec_committee_history` | the FEC's names (it already has `cycle`) |
///
/// Idempotent: uses `DROP VIEW` + `CREATE VIEW` because Postgres refuses
/// `CREATE OR REPLACE` when a column's type changes. A table dropped by a
/// concurrent restore between the existence check and the `CREATE` is
/// tolerated (the view is simply not created; the restore recreates its
/// own namespace's views, and this namespace's come back the next time
/// this runs -- `serve` and `schema-init` both call it).
///
/// Each block locks `disclosure.<table>` in `ACCESS SHARE` mode *before*
/// dropping the old view. A restore's `DROP TABLE ... CASCADE` takes the
/// table exclusively and then every dependent view, in every namespace;
/// dropping our view first and then touching the table would lock in the
/// opposite order and deadlock against it (observed when the integration
/// tests ran in parallel). Same order on both sides means this simply
/// waits for the restore's `DROP` to finish.
///
/// Lives outside the migration set because it depends on out-of-band state
/// (whether the dump has been restored) and must be re-run after a restore.
pub async fn ensure_views(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(INDEPENDENT_EXPENDITURES_VIEW_SQL)
        .execute(pool)
        .await?;
    ensure_dump_views(pool).await
}

/// Creates the `dump_*` views listed on [`ensure_views`] (and nothing
/// else). Called by `ensure_views`; exposed for callers that restored a
/// dump through their own `pg_restore` and want only these.
pub async fn ensure_dump_views(pool: &PgPool) -> Result<(), sqlx::Error> {
    for (view, table, cycle_expr) in DUMP_VIEWS {
        let sql = dump_view_sql(view, table, cycle_expr);
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(pool)
            .await?;
    }
    Ok(())
}

/// `(view name, disclosure table, cycle column expression or None)`.
const DUMP_VIEWS: [(&str, &str, Option<&str>); 4] = [
    (
        "dump_schedule_a",
        "fec_fitem_sched_a",
        Some("two_year_transaction_period::int"),
    ),
    (
        "dump_schedule_b",
        "fec_fitem_sched_b",
        Some("two_year_transaction_period::int"),
    ),
    (
        "dump_schedule_e",
        "fec_fitem_sched_e",
        Some("election_cycle::int"),
    ),
    ("dump_committee_history", "ofec_committee_history", None),
];

/// The `DO` block that (re)creates one `dump_*` view if its table exists.
/// Only compile-time constants are interpolated.
fn dump_view_sql(view: &str, table: &str, cycle_expr: Option<&str>) -> String {
    let select = match cycle_expr {
        Some(expr) => format!("SELECT t.*, t.{expr} AS cycle FROM disclosure.{table} t"),
        None => format!("SELECT t.* FROM disclosure.{table} t"),
    };
    format!(
        "DO $$\n\
         BEGIN\n\
             IF to_regclass('disclosure.{table}') IS NOT NULL THEN\n\
                 EXECUTE 'LOCK TABLE disclosure.{table} IN ACCESS SHARE MODE';\n\
                 EXECUTE 'DROP VIEW IF EXISTS {view}';\n\
                 EXECUTE 'CREATE VIEW {view} AS {select}';\n\
             END IF;\n\
         EXCEPTION WHEN undefined_table THEN\n\
             NULL;\n\
         END\n\
         $$;"
    )
}

const INDEPENDENT_EXPENDITURES_VIEW_SQL: &str = r#"
DO $$
BEGIN
    IF to_regclass('disclosure.fec_fitem_sched_e') IS NOT NULL THEN
        EXECUTE 'LOCK TABLE disclosure.fec_fitem_sched_e IN ACCESS SHARE MODE';
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
EXCEPTION WHEN undefined_table THEN
    NULL;
END
$$;
"#;

/// The chain-resolution statement, parameterised on how the rows to
/// recompute are chosen (see [`resolve_amendment_chain`] for the rules it
/// encodes). `concat!` needs literals, so this is a macro rather than a
/// `format!` at call time: the SQL is fixed at compile time and never
/// interpolates data.
///
/// Shape: `scope` picks candidate rows and derives each one's chain key
/// (`original_id`); `members` keeps the rows whose partition is complete
/// within `scope`; `ranked` numbers each partition with window functions;
/// the `UPDATE` writes the six derived columns in one set-based statement.
macro_rules! resolve_chain_sql {
    ($scope:literal, $members:literal) => {
        concat!(
            "WITH scope AS (",
            "  SELECT f.filing_id, f.is_amendment, f.amendment_number,",
            "         CASE WHEN f.is_amendment",
            "               AND f.amends_filing_id IS NOT NULL",
            "               AND f.amends_filing_id <> f.filing_id",
            "               AND EXISTS (SELECT 1 FROM filings o",
            "                           WHERE o.filing_id = f.amends_filing_id",
            "                             AND NOT o.is_amendment)",
            "              THEN f.amends_filing_id",
            "              ELSE f.filing_id END AS original_id",
            "  FROM filings f ",
            $scope,
            "), ",
            "members AS (SELECT * FROM scope s ",
            $members,
            "), ",
            "ranked AS (",
            "  SELECT filing_id, original_id, is_amendment,",
            "         (row_number() OVER w - 1)::int AS version,",
            "         array_agg(filing_id) OVER (w ROWS BETWEEN UNBOUNDED PRECEDING AND CURRENT ROW) AS chain,",
            "         lag(filing_id) OVER w AS previous_id,",
            "         last_value(filing_id) OVER (w ROWS BETWEEN UNBOUNDED PRECEDING AND UNBOUNDED FOLLOWING) AS latest_id",
            "  FROM members",
            "  WINDOW w AS (PARTITION BY original_id",
            "               ORDER BY (filing_id <> original_id), amendment_number NULLS FIRST, filing_id)",
            ") ",
            "UPDATE filings f",
            "   SET amendment_version     = r.version,",
            "       amendment_chain       = r.chain,",
            "       most_recent           = (f.filing_id = r.latest_id),",
            "       most_recent_filing_id = r.latest_id,",
            "       previous_filing_id    = COALESCE(r.previous_id, f.filing_id),",
            "       chain_unresolved      = (r.is_amendment AND r.original_id = r.filing_id)",
            "  FROM ranked r",
            " WHERE f.filing_id = r.filing_id",
        )
    };
}

/// One chain: candidate rows are the key itself plus everything whose
/// header names it (both indexed). A row is kept when its derived key is
/// `$1` -- the partition is then complete, because every member of a chain
/// keyed by a non-amendment either *is* that filing or has
/// `amends_filing_id` equal to it -- or when it is an amendment that stands
/// alone (its partition is just itself). A non-amendment that merely
/// *mentions* `$1` in its header is left for its own key: its amendments
/// are outside this scope, so recomputing it here would be wrong.
const RESOLVE_ONE_CHAIN_SQL: &str = resolve_chain_sql!(
    "WHERE f.filing_id = $1 OR f.amends_filing_id = $1",
    "WHERE s.original_id = $1 OR (s.original_id = s.filing_id AND s.is_amendment)"
);

/// The whole table: every partition is complete, so no member filter.
const RESOLVE_ALL_CHAINS_SQL: &str = resolve_chain_sql!("", "");

/// Recomputes the amendment-chain columns of `filings` for the chain keyed
/// by `original_id`, in one set-based `UPDATE`, and returns how many rows
/// were written. Idempotent; safe to call for an id that is not in the
/// table (returns `Ok(0)` unless amendments already point at it).
///
/// # Semantics (openFEC's)
///
/// A chain is keyed by its **original** filing. An amendment (`form_type`
/// ending in `A`) joins the chain named by its header's `FEC-<n>`
/// (`amends_filing_id`) when that filing is in the table and is itself not
/// an amendment. Within a chain, members are ordered original first, then
/// by the filed amendment number (`amendment_number NULLS FIRST`, so an
/// amendment that omitted its number never outranks a numbered one), then
/// by `filing_id` (the FEC assigns ids chronologically, so of two
/// amendments with the same number the later filing wins). Each member
/// gets:
///
/// * `amendment_version` -- 0 for the original, 1, 2, ... after;
/// * `amendment_chain` -- the ids up to and including this one;
/// * `most_recent_filing_id` -- the last member; `most_recent` is true on
///   that member only;
/// * `previous_filing_id` -- the preceding member, or itself for the
///   original (as openFEC's `previous_file_number` does);
/// * `chain_unresolved` -- false.
///
/// A non-amendment is always the head of its own chain, whatever its
/// header says: Form 99s and RFAI responses (`FRQ`) never carry the `A`
/// designator and so always stand alone, `chain = [self]`,
/// `most_recent = true`. Registrations (F1, F1M, F2) and 24/48-hour
/// notices (F24, F6) amend by the same header rule and are treated
/// uniformly.
///
/// An amendment that cannot be attached -- its header had no usable
/// `FEC-<n>`, or names itself, or names a filing that is not in the table,
/// or names a filing that is itself an amendment (the FEC's validator
/// rejects that; the header must name the original) -- also stands alone
/// with `chain = [self]`, `amendment_version = 0`, `most_recent = true`,
/// and **`chain_unresolved = true`** so the gap is visible. Ingesting the
/// missing original later re-resolves it into the chain
/// ([`crate::bulk::ingest_filing`] calls this after every insert), so the
/// result does not depend on the order filings arrive in.
///
/// # Concurrency
///
/// Resolutions of the same chain are serialised with a transaction-scoped
/// advisory lock on `original_id` (`pg_advisory_xact_lock`). Without it
/// two ingests of amendments to one original, running at the same time
/// under `READ COMMITTED`, each compute the chain from a snapshot that
/// lacks the other's uncommitted row and both end up `most_recent`; with
/// it the second waits for the first to commit and then sees its row.
///
/// # Errors
///
/// Any database error, including a namespace whose schema predates
/// migration `0003` (the columns do not exist).
pub async fn resolve_amendment_chain(
    pool: &PgPool,
    original_id: i64,
) -> Result<usize, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let n = resolve_amendment_chain_in(&mut tx, original_id).await?;
    tx.commit().await?;
    Ok(n)
}

/// [`resolve_amendment_chain`] on an existing connection or transaction,
/// so an ingester can make the insert and the resolution atomic. Takes
/// `chain_lock` on `original_id` first; a caller that will resolve two
/// chains in one transaction should take both locks up front, in
/// ascending order, so two such transactions cannot deadlock.
pub(crate) async fn resolve_amendment_chain_in(
    conn: &mut PgConnection,
    original_id: i64,
) -> Result<usize, sqlx::Error> {
    chain_lock(&mut *conn, original_id).await?;
    let done = sqlx::query(RESOLVE_ONE_CHAIN_SQL)
        .bind(original_id)
        .execute(conn)
        .await?;
    Ok(usize::try_from(done.rows_affected()).unwrap_or(usize::MAX))
}

/// Takes the transaction-scoped advisory lock that serialises chain
/// resolution for `original_id` (released at commit or rollback; taking it
/// again in the same transaction is a no-op). The key is the filing id
/// itself, so two namespaces in one database that hold the same filing id
/// contend with each other -- harmless, since a resolution is milliseconds.
pub(crate) async fn chain_lock(
    conn: &mut PgConnection,
    original_id: i64,
) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(original_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Recomputes the amendment-chain columns for **every** row of `filings`
/// in one set-based statement (no per-row round trips) and returns the
/// number of rows written -- the whole table, since every row gets a
/// value. Same rules as [`resolve_amendment_chain`]. Use it after a batch
/// ingested with [`crate::bulk::ingest::ChainResolution::Defer`], after
/// applying migration `0003` to a namespace that already held filings, or
/// whenever the derived columns are in doubt.
///
/// # Errors
///
/// Any database error.
pub async fn resolve_all_amendment_chains(pool: &PgPool) -> Result<usize, sqlx::Error> {
    let done = sqlx::query(RESOLVE_ALL_CHAINS_SQL).execute(pool).await?;
    Ok(usize::try_from(done.rows_affected()).unwrap_or(usize::MAX))
}

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
        assert_eq!(versions, vec![1, 2, 3]);
    }
}
