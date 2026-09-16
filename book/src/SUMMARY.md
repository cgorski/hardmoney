# Summary

[Introduction](./intro.md)

- [How data flows through hardmoney](./architecture.md)
- [Installation](./installation.md)
- [Quick start](./quick-start.md)

# Python

- [Getting started with Python](./python.md)
- [Python cookbook](./python-cookbook.md)
- [Python API reference](./python-api.md)
- [Using hardmoney from FECfile+ and other Python projects](./python-fecfile-plus.md)
- [For FEC staff](./for-fec-staff.md)

# Tutorials

- [Tutorials](./tutorials.md)
  - [Who is funding a candidate?](./tutorial-journalist.md)
  - [Loading a full election cycle (and keeping it fresh)](./tutorial-researcher.md)
  - [Tracking independent expenditures for or against a candidate](./tutorial-independent-expenditures.md)
  - [Importing the FEC's database dumps, from nothing](./tutorial-dumps.md)
  - [Parsing filings in Rust: from bytes to exact dollars](./tutorial-rust-library.md)
  - [What hardmoney does with messy FEC data](./tutorial-fec-data-quality.md)

# Filings

- [Parsing a filing, explained](./parsing-explained.md)
  - [Strict vs. lenient parsing](./strict-vs-lenient.md)
  - [Fidelity: what the parser does and does not change](./fidelity.md)
  - [Streaming large filings](./streaming.md)
- [Tables and typed views](./typed-views.md)
- [The schema: versions, layouts, and compile-time-checked fields](./library-schema.md)
- [Field names](./field-names.md)
- [The spec as data](./spec-as-data.md)
- [Writing `.fec` files](./writing-fec.md)
- [Exporting a filing](./exporting.md)
- [Reconciling a filing](./reconciling.md)
- [Validating a filing](./validating.md)
- [Reviewing a filing](./reviewing.md)
- [Finding filings: openFEC and the e-file feed](./discovery.md)

# Data in Postgres

- [Loading bulk data into Postgres](./bulk-etl.md)
  - [Namespaces: many sessions, one database](./namespaces.md)
  - [Reloading: replace, append, and `--if-changed`](./reloading.md)
  - [Dates: two FEC formats, twin columns](./dates.md)
  - [Amendments: which version of a report is current?](./amendments.md)
  - [The FEC's Postgres dump files](./pg-dumps.md)

# Serving

- [The REST API](./rest-api.md)
  - [Hardening the API](./api-hardening.md)
  - [The browser UI](./web-ui.md)
  - [Accessibility of the browser UI](./accessibility.md)

# Reference

- [CLI reference](./cli-reference.md)
- [Security, provenance, and deployment notes](./security-and-provenance.md)
- [Troubleshooting & FAQ](./troubleshooting.md)
