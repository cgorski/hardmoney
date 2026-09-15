//! `hardmoney schema-*`: migrations and namespace management.

use clap::Args;
use hardmoney::db::{self, Namespace};

use super::db_args::DbArgs;

#[derive(Args, Debug)]
pub struct SchemaInitArgs {
    #[command(flatten)]
    pub db: DbArgs,
}

pub async fn init(args: SchemaInitArgs) -> super::CliResult {
    let pool = db::connect(&args.db.config()).await?;
    let before = db::migration_status(&pool).await?;
    db::migrate(&pool).await?;
    let after = db::migration_status(&pool).await?;
    let applied_now = after.applied.len().saturating_sub(before.applied.len());
    println!(
        "namespace '{}': schema at version {} ({} migration(s) applied now)",
        args.db.schema,
        after.applied.last().copied().unwrap_or(0),
        applied_now
    );
    Ok(())
}

#[derive(Args, Debug)]
pub struct SchemaStatusArgs {
    #[command(flatten)]
    pub db: DbArgs,
}

pub async fn status(args: SchemaStatusArgs) -> super::CliResult {
    let pool = db::connect(&args.db.config()).await?;
    let st = db::migration_status(&pool).await?;
    println!("namespace:  {}", args.db.schema);
    println!("applied:    {:?}", st.applied);
    println!("pending:    {:?}", st.pending);
    if st.applied.is_empty() {
        println!("(never initialized; run `hardmoney schema-init`)");
        return Ok(());
    }
    let loads = sqlx::query_as::<_, LatestLoad>(
        "SELECT DISTINCT ON (source, cycle) source, cycle, loaded_at, mode, row_count, row_limit \
         FROM loads ORDER BY source, cycle, loaded_at DESC",
    )
    .fetch_all(&pool)
    .await?;
    if loads.is_empty() {
        println!("loads:      (none)");
    } else {
        println!("loads (latest per source/cycle):");
        for l in loads {
            let cyc = l.cycle.map(|c| c.to_string()).unwrap_or_else(|| "-".into());
            let sample = l
                .row_limit
                .map(|n| format!(" (sample, limit {n})"))
                .unwrap_or_default();
            println!(
                "  {:<40} cycle {cyc:<5} {:>12} rows  {:<7} {}{sample}",
                l.source,
                l.row_count,
                l.mode,
                l.loaded_at.format("%Y-%m-%d %H:%M:%SZ")
            );
        }
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct LatestLoad {
    source: String,
    cycle: Option<i32>,
    loaded_at: chrono::DateTime<chrono::Utc>,
    mode: String,
    row_count: i64,
    row_limit: Option<i64>,
}

#[derive(Args, Debug)]
pub struct SchemaListArgs {
    #[command(flatten)]
    pub db: DbArgs,
}

pub async fn list(args: SchemaListArgs) -> super::CliResult {
    let pool = db::connect(&args.db.config()).await?;
    for ns in db::list_namespaces(&pool).await? {
        println!("{ns}");
    }
    Ok(())
}

#[derive(Args, Debug)]
pub struct SchemaDropArgs {
    #[command(flatten)]
    pub db: DbArgs,
    /// The namespace to drop (must not be `public`).
    pub name: Namespace,
    /// Confirm the drop.
    #[arg(long)]
    pub yes: bool,
}

pub async fn drop(args: SchemaDropArgs) -> super::CliResult {
    if !args.yes {
        return Err(format!(
            "this will permanently delete namespace '{}' and everything in it; re-run with --yes",
            args.name
        )
        .into());
    }
    let pool = db::connect(&args.db.config()).await?;
    db::drop_namespace(&pool, &args.name).await?;
    println!("dropped namespace '{}'", args.name);
    Ok(())
}
