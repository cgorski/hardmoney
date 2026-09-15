# Tutorials

The chapters after this one explain hardmoney a concept at a time --
parsing, typed views, the bulk loader, the REST API. The five tutorials in
this section go the other way: each one starts with a person and a goal
and walks, command by command, to a finished result. Every command was
run while writing, against a local Postgres 18 and the live FEC servers,
and the output shown is what came back; the handful of things that could
not be reproduced on demand are marked *illustrative*.

Pick the one that matches what you're trying to do:

| If you are... | And you want... | Read |
|---|---|---|
| A reporter with no FEC background | to find a candidate, their committee, and their biggest donors, from a laptop, in about fifteen minutes | [Who Is Funding a Candidate?](./tutorial-journalist.md) |
| A researcher or data team | a full election cycle in Postgres that you can refresh weekly, archive, and compare against last week | [Loading a Full Election Cycle (and Keeping It Fresh)](./tutorial-researcher.md) |
| A watchdog or campaign staffer | every independent expenditure for or against a candidate, both from the FEC's aggregated data and straight from today's filings | [Tracking Independent Expenditures](./tutorial-independent-expenditures.md) |
| A Rust developer | a small program that fetches a filing and produces exact dollar totals, with proper error handling | [Parsing Filings in Rust](./tutorial-rust-library.md) |
| Anyone who has been burned by government data | to know exactly which FEC quirks hardmoney handles, how, and how to see each one yourself | [What hardmoney Does With Messy FEC Data](./tutorial-fec-data-quality.md) |

The tutorials are independent. Each opens by saying who it's for and what
you'll have at the end, and each links to the reference chapters rather
than repeating them.

Three jobs that arrived with 2.0 have no tutorial of their own because
each is one command: checking whether the FEC would accept a filing
(`hardmoney validate`), checking whether a report's cover page agrees
with its schedules (`hardmoney reconcile`), and writing an edited filing
back out (`hardmoney write`). Their chapters --
[Validating a Filing](./validating.md),
[Reconciling a Filing](./reconciling.md), and
[Writing `.fec` Files](./writing-fec.md) -- are written in the same
style, with real filings and real output, and
[Streaming Large Filings](./streaming.md) covers the 135 MB case the
Rust tutorial's eager parser would not fit in memory for.

## Conventions used in every tutorial

- **`hardmoney`** in a command means the built binary. After
  [Installation](./installation.md), that's `target/release/hardmoney`
  (or `target/debug/hardmoney` for a debug build); either put it on your
  `PATH` or type the path. The tutorials were run with the debug build --
  the output is identical, the release build is faster.
- **`DATABASE_URL`** is set once per tutorial with `export`. It needs a
  username in it (`postgres://you@localhost:5432/yourdb`). See
  [Installation](./installation.md#what-you-need-for-each-part-of-the-crate)
  if you don't have a Postgres database yet.
- **`--schema tut_...`** puts each tutorial's data in its own
  [namespace](./namespaces.md), so you can throw it away afterwards with
  `hardmoney schema-drop tut_... --yes` without touching anything else in
  the database. You can pick any name that matches `[a-z_][a-z0-9_]*`.
- **Real people's names** appear in the output, because FEC filings are
  public records and that is what the data contains.
