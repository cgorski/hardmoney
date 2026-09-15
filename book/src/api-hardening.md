# Hardening the API

`hardmoney serve`'s defaults are tuned for a laptop: no API key, any
CORS origin, a 30-second request timeout. Before you point a public
hostname at it, there are four flags to know about and a handful of
behaviours -- what errors look like, how shutdown works -- that are on
whether you ask for them or not.

## The flags

| Flag | Default | Effect |
|---|---|---|
| `--api-key <key>` (or `HARDMONEY_API_KEY`) | none | Every route except `/health` requires the key, in an `X-Api-Key` header or `?api_key=`. Missing or wrong: 401. |
| `--cors-origin <origin>` (repeatable) | any origin | Switches CORS from permissive to an allow-list of exactly these origins, and restricts methods to `GET` and `HEAD`. |
| `--timeout-secs <n>` | 30 | Wall-clock cap per request; a request that exceeds it gets a 408. Also sets a Postgres `statement_timeout` two seconds shorter on every pooled connection, so a slow query is cancelled *server-side* rather than left running after the client has given up. |
| `--max-connections <n>` | 10 | Size of the database connection pool. |

A production-shaped invocation:

```bash
hardmoney serve --schema cycle_2026 --bind 0.0.0.0:8080 \
    --api-key "$HARDMONEY_API_KEY" \
    --cors-origin https://example.org --cors-origin https://staging.example.org \
    --timeout-secs 20 --max-connections 25
```

The startup log line tells you which posture you're in -- here from a
server started against the book's `book_demo` namespace with both a key
and one allowed origin (`--api-key demo-key --cors-origin https://example.org`):

```text
2026-09-15T02:22:08.847701Z  INFO hardmoney::api: hardmoney API listening bind=127.0.0.1:18090 cors="allow-list" api_key=true
```

(`cors="permissive" api_key=false` is what you see with neither flag.)
The key itself is never logged, and `--help` hides the value of
`HARDMONEY_API_KEY` too.

## The API key

`/health` is always open, so load balancers and uptime checks don't need
the secret:

```bash
$ curl -s -i http://127.0.0.1:18090/health
```

```text
HTTP/1.1 200 OK
ok
```

Everything else is closed without it:

```bash
$ curl -s -i "http://127.0.0.1:18090/candidates?limit=1"
```

```text
HTTP/1.1 401 Unauthorized
{"error":"missing or invalid API key"}
```

A wrong key gets the identical response -- the server doesn't
distinguish "missing" from "incorrect" -- and the comparison is
constant-time, so response timing doesn't leak how many leading bytes
were right. Present it either way:

```bash
curl -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/candidates?limit=1"
curl "http://127.0.0.1:18090/candidates?limit=1&api_key=demo-key"
```

An empty `--api-key ""` (or an empty `HARDMONEY_API_KEY`) is treated as
no key at all, so an unset variable in a deployment template can't
silently lock everyone out -- or, more to the point, can't make you think
you're protected when you aren't; check the `api_key=` field in the
startup log.

## CORS

With no `--cors-origin`, the server answers every origin (the right
thing for a local tool you're poking at from a browser dev console).
With one or more, only those origins get the `Access-Control-Allow-Origin`
header back. From the server above, started with
`--cors-origin https://example.org`:

```bash
$ curl -s -i -H 'Origin: https://example.org' http://127.0.0.1:18090/health | grep -i 'access-control'
```

```text
access-control-allow-origin: https://example.org
```

```bash
$ curl -s -i -H 'Origin: https://evil.example' http://127.0.0.1:18090/health | grep -i 'access-control'
```

(No output: no header, so the browser refuses to hand the response to
the page.) The API is read-only, so the allow-list mode also restricts
methods to `GET` and `HEAD`.

## Errors that tell the client the right amount

Every error the handlers produce is `{"error": "..."}` with a status code
that means something. Three you'll see in practice:

**An odd or out-of-range `?cycle=` is a 400** -- not an empty `[]`, which
is the kind of answer that makes people think their data didn't load:

```bash
$ curl -s -i -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/candidates?cycle=2027"
```

```text
HTTP/1.1 400 Bad Request
{"error":"cycle 2027 is not an even year (cycles are the even year of a two-year period)"}
```

This is the same `hardmoney::Cycle` validation the CLI applies to
`--cycle` ([Bulk ETL](./bulk-etl.md#the-cycle-is-validated)).

**A missing resource is a 404**, with the id in the message:

```text
HTTP/1.1 404 Not Found
{"error":"no filing with filing_id 1"}
```

**A data source that hasn't been loaded is a 503**, with the command that
fixes it. This is what `/independent-expenditures` returns before
`bulk-restore-dump schedule_e` has been run against the database (shown
in the [previous chapter](./rest-api.md#independent-expenditures-from-the-fecs-own-dump)).

A **malformed filter value is a 400** too, but from one layer earlier --
axum's query-string extractor rejects it before the handler runs, so the
body is plain text rather than JSON:

```bash
$ curl -s -i -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/schedule-a?min_amount=notanumber"
```

```text
HTTP/1.1 400 Bad Request
Failed to deserialize query string: min_amount: invalid value: string "notanumber", expected a Decimal type representing a fixed-point number
```

```bash
$ curl -s -i -H 'X-Api-Key: demo-key' "http://127.0.0.1:18090/disbursements?min_date=02/18/2026"
```

```text
HTTP/1.1 400 Bad Request
Failed to deserialize query string: min_date: input contains invalid characters
```

That second one is worth a note: `min_date`/`max_date` take ISO
`YYYY-MM-DD`, not the FEC's `MM/DD/YYYY`. The strong typing is the point
-- `min_amount` is an `Option<Decimal>` and `min_date` is an
`Option<NaiveDate>` in the handler's parameter struct, so a bad value is
rejected up front with a clear message rather than silently matching
nothing or being coerced to zero.

## Errors that *don't* tell the client too much

A database error -- a connection dropped, a query that hit the statement
timeout, anything from `sqlx` -- is logged server-side with full detail
and returned to the client as an opaque 500:

```text
HTTP/1.1 500 Internal Server Error
{"error":"internal database error"}
```

*(Illustrative -- the book's server didn't fail during writing.)* This
is deliberate. `sqlx::Error`'s `Display` output includes table and
column names and, for constraint violations, the offending row's data.
None of that should be handed to an anonymous caller. Look in the server
log (`ERROR hardmoney::api::error: database error while serving
request`) for the actual cause.

Two more limits are always on: request bodies are capped at 64 KB (the
API is read-only, so this only bounds abuse), and `limit=` is clamped to
at most 500 rows per page so a client can't request an unbounded scan of
a multi-million-row `schedule_a`.

## Graceful shutdown

The server stops on `SIGINT` (Ctrl-C) or `SIGTERM` (what `systemd`,
Docker, and Kubernetes send), finishing in-flight requests before the
process exits. Every server run in this book ended with:

```text
2026-09-15T02:22:13.779617Z  INFO hardmoney::api: shutdown signal received
```

## In library code

All of these are fields on `hardmoney::api::ApiConfig`, with a builder:

```rust
use std::time::Duration;
use hardmoney::api::{ApiConfig, serve};
use hardmoney::db::{DbConfig, connect, migrate};

# async fn run(url: &str) -> Result<(), Box<dyn std::error::Error>> {
let pool = connect(&DbConfig::new(url)).await?;
migrate(&pool).await?;

let config = ApiConfig::new("0.0.0.0:8080".parse()?)
    .request_timeout(Duration::from_secs(20))
    .cors_origins(vec!["https://example.org".to_string()])
    .api_key(std::env::var("HARDMONEY_API_KEY").ok());
serve(pool, config).await?;
# Ok(())
# }
```

`max_body_bytes` is a public field if you need to change the 64 KB cap.
The CLI's `--timeout-secs` also sets `DbConfig::statement_timeout`; if
you build the pool yourself, set that too, or a query the HTTP layer has
already abandoned will keep running in Postgres.
