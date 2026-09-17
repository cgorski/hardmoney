# Installation

## Python

```bash
pip install hardmoney
```

Wheels are published for Linux (x86_64, aarch64) and macOS (Intel and
Apple silicon), CPython 3.9 and newer; the package has no Python
dependencies. Other platforms build from the source distribution, which
needs a Rust toolchain. Confirm it works:

```bash
$ python -c "import hardmoney; print(hardmoney.__version__, hardmoney.BUNDLED_SPEC_VERSION)"
3.0.0 8.5
```

The Python package is parser-only (parse, write, validate, reconcile);
the Postgres ETL and REST API in this book are Rust and CLI only. Start
with [Getting started with Python](./python.md); the
[Python cookbook](./python-cookbook.md) and
[Python API reference](./python-api.md) follow it.

## As a Rust library

Add `hardmoney` to your `Cargo.toml`:

```toml
[dependencies]
hardmoney = "3"
```

That pulls in every feature: the parser, the Postgres bulk-ETL module, and
the Axum REST API server. If your program only needs to parse `.fec`
files (no Postgres, no HTTP server), you can depend on a much smaller
feature set instead:

```toml
[dependencies]
hardmoney = { version = "3", default-features = false, features = ["fetch"] }
```

`fetch` enables `Filing::fetch`, which downloads a raw filing directly
from `docquery.fec.gov` by filing ID instead of requiring you to have the
file on disk already. Drop it too if you only ever parse local files.

The minimum supported Rust version is 1.94 (the crate uses the 2024
edition). See [Feature flags](#feature-flags-reference) at the end of this
chapter for the full list.

## As a command-line tool

Clone the repository and build the `hardmoney` binary with Cargo:

```bash
git clone https://github.com/cgorski/hardmoney.git
cd hardmoney
cargo build --release --all-features
```

The binary ends up at `target/release/hardmoney`. This book's examples
run it straight from a debug build (`cargo run --` or
`./target/debug/hardmoney`) since that's what you'll be doing while
following along; use `--release` once you're past experimenting, since
parsing large bulk files is meaningfully faster with optimizations on.

To confirm it built correctly:

```bash
$ cargo run --quiet --bin hardmoney -- --version
hardmoney 2.0.0
$ cargo run --quiet --bin hardmoney -- --help
```

```text
FEC campaign-finance data: parse .fec filings, load bulk data, serve a REST API

Usage: hardmoney <COMMAND>

Commands:
  parse              Parse a single `.fec` filing and print a JSON summary
  schema-init        Create or upgrade the Postgres schema in the target namespace
  schema-status      Show migration state and recorded loads for the target namespace
  schema-list        List hardmoney namespaces (schemas) in the database
  schema-drop        Drop a namespace and all data in it
  bulk-load          Load one bulk-data source into Postgres
  bulk-load-all      Load every bulk source for one cycle
  bulk-restore-dump  Restore one of the FEC's own official pg_dump archives
  bulk-load-filing   Ingest a single raw `.fec` filing directly
  serve              Run the REST API server
  help               Print this message or the help of the given subcommand(s)

Options:
  -h, --help     Print help
  -V, --version  Print version

Database commands read DATABASE_URL (or --database-url) and an optional HARDMONEY_SCHEMA (or --schema) naming an isolated namespace inside that database. Include a username in the URL: postgres://user@host/db
```

If you see that list of ten subcommands, you're set up correctly.

## What you need for each part of the crate

Not every chapter of this book needs every dependency:

| If you want to... | You need |
|---|---|
| Parse `.fec` files ([Quick start](./quick-start.md), [Parsing a filing](./parsing-explained.md), [Tables and typed views](./typed-views.md)) | Just Rust and Cargo. Nothing else. |
| Download a filing live from the FEC ([Quick start](./quick-start.md)'s live-fetch example) | Network access. No database. |
| Load bulk data into Postgres ([Bulk ETL](./bulk-etl.md)) | A running Postgres server and a database URL. |
| Restore the FEC's own `pg_dump` archives ([Bulk ETL](./bulk-etl.md#an-alternative-restoring-the-fecs-own-database-dumps)) | The same, plus `pg_restore` on your `PATH`. |
| Run the REST API ([The REST API](./rest-api.md)) | The same Postgres database, already loaded with data. |

If you don't have Postgres handy, the repository ships a compose file
that brings up the same container the test suite runs against in CI
(Postgres 18 by default; `PG_VERSION=15` for the major the FEC itself
runs), on host port 5433 so a Postgres you already have on 5432 is left
alone:

```bash
scripts/dev-postgres.sh up      # docker compose up -d --wait, then prints the URL
scripts/dev-postgres.sh psql    # a psql session in it
scripts/dev-postgres.sh down    # stop and delete the data
```

Then set what `up` printed:

```bash
export DATABASE_URL="postgres://hardmoney:hardmoney@127.0.0.1:5433/hardmoney_test"
```

(Without Docker Compose, the one-liner is `docker run --name fec-postgres
-e POSTGRES_PASSWORD=postgres -p 5432:5432 -d postgres:18` and
`export DATABASE_URL="postgres://postgres:postgres@127.0.0.1:5432/postgres"`.)

Two things about that URL. Include a username: if the URL has no user
(`postgres://localhost/fec`), `sqlx` falls back to your operating-system
username, which is only right when your Postgres role happens to share
it. An explicit `user@` is the form every example in this book uses, and
the form the CLI's own help text recommends. The database name doesn't
matter: this book's database examples were run against a local Postgres
18 database named `hardmoney_v1_test`; yours can be called anything, as
long as `DATABASE_URL` points at it and it already exists (Postgres
doesn't auto-create databases; see
[Troubleshooting](./troubleshooting.md)).

Every database command also accepts `--schema <name>` (or the
`HARDMONEY_SCHEMA` environment variable) to work inside an isolated
namespace within that database. You don't need one to get started (the
default is Postgres's `public` schema). They get their own chapter:
[Namespaces](./namespaces.md).

## Feature flags reference

| Feature | Enables | Default |
|---|---|---|
| `fetch` | `Filing::fetch` (download a raw filing from `docquery.fec.gov`) | on |
| `serde` | `Serialize`/`Deserialize` on parser types | on (pulled in by `bulk`/`api`) |
| `bulk` | the `bulk`, `db`, and `cycle` modules (Postgres ETL, needs `sqlx`) | on |
| `api` | the `api` module (Axum REST server) | on |
| `cli` | the `hardmoney` binary itself | on |

Turning off `default-features` and picking only what you need keeps
compile times down and avoids pulling in `sqlx`/`axum`/`tokio` if you're
only ever parsing files. All four bundled examples build under the
minimal `--no-default-features --features fetch` set.

## Running the test suite

The parser and format-table tests need nothing but Cargo:

```bash
cargo test --all-features
```

The end-to-end Postgres tests (migrations, namespace isolation,
replace/append semantics, filing ingestion, and the REST API) only run
when you point them at a database; otherwise they print a skip notice and
pass:

```bash
HARDMONEY_TEST_DATABASE_URL="postgres://postgres:postgres@127.0.0.1:5432/postgres" \
  cargo test --all-features
```

Each integration test works in its own randomly-named namespace and drops
it when it finishes, so it's safe to point at a database you're also
using for something else.
