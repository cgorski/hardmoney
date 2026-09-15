# A chain of one committee's consecutive reports

`ReportChain` checks what a single filing cannot: Column B (the calendar
year to date on Form 3X) against the schedules of every report in the
year, and cash on hand carried from one report's close to the next one's
beginning. These fixtures are the Republican Party of Minnesota - Federal
(C00001313) at the turn of 2025 into 2026, fetched on 2026-09-15 with

```text
hardmoney filings --committee C00001313 --form-type F3X --cycle 2026 --most-recent --fetch tmp/review/chain/
```

and renamed `<form type as filed>_<filing id>.fec`. Each file is
byte-identical to `https://docquery.fec.gov/dcdev/posted/<id>.fec`
(sha256 below).

| File | Report | Coverage | Version | Notes |
|---|---|---|---|---|
| `F3XN_1943038.fec` | YE 2025 (semi-annual) | 2025-07-01..2025-12-31 | 8.5 | The last report of 2025. Its line 8 (cash on hand at close, 97,188.87) is the 2026 reports' line 6(a) and the January report's 6(b). 3,367 body lines. |
| `F3XN_1948502.fec` | M2 2026 (January) | 2026-01-01..2026-01-31 | 8.5 | First report of the year; its Column B equals its Column A. 363 body lines. |
| `F3XA_2011895.fec` | M3 2026 (February), amendment 1 of 1953805 | 2026-02-01..2026-02-28 | 8.5 | Filed 2026-09-14; `most_recent` per openFEC. 399 body lines. |
| `F3XA_2011898.fec` | M4 2026 (March), amendment 1 of 1970202 | 2026-03-01..2026-03-31 | 8.5 | Filed 2026-09-14; `most_recent` per openFEC. 436 body lines. The chain's current report in `tests/reconcile_fixtures.rs`. |
| `F3XA_2011901.fec` | M5 2026 (April), amendment 1 of 1976565 | 2026-04-01..2026-04-30 | 8.5 | Filed 2026-09-14; `most_recent` per openFEC. 765 body lines. Fills the gap between March and the May report in `tests/fixtures/rad/`. |

All five balance on every Column A rule. With January and February
behind it, the March report's 27 Column B schedule lines equal the
three-report sums exactly, and every cash-on-hand carry-forward holds,
including 6(a) against the 2025 year-end close. The same committee's
May 2026 amendment (`tests/fixtures/rad/F3XA_2011912.fec`) carries a
$200 Column A discrepancy on line 11(c); chained behind January through
April, its Column B line 11(c) is over by the same $200 and its other 26
Column B lines agree. See `tests/fixtures/ORACLE_NOTES.md`.

```text
05e7308515d2c0fc079020279ace00fd38ccd210413c8b3cf998dd705b0c147b  F3XN_1943038.fec
dbde63bfd8dde19b14e25b94ec065a63bb3ecc0c3f07b0636802f0a19ce1bdaf  F3XN_1948502.fec
8e1702f4af0a7cf54f6bfbadf4120f246a1bb698c078610ee64bab77d74863e6  F3XA_2011895.fec
79a09b83648ee33b315da22852eeebee2957c5a3eb39df480636540b0cd1de10  F3XA_2011898.fec
ec3875538da49ced42c5dcaa2faa890599127f6670c217bd03e9561e1e1eb1ab  F3XA_2011901.fec
```
