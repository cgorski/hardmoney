# Security, provenance, and deployment notes for reviewers

This page is for the person who has to decide whether hardmoney may be
installed on a government or corporate machine. It states what the
software does and does not do, where every byte it ships comes from, how
the dependencies and the release artifacts are checked, and who answers
when something is wrong. It should take about ten minutes. The short
form is [`SECURITY.md`](https://github.com/cgorski/hardmoney/blob/main/SECURITY.md)
in the repository root.

## What the software is

hardmoney is a library for FEC electronic filings (`.fec` files) and the
FEC's bulk data. It exists in three forms built from one source tree:

| Form | Where | What it contains |
|---|---|---|
| Rust crate `hardmoney` | crates.io | parser, writer, validator, reconciler, spec tables; optionally (Cargo features) the openFEC and e-file clients, the Postgres ETL, the REST API, and the CLI |
| Python package `hardmoney` | PyPI | the parser, writer, validator, reconciler, spec tables, and `fetch`, compiled into one `abi3` extension module; no Python dependencies |
| `hardmoney` command-line tool | `cargo install hardmoney` | all of the above plus the database and API commands |

The Python package is a pure function library: bytes in, objects out. It
does not run a server, does not write files unless you call
`open(...).write(filing.to_fec())` yourself, and has one network function
(`fetch`). The CLI's database and server commands are optional and need a
Postgres you provide.

## Network behaviour

The Python package contacts one host, and only when you call the one
function that does:

| Call | Host | When | Override |
|---|---|---|---|
| `hardmoney.fetch(id)` | `docquery.fec.gov` | when called | `HARDMONEY_DOCQUERY_BASE` |
| everything else (`parse`, `parse_file`, `validate`, `reconcile`, `to_fec`, spec queries) | none | never | |

In the Rust crate, network access is confined to `Filing::fetch`, the
`fec` module (openFEC client, e-file feed, daily archives; behind the
`fetch` feature), `parser::webcheck` (WebCheck submission), and the
downloaders in `bulk` (behind the `bulk` feature). The parser, writer,
validator, reconciler, typed views, and `export` module make no network
calls. The CLI commands below are the callers of those modules, so the
host list is the same.

The CLI contacts only FEC hosts, only for the commands that exist to talk
to the FEC. Every host is a default that a flag or an environment
variable replaces (the "Override" column; see [Pointing hardmoney at a
mirror or proxy](#pointing-hardmoney-at-a-mirror-or-proxy)):

| Command | Hosts | When | What is sent | Override |
|---|---|---|---|---|
| `validate --oracle webcheck` | `efoservices.fec.gov` | only with `--oracle` | the `.fec` file under test; with `--webcheck-api-key`, also the key and `--webcheck-email` | `--webcheck-endpoint` / `HARDMONEY_WEBCHECK_ENDPOINT` |
| `filings` | `api.open.fec.gov`; `docquery.fec.gov` with `--fetch`, `--validate`, `--reconcile`, or `--ingest` | every run | the query filters; the openFEC API key as `?api_key=` | `--openfec-base` / `HARDMONEY_OPENFEC_BASE`; `--docquery-base` / `HARDMONEY_DOCQUERY_BASE` |
| `lag` | `api.open.fec.gov`; `docquery.fec.gov` with `--counts` | every run | the filing ids or committee filter; the openFEC API key | `--openfec-base`; `--docquery-base` |
| `efile watch` | `efilingapps.fec.gov`, `docquery.fec.gov` | every poll | GET only | `--efile-rss-url` / `HARDMONEY_EFILE_RSS_URL` (or `HARDMONEY_EFILINGAPPS_BASE`); `--docquery-base` |
| `efile backfill` | `www.fec.gov` (the daily e-file archives; the filings come from inside them) | every run | GET only | `--fec-www-base` / `HARDMONEY_FEC_WWW_BASE` |
| `bulk-load`, `bulk-load-all` | `www.fec.gov` | every run (unless the file is cached and `--if-changed` finds it unchanged) | GET / HEAD only | `--fec-www-base` |
| `bulk-load-filing <numeric id>` | `docquery.fec.gov` | when given an id rather than a path | GET only | `--docquery-base` |
| `bulk-restore-dump`, `bulk-dump-info`, `dumps`, `dumps check`, `dumps import`, `dumps status`, `dumps update` | `www.fec.gov` | every run, except `dumps check` and `dumps status` with `--offline` | GET / HEAD only | `--fec-www-base` (the bare `dumps` overview reads only the variable) |
| `serve` | none by itself; `docquery.fec.gov` for `GET /tools/fetch/{id}` (with `--ui`) and `api.open.fec.gov` for `GET /filings/{id}/processing`, when a client asks | per request | GET only; the openFEC API key on `/processing` | `--docquery-base` (also sets the `fec_url` in `/filings` responses); `--openfec-base` |

Commands not in the table (`parse`, `write`, `export`, `reconcile`,
`validate` without `--oracle`, `spec`, `schema-init`, `schema-status`,
`schema-list`, `schema-drop`, `bulk-dump-index`, `bulk-dump-compare`,
`bulk-resolve-chains`, `dumps remove`, `query`) open no outbound
connection. `serve` listens where you tell it and, apart from the two
routes above, calls nothing; its embedded browser UI ships its own
assets and loads no third-party scripts, fonts, or analytics.

There is no telemetry, no update check, and no crash reporting. The
only filing bytes that ever leave the machine are the single file passed
to `validate --oracle webcheck`, which go to the FEC's own validator.

Transport details a network team may want:

- HTTPS via [rustls](https://github.com/rustls/rustls); no OpenSSL is
  linked (this is enforced by `deny.toml`).
- Trust anchors are the Mozilla root bundle compiled in
  (`webpki-roots`). The operating system trust store is not consulted,
  so a TLS-inspecting proxy with a private root causes a certificate
  error rather than being silently trusted. If your network requires
  such a proxy, point hardmoney at an internal mirror over plain
  `http://` or a host with a public certificate (next section), run the
  offline commands only, or fetch files with your own tooling and pass
  paths.
- Proxy variables are read by the HTTP client (`ureq` 3): the first of
  `ALL_PROXY`, `all_proxy`, `HTTPS_PROXY`, `https_proxy`, `HTTP_PROXY`,
  `http_proxy` that holds a parseable proxy URI is used for every request
  (there is no per-scheme selection, so `ALL_PROXY` takes precedence over
  `HTTPS_PROXY` when both are set), and `NO_PROXY` / `no_proxy` lists
  hosts that bypass it (exact names, `*.suffix`, `.suffix`, or `*`).
  Only `http://` and `https://` proxy URIs are supported; hardmoney
  does not enable the client's SOCKS feature. A proxy URI may carry
  `user:password@`.
- User agent: `hardmoney/<version>`. The FEC clients use a 30 s connect
  timeout and a 60 s response timeout.

### Pointing hardmoney at a mirror or proxy

Every FEC address hardmoney requests is built from one of six bases, each
with the production address as its default. The Rust type is
`hardmoney::fec::Endpoints`; on the CLI each base is a flag on every
command that uses it, and each flag reads an environment variable when
absent, so a deployment sets the variables once (in the environment or
a `.env` file next to the command) and changes nothing else:

| Variable | Flag | Default | Used for |
|---|---|---|---|
| `HARDMONEY_FEC_WWW_BASE` | `--fec-www-base` | `https://www.fec.gov` | bulk zips (`/files/bulk-downloads/<cycle>/`), `pg_dump` archives (`/files/bulk-downloads/data-dump/schedules/`), daily e-file zips (`/files/bulk-downloads/electronic/`), data dictionaries |
| `HARDMONEY_OPENFEC_BASE` | `--openfec-base` | `https://api.open.fec.gov/v1/` | the openFEC API |
| `HARDMONEY_DOCQUERY_BASE` | `--docquery-base` | `https://docquery.fec.gov` | raw filings (`/dcdev/posted/<id>.fec`); also the `fec_url` the REST API reports and the target of `hardmoney.fetch` in Python |
| `HARDMONEY_EFILINGAPPS_BASE` | (variable only) | `https://efilingapps.fec.gov` | the e-filing applications host; the RSS feed is `/rss/generate?preDefinedFilingType=ALL` under it |
| `HARDMONEY_EFILE_RSS_URL` | `--efile-rss-url` | `https://efilingapps.fec.gov/rss/generate?preDefinedFilingType=ALL` | the e-file RSS feed as a complete URL; wins over the derived one |
| `HARDMONEY_WEBCHECK_ENDPOINT` | `--webcheck-endpoint` | `https://efoservices.fec.gov/webcheck` | the WebCheck validator (`/services/upload`, `/services/validate`) |

A mirror that serves the FEC's files under the same paths needs one
variable per host it replaces:

```bash
export HARDMONEY_FEC_WWW_BASE=https://mirror.example.gov
export HARDMONEY_DOCQUERY_BASE=https://mirror.example.gov/docquery
hardmoney dumps import all-small          # downloads from mirror.example.gov
hardmoney efile watch --once --validate   # feed items are fetched from the mirror
```

The paths under each base are fixed (a mirror is expected to preserve
the FEC's layout), and a trailing slash on a base is optional. Links the
FEC hands out that name `docquery.fec.gov` (in the RSS feed and in
openFEC's `fec_url`) are rewritten onto `HARDMONEY_DOCQUERY_BASE`
before the download, so feed- and API-driven fetches follow the mirror
too. The `fec_url` in `hardmoney serve`'s `/filings` responses is built
from the same base, as a bound SQL parameter.

A value that is not an `http://` or `https://` URL with a host (or, for
the five bases, that carries a query string) is refused before any
request is made, with a message naming the variable and the flag:

```text
error: --docquery-base / HARDMONEY_DOCQUERY_BASE: "mirror.example.gov" is not a usable URL: it must start with http:// or https://
```

An empty variable counts as unset. Nothing falls back to the production
host when an override is present but wrong.

For an egress proxy that does not rewrite hosts, leave the bases alone
and set `HTTPS_PROXY` (or `ALL_PROXY`) and, if needed, `NO_PROXY`, as
described above. The two mechanisms compose: a base can name a mirror
while the proxy variables route the connection.

In the Rust crate, `Endpoints::from_env()` reads the six variables and
returns the error above for a bad one; `Endpoints::default()` is
production; the `with_*` builders set a base explicitly. Constructors
named `new` (`OpenFec::new`, `EfileFeed::new`, `WebCheck::new`) do not
read the environment; `OpenFec::from_env`, `WebCheck::from_env`,
`Filing::fetch`, and the `fec::fetch_filing_bytes` family do.

## Data handling

- Input filings are read from the path or bytes you give. Field values
  come back as filed apart from trimmed ASCII whitespace, one pair of
  wrapping double quotes, and decoding from UTF-8 or Windows-1252
  ([Fidelity](./fidelity.md)).
- Nothing is written unless a command's argument names an output
  (`export --out`, `write`, `filings --fetch <dir>`), or a download is
  cached. Downloads (raw filings, daily e-file archives, bulk zips,
  dump files) are cached under `$XDG_CACHE_HOME/hardmoney`, else
  `~/.cache/hardmoney`; `--cache-dir` / `HARDMONEY_CACHE_DIR` moves it.
- Database commands write only to the Postgres named by `DATABASE_URL`,
  in the schema named by `--schema` (default `public`), plus the
  `disclosure` schema for the FEC's own dump tables
  ([Namespaces](./namespaces.md)).
- All FEC data hardmoney handles is public record. The software has no
  notion of a user account and stores no personal data of its own.

## Secrets

| Secret | Read from | Where it goes | Logging |
|---|---|---|---|
| openFEC API key | `FEC_API_KEY`, else `~/fec_api_key.txt` | `api.open.fec.gov` only | never in `Debug` output, errors, or logs; URLs are recorded as `api_key=REDACTED` |
| WebCheck vendor key and contact email | `--webcheck-api-key` / `WEBCHECK_API_KEY`, `--webcheck-email` / `WEBCHECK_EMAIL` | `efoservices.fec.gov` only; optional, the default channel needs no key | not logged |
| `DATABASE_URL` (may embed a password) | flag, environment, or a `.env` file in the current directory | your Postgres only | not printed, except by the `dumps` commands, which show it with the password replaced by `***` |
| REST API key (`serve --api-key`) | flag | compared against `X-Api-Key` / `?api_key=` on each request | request log records `api_key=REDACTED` |

No secret is ever sent to a host other than the one in the table.

## Dependencies and how they are checked

The dependency policy is in `deny.toml` and is enforced on every push,
every pull request, and weekly by `.github/workflows/supply-chain.yml`,
against both lockfiles (`Cargo.lock` for the crate and the CLI,
`python/Cargo.lock` for the Python extension):

- `cargo deny check advisories licenses bans sources`: fails on any
  RustSec advisory (vulnerability, unsound, unmaintained, or yanked
  crate); allows only Apache-2.0, MIT, BSD-2-Clause, BSD-3-Clause, ISC,
  Unicode-3.0, and Zlib, plus three crate-scoped exceptions listed
  below; denies any registry other than crates.io and any git
  dependency; bans `openssl`, `openssl-sys`, and `native-tls`; warns on
  duplicate versions of a crate.
- `cargo audit --deny warnings` against the RustSec database.
- Dependabot proposes weekly updates for Cargo (both lockfiles), pip
  (`python/pyproject.toml`), and the pinned GitHub Actions.

Licence exceptions in `deny.toml`, each for one named crate:

| Crate | Licence | Why it is acceptable |
|---|---|---|
| `tiny-keccak` | CC0-1.0 | public-domain dedication; reached through `ahash`/`const-random` in the Arrow and sqlx stacks |
| `webpki-roots` | CDLA-Permissive-2.0 | Mozilla's CA bundle, a data file; permissive, no obligation on embedding software |
| `target-lexicon` | Apache-2.0 WITH LLVM-exception | build-time dependency of `pyo3-build-config` only; nothing from it is in the wheel |

Other facts a reviewer usually asks for:

- The crate has no `unsafe` code.
- The parser is required to return an error, never panic, on any input
  ([`CONTRIBUTING.md`](https://github.com/cgorski/hardmoney/blob/main/CONTRIBUTING.md)
  rule 1). It is exercised with property-based adversarial inputs
  (`tests/parser_adversarial.rs`) and the real filings in
  `tests/fixtures/` (spec 3.00 through 8.5, 2001 to the present) on
  every CI run.
- The Python wheel declares no `Requires-Dist`; its whole dependency set
  is the Rust tree in the SBOM.
- The parser-only build (`default-features = false, features = ["fetch"]`),
  which is what the wheel compiles, excludes Tokio, sqlx, Axum, Arrow,
  and SQLite.

## Software bill of materials

The supply-chain workflow produces a CycloneDX 1.5 JSON SBOM on every
run and uploads it as the workflow artifact named `sbom`, containing:

| File | Describes |
|---|---|
| `hardmoney-crate.cdx.json` | the crate with all features, for all target platforms |
| `hardmoney-python-extension.cdx.json` | the extension crate compiled into the wheel (`python/Cargo.lock`) |
| `hardmoney-wheel-environment.cdx.json` | a fresh virtualenv with the built wheel installed (shows the empty Python dependency set) |
| `hardmoney-wheel-METADATA.txt` | the wheel's own metadata |

To fetch the SBOM for a release, find the run of the "Supply chain"
workflow on the tag and download the artifact:

```bash
gh run list --repo cgorski/hardmoney --workflow supply-chain.yml --branch v3.0.2
gh run download <run-id> --repo cgorski/hardmoney --name sbom
```

You can regenerate the crate SBOM locally from a checkout with
`cargo install cargo-cyclonedx && cargo cyclonedx --all-features --target all --format json --spec-version 1.5`.

## Build provenance

Wheels and the sdist are built by GitHub Actions
(`.github/workflows/python.yml`) on GitHub-hosted runners, one native
build per platform. On a `v*` tag the workflow signs a
[build provenance attestation](https://docs.github.com/en/actions/security-for-github-actions/using-artifact-attestations)
for each file: a Sigstore-signed SLSA provenance statement, stored by
GitHub, that binds the file's SHA-256 to the repository, the commit, the
workflow file, and the run that produced it.

To check a downloaded wheel or sdist:

```bash
gh attestation verify hardmoney-3.0.2-cp39-abi3-win_amd64.whl --repo cgorski/hardmoney
```

The command fails if the file was not produced by a workflow in this
repository or has been modified since. Add
`--signer-workflow cgorski/hardmoney/.github/workflows/python.yml` to
require this particular workflow, and `--format json` to read the
provenance statement itself. Attestations for public repositories can
also be verified offline against the public Sigstore transparency log
with the `sigstore` tooling.

The crates.io package carries no attestation; crates.io records the
publishing account and the checksum of the tarball, and `cargo` verifies
the checksum on download. Publishing is a manual, human-approved
workflow (below).

## Reproducibility

What is fixed:

- Both lockfiles (`Cargo.lock`, `python/Cargo.lock`) are committed.
  Wheels are built with `--locked`, so a wheel's dependency set is
  exactly the committed lockfile at the tag, which is what the SBOM
  records.
- Every FEC table compiled into the binary is generated by `build.rs`
  from files in `data/`, deterministically; a change to those files is a
  change to the source tree.
- `cargo install hardmoney --locked` reproduces the CLI's dependency
  set from the crate's own lockfile.

What is not fixed:

- The Rust toolchain. CI builds with the current stable compiler and
  separately checks that the crate still builds on the minimum
  supported version (1.94). There is no `rust-toolchain.toml`, so two
  builds a month apart may use different compilers.
- Byte-for-byte reproducibility of wheels. The extension is built with
  fat LTO and symbol stripping on runners whose images change; rebuilding
  from the same commit will produce a functionally identical but not
  bit-identical `.so`/`.pyd`. The attestation, not a rebuild, is the way
  to confirm a wheel's origin.

## Licences

hardmoney is `Apache-2.0 OR BSD-3-Clause` (your choice of either). The
texts are `LICENSE-APACHE` and `LICENSE-BSD`; `NOTICE` carries the
third-party attributions. Every dependency is under one of the
permissive licences allowed by `deny.toml` (above). No copyleft licence
is in the tree, and the build cannot add one without failing CI.

## Data provenance

Everything hardmoney knows about the FEC's format comes from the FEC or
from one Apache-2.0 project, and the lineage is recorded in `NOTICE`:

| Data | Source | Terms |
|---|---|---|
| `data/fec-spec/FEC_EFO_Format_Specifications_v8.5.xlsx` and its distillation `spec-8.5.json` (field types, lengths, required levels, rule text, code lists) | the FEC's Electronic Filing Specification workbook; the JSON Schemas in `fecgov/fecfile-validate` | works of the United States Government, public domain (17 U.S.C. § 105) |
| `data/fec-csv-sources/*.csv` (column positions per spec version, 3.x to 8.5) | `dwillis/fech-sources`, extracted from the New York Times's Fech gem, with the corrections listed in `NOTICE`; `F2S.csv` authored here | Apache-2.0 |
| reconcile formulas and FECfile+ expected totals used as oracle tests | `fecgov/fecfile-web-api` | public domain |
| `tests/fixtures/*.fec` | real filings from the FEC's public document store (`docquery.fec.gov`) | public record |

A weekly job (`.github/workflows/spec-drift.yml`) re-derives the spec
JSON from the FEC's current `fecfile-validate` schemas and fails if the
committed copy has drifted, so a format change by the FEC shows up as a
red build rather than as a stale validator.

## How releases are cut

1. `CHANGELOG.md` gets a dated section; `Cargo.toml`,
   `python/Cargo.toml`, and `python/pyproject.toml` get the version; the
   release checklist in `CONTRIBUTING.md` (fmt, clippy, rustdoc with
   warnings denied, tests, `cargo package`, `mdbook build`) passes.
2. The commit is tagged `vX.Y.Z` and pushed. The tag triggers the Python
   workflow: lint, tests on Linux, macOS, and Windows with Python 3.9 and
   3.13, five wheels and the sdist, the provenance attestations, and,
   if enabled, publication to PyPI.
3. crates.io: a maintainer dispatches `.github/workflows/publish.yml`
   with the version. The job verifies that `Cargo.toml` and the tag
   agree, runs the tests, does a dry run, and publishes with a token
   held in the `crates-io` GitHub environment. Nothing publishes on a
   push or tag event.

Configuration the maintainer must set before PyPI publication runs from
CI (until then the `publish` job is skipped and shows as such):

| Setting | Where | Value |
|---|---|---|
| `PYPI_PUBLISH` | repository variable (Settings, Secrets and variables, Actions, Variables) | `true` |
| `PYPI_API_TOKEN` | secret in the `pypi` GitHub environment (created automatically on first use; add a required reviewer there) | a PyPI API token scoped to the `hardmoney` project |
| `CRATES_IO` | secret in the `crates-io` environment | a crates.io token scoped to publishing `hardmoney` |

If `PYPI_PUBLISH` is `true` but the token is missing, the job fails on
its first step with a message saying so, before anything is downloaded
or uploaded. The next step after tokens is PyPI
[Trusted Publishing](https://docs.pypi.org/trusted-publishers/)
(OIDC from the `pypi` environment, no long-lived secret), which also
lets `pypa/gh-action-pypi-publish` upload PEP 740 attestations to PyPI
alongside the GitHub ones.

Release artefacts per version: the crate on crates.io, five wheels and an
sdist on PyPI (Linux x86_64 and aarch64, macOS Intel and Apple silicon,
Windows x86_64), the `sbom` artifact, and the attestations.

## Support plan

- **Contact.** [GitHub issues](https://github.com/cgorski/hardmoney/issues)
  for anything public; cgorski@cgorski.org for anything that is not,
  including security reports.
- **Response commitment.** Security reports: acknowledgement within five
  business days, a fix or written mitigation plan within thirty days
  (`SECURITY.md`). Other issues: no fixed SLA, but a report that
  includes a filing id is normally reproduced and answered within a
  week.
- **Cadence.** Semantic versioning. A minor release when the FEC
  publishes a change to the filing format; a patch release when a
  validator or reconcile disagreement is confirmed against WebCheck or
  FECfile+ and hardmoney is the one that is wrong; patch releases for
  security fixes. Only the latest 3.x is supported.
- **Reporting a disagreement with the FEC's own tools.** The
  [validator disagreement](https://github.com/cgorski/hardmoney/issues/new?template=validator-disagreement.yml)
  and
  [reconcile disagreement](https://github.com/cgorski/hardmoney/issues/new?template=reconcile-disagreement.yml)
  issue templates ask for the filing id, the `--json` output, and, if
  you ran it, the `--oracle webcheck --json` diff. See also
  [For FEC staff](./for-fec-staff.md).
- **What happens if the maintainer disappears.** The licence permits
  forking; the data files are the FEC's; the build has no private
  inputs; the whole test suite runs from a checkout with no credentials
  (network tests are opt-in via `HARDMONEY_NETWORK_TESTS`). A fork can
  release under another name with no permission needed.
