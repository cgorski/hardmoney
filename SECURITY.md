# Security policy

This file covers the `hardmoney` crate (crates.io), the `hardmoney` Python
package (PyPI), and the `hardmoney` command-line tool built from this
repository. A longer note for security reviewers, including how to verify
a release's provenance, is the book chapter
[Security, provenance, and deployment notes](https://cgorski.github.io/hardmoney/security-and-provenance.html)
(`book/src/security-and-provenance.md`).

## Supported versions

| Version | Supported |
|---|---|
| latest 3.x release | yes: security fixes ship as a patch release of the current 3.x |
| older 3.x | no: upgrade to the latest 3.x |
| 2.x and earlier | no |

Within 3.x every release is backwards compatible (semantic versioning), so
upgrading to the latest patch is the fix path.

## Reporting a vulnerability

Do not open a public issue for a security problem.

Email **cgorski@cgorski.org** with the subject `hardmoney security`. Include
the version, how it was installed, what the problem is, and, if you have
one, a filing or input that demonstrates it (a `.fec` file that makes the
parser misbehave is the most likely shape of a report). If GitHub's
private vulnerability reporting is enabled for this repository, the
"Report a vulnerability" button on the
[Security tab](https://github.com/cgorski/hardmoney/security) is an
equivalent channel.

Commitment:

- Acknowledgement within **5 business days**.
- A fix, or a written mitigation plan with a date, within **30 days** of
  the report. Fixes are released as a patch of the current 3.x, recorded
  in `CHANGELOG.md`, and, where one applies, filed with the RustSec
  advisory database so `cargo audit` users are told.
- Credit in the changelog if you want it.

## What the software does on the network

The Python package contains one function that opens a network
connection: `hardmoney.fetch(filing_id)`, which downloads
`https://docquery.fec.gov/dcdev/posted/<id>.fec`. Everything else in it
(`parse`, `parse_file`, `validate`, `reconcile`, `to_fec`, the spec
queries) is a pure function over bytes you supply.

In the Rust crate, network access is confined to `Filing::fetch`, the
`fec` module (openFEC client, e-file feed, daily archives; behind the
`fetch` feature), the `parser::webcheck` module (WebCheck submission),
and the downloaders in the `bulk` module (behind `bulk`). Each contacts
only the FEC hosts in the table below and only when called; the parser,
writer, validator, reconciler, typed views, and `export` module make no
network calls. There is no telemetry, no update check, and no crash
reporting anywhere in the project.

The command-line tool contacts only the FEC, only for the commands below,
and only to do what the command says:

| Command | Host(s) | What is sent |
|---|---|---|
| `validate --oracle webcheck` | `efoservices.fec.gov` | the `.fec` file being validated (the FEC's public WebCheck upload; with `--webcheck-api-key`, its vendor SOAP service, plus the key and `--webcheck-email`) |
| `filings` | `api.open.fec.gov`, then `docquery.fec.gov` with `--fetch`/`--validate`/`--reconcile`/`--ingest` | the query filters and the openFEC API key |
| `efile watch` | `efilingapps.fec.gov` (RSS), `docquery.fec.gov` | nothing beyond the GET |
| `efile backfill` | `www.fec.gov` (`/files/bulk-downloads/electronic/`, the daily archives; the filings come from inside them) | nothing beyond the GET |
| `bulk-load`, `bulk-load-all` | `www.fec.gov` (`/files/bulk-downloads/`) | nothing beyond the GET |
| `bulk-load-filing <numeric id>` | `docquery.fec.gov` | nothing beyond the GET |
| `bulk-restore-dump`, `bulk-dump-info`, `dumps` (overview, `check`, `import`, `status`, `update`) | `www.fec.gov` (`/files/bulk-downloads/data-dump/`) | nothing beyond the GET/HEAD |

`parse`, `write`, `export`, `reconcile`, `validate` (without `--oracle`),
`spec`, `schema-*`, `bulk-dump-index`, `bulk-dump-compare`,
`bulk-resolve-chains`, `query`, and `serve` make no outbound connection.
`serve` listens on the address you give it and never calls out; the
embedded browser UI loads no third-party scripts, fonts, or analytics.

Filing data you parse is never sent anywhere. The only filing bytes that
leave the machine are the one file you pass to `validate --oracle
webcheck`, and they go to the FEC's own validator.

TLS is rustls with the Mozilla root bundle compiled in (`webpki-roots`);
the operating system's trust store is not consulted, so a TLS-inspecting
proxy with a private root will fail certificate verification rather than
be silently trusted. `HTTPS_PROXY` / `ALL_PROXY` are honoured. Requests
carry the user agent `hardmoney/<version>`.

## Secrets

| Secret | Read from | Handling |
|---|---|---|
| openFEC API key | `FEC_API_KEY`, else `~/fec_api_key.txt` | sent only to `api.open.fec.gov`; appears in no `Debug` output, error message, or log: URLs are recorded as `api_key=REDACTED` |
| WebCheck vendor key | `--webcheck-api-key` / `WEBCHECK_API_KEY` | sent only to `efoservices.fec.gov`; optional (the default channel needs no key) |
| `DATABASE_URL` | flag, environment, or a `.env` file in the current directory | used only to connect to your Postgres; never sent to the FEC |
| REST API key (`serve --api-key`) | flag | checked against `X-Api-Key` / `?api_key=`; the request log records `api_key=REDACTED` |

Downloaded filings and archives are cached under `$XDG_CACHE_HOME/hardmoney`
or `~/.cache/hardmoney` (override with `--cache-dir` /
`HARDMONEY_CACHE_DIR`). Nothing is written elsewhere unless a command's
argument names an output path.

## Dependencies

- `deny.toml` (`cargo deny check`) enforces a permissive-licence
  allow-list, denies unknown registries and git dependencies, bans
  OpenSSL, and fails on any RustSec advisory. It runs on every push and
  pull request and weekly (`.github/workflows/supply-chain.yml`), for
  both `Cargo.lock` and `python/Cargo.lock`.
- `cargo audit --deny warnings` runs in the same workflow.
- Dependabot (`.github/dependabot.yml`) proposes weekly updates for Cargo
  (both lockfiles), pip (`python/pyproject.toml`), and the pinned GitHub
  Actions.
- A CycloneDX SBOM for the crate, the Python extension crate, and the
  built wheel is produced by the same workflow and uploaded as the `sbom`
  artifact of each run.
- The Python wheel has no Python dependencies.
- The crate contains no `unsafe` code. `CONTRIBUTING.md` rule 1 bars
  panicking calls (`unwrap`, `expect`, indexing) from library code:
  malformed input yields an `Err` naming the line, and a panic on any
  input is a bug. The parser is exercised with property-based adversarial
  inputs (`tests/parser_adversarial.rs`) and the real filings in
  `tests/fixtures/` (spec 3.00 through 8.5, 2001 to the present).

## Releases

Wheels and the sdist are built by GitHub Actions and carry build
provenance attestations (`gh attestation verify <file> --repo
cgorski/hardmoney`). Publishing to crates.io is a manual, human-approved
workflow. Details, including what is and is not reproducible, are in the
book chapter linked above.
