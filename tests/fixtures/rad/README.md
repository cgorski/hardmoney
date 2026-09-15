# Accepted filings whose cover page does not balance

The fixtures at the top level of `tests/fixtures/` all satisfy every
reconciliation rule, and the tests there assert it. This directory holds
real, FEC-accepted filings that do *not*: the discrepancy is the filer's,
the FEC's validator only warns about unsupported subtotals (its message
#4), and the report was accepted as filed. They are the working material
for the "For FEC staff" chapter of the book and `python/examples/fec/`.

Each file is byte-identical to `https://docquery.fec.gov/dcdev/posted/<id>.fec`.

| File | Filer | What does not balance |
|---|---|---|
| `F3XA_2011912.fec` | Republican Party of Minnesota - Federal (C00001313), M6 2026 amendment of 1986128, 1,232 body lines, spec 8.5 | Column A line 11(c) reports 2,045.00; the non-memo `SA11C` lines sum to 1,845.00. One $200 committee contribution is on the cover and not on Schedule A. Every formula that consumes 11(c) holds, and `validate` finds nothing. See `tests/fixtures/ORACLE_NOTES.md`. |
