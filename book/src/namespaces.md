# Namespaces: Many Sessions, One Database

Every hardmoney table name is unqualified -- `candidates`, not
`public.candidates` -- so a table lives in whatever Postgres schema is
first on the connection's `search_path`. That one fact is the whole
mechanism behind **namespaces**: pass `--schema <name>` (or set
`HARDMONEY_SCHEMA`) on any database command, and every table that
command reads or writes lives inside that Postgres schema. Nothing else
changes -- not the SQL, not the migrations, not the API.

The result is that one database can hold many fully independent sessions
of FEC data:

- a 2024 load and a 2026 load,
- a complete load and a `--limit` sample for development,
- last week's snapshot and this week's, for A/B comparison,
- one namespace per investigation, so a story's data can be frozen and
  handed around,
- one namespace per CI run, dropped when the run finishes (this is
  exactly how `tests/postgres_integration.rs` works).

## Creating and inspecting one

You don't create a namespace explicitly. The first command that connects
with `--schema <name>` runs `CREATE SCHEMA IF NOT EXISTS`, and then
`schema-init` migrates it:

```bash
$ export DATABASE_URL="postgres://chris.gorski@localhost:5432/hardmoney_v1_test"
$ cargo run --quiet --bin hardmoney -- schema-status --schema book_demo
```

```text
namespace:  book_demo
applied:    []
pending:    [1, 2]
(never initialized; run `hardmoney schema-init`)
```

```bash
$ cargo run --quiet --bin hardmoney -- schema-init --schema book_demo
```

```text
namespace 'book_demo': schema at version 2 (2 migration(s) applied now)
```

Each namespace has its **own** `_sqlx_migrations` table, so different
namespaces can sit at different schema versions -- an old snapshot stays
at the version it was loaded with until you `schema-init` it. (The
status check is careful to look at the namespace's own migrations table
rather than falling through `search_path` to `public`'s; otherwise a
brand-new namespace could report `public`'s state and let a load `COPY`
into the wrong schema.)

`schema-list` shows every namespace in the database -- defined as every
schema that contains a `_sqlx_migrations` table:

```bash
$ cargo run --quiet --bin hardmoney -- schema-list
```

```text
book_demo
public
snap_a
snap_b
```

## Isolation, demonstrated

The book's database has three filings ingested into `public` and, after
[the previous chapter](./bulk-etl.md), a different set in `book_demo`.
Serving each namespace shows they don't see each other -- here's a
filing that exists in `public` requested from a server running against
`book_demo`:

```bash
$ cargo run --quiet --bin hardmoney -- serve --schema book_demo --bind 127.0.0.1:18090 &
$ curl -s -i http://127.0.0.1:18090/filings/2011823 | sed -n '1p;$p'
```

```text
HTTP/1.1 404 Not Found
{"error":"no filing with filing_id 2011823"}
```

And `GET /schema` on that same server reports which namespace it's
serving and what's been loaded there:

```bash
$ curl -s http://127.0.0.1:18090/schema
```

```json
{
  "hardmoney_version": "1.0.0",
  "namespace": "book_demo",
  "migrations_applied": [1, 2],
  "migrations_pending": [],
  "loads": [
    {"source": "candidates", "row_count": 8557, "row_limit": null, "...": "..."},
    {"source": "committees", "row_count": 20674, "row_limit": null, "...": "..."},
    {"source": "disbursements", "row_count": 100, "row_limit": 100, "...": "..."}
  ],
  "independent_expenditures_available": true
}
```

(Trimmed to three of the ten sources and a few fields each; the full
response is shown in [The REST API](./rest-api.md#what-is-loaded-here-the-schema-route).)

## The one thing namespaces don't isolate

`independent_expenditures_available: true` in that response points at
the exception. The FEC's own `pg_dump` archives, restored by
`bulk-restore-dump`, hard-code the `disclosure` schema in their DDL, so
they always land in `disclosure` -- shared across every namespace -- no
matter which `--schema` you pass. That's the right behaviour for what it
is: a ~550,000-row reference table you want restored once, not once per
session.

What *is* per-namespace is the `independent_expenditures` view over it.
`db::ensure_views` creates that view inside the current namespace after
every `schema-init`, every `serve` start, and every restore, if the
`disclosure` table exists; if it doesn't, the view is simply absent and
the API's `/independent-expenditures` route returns a 503 saying so.
That's also why `disclosure` is a reserved namespace name.

## Naming rules

A namespace name must match `[a-z_][a-z0-9_]*`, be at most 63 bytes
(Postgres's identifier limit), and not be `pg_*`, `information_schema`,
or `disclosure`. Lowercase letters, digits, and underscores only -- no
hyphens, no capitals, no spaces. That conservative grammar is what makes
it safe to interpolate the name into `CREATE SCHEMA` and `search_path`
without quoting. Bad names are rejected before anything connects:

```bash
$ cargo run --quiet --bin hardmoney -- schema-status --schema Snap-2026
```

```text
error: invalid value 'Snap-2026' for '--schema <SCHEMA>': invalid namespace 'Snap-2026': must match [a-z_][a-z0-9_]* and be at most 63 bytes (Postgres schema name)

For more information, try '--help'.
```

```bash
$ cargo run --quiet --bin hardmoney -- schema-status --schema disclosure
```

```text
error: invalid value 'disclosure' for '--schema <SCHEMA>': namespace 'disclosure' is reserved

For more information, try '--help'.
```

`public` is the default and is always valid.

## Dropping one

When a session is over, drop the whole namespace -- every table, every
row, its migrations record, its view -- in one command. It refuses to run
without `--yes`:

```bash
$ cargo run --quiet --bin hardmoney -- schema-drop book_demo
```

```text
error: this will permanently delete namespace 'book_demo' and everything in it; re-run with --yes
```

```bash
$ cargo run --quiet --bin hardmoney -- schema-drop book_demo --yes
```

```text
dropped namespace 'book_demo'
```

`schema-drop public` is refused outright, even with `--yes`:

```text
error: encountered unexpected or invalid data: refusing to drop the public schema; drop the database instead
```

The shared `disclosure` schema is not touched by any `schema-drop`.

## In library code

The same thing from Rust is a `Namespace` on a `DbConfig`:

```rust
use hardmoney::db::{self, DbConfig, Namespace};

# async fn run(url: &str) -> Result<(), Box<dyn std::error::Error>> {
let ns = Namespace::new("cycle_2026")?;           // validated here
let pool = db::connect(&DbConfig::new(url).namespace(ns)).await?;
db::migrate(&pool).await?;

let status = db::migration_status(&pool).await?;
println!("applied {:?}, pending {:?}", status.applied, status.pending);
for name in db::list_namespaces(&pool).await? {
    println!("{name}");
}
# Ok(())
# }
```

`Namespace::new("Snap")`, `Namespace::new("pg_temp")`, and
`Namespace::new("disclosure")` all return `Err`; `Namespace::public()` is
the default. `db::drop_namespace(&pool, &ns)` is `schema-drop`.
