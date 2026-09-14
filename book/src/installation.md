# Installation

## As a Rust library

Add `hardmoney` to your `Cargo.toml`:

```toml
[dependencies]
hardmoney = "0.1"
```

That pulls in every feature: the parser, the Postgres bulk-ETL module, and
the Axum REST API server. If your program only needs to parse `.fec`
files -- no Postgres, no HTTP server -- you can depend on a much smaller
feature set instead:

```toml
[dependencies]
hardmoney = { version = "0.1", default-features = false, features = ["fetch"] }
```

`fetch` enables `Filing::fetch`, which downloads a raw filing directly
from `docquery.fec.gov` by filing ID instead of requiring you to have the
file on disk already. Drop it too if you only ever parse local files.

See [Feature flags](#feature-flags-reference) at the end of this chapter
for the full list.

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
$ cargo run --quiet --bin hardmoney -- --help
```

If that prints a list of subcommands (`parse`, `schema-init`,
`bulk-load`, `bulk-load-all`, `bulk-restore-dump`, `bulk-load-filing`,
`serve`), you're set up correctly.

## What you need for each part of the crate

Not every chapter of this book needs every dependency:

| If you want to... | You need |
|---|---|
| Parse `.fec` files (Quick Start, [Parsing a Filing](./parsing-explained.md), [Money/Dates/Names](./typed-views.md)) | Just Rust and Cargo. Nothing else. |
| Download a filing live from the FEC ([Quick Start](./quick-start.md)'s live-fetch example) | Network access. No database. |
| Load bulk data into Postgres ([Bulk ETL](./bulk-etl.md)) | A running Postgres server and a database URL. |
| Run the REST API ([The REST API](./rest-api.md)) | The same Postgres database, already loaded with data. |

If you don't have Postgres handy, the fastest way to get one for local
experimentation is usually a container:

```bash
docker run --name fec-postgres -e POSTGRES_PASSWORD=postgres \
  -p 5432:5432 -d postgres:16
```

Then set:

```bash
export DATABASE_URL="postgresql://postgres:postgres@127.0.0.1:5432/postgres"
```

(This book's own examples were run against a local Postgres instance with
a database named `fec_dev`; the exact database name doesn't matter, only
that `DATABASE_URL` points at it.)

## Feature flags reference

| Feature | Enables | Default |
|---|---|---|
| `fetch` | `Filing::fetch` (download a raw filing from `docquery.fec.gov`) | on |
| `serde` | `Serialize`/`Deserialize` on parser types | on (pulled in by `bulk`/`api`) |
| `bulk` | the `bulk` module (Postgres ETL, needs `sqlx`) | on |
| `api` | the `api` module (Axum REST server) | on |
| `cli` | the `hardmoney` binary itself | on |

Turning off `default-features` and picking only what you need keeps
compile times down and avoids pulling in `sqlx`/`axum` if you're only
ever parsing files.
