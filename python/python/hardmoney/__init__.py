"""FEC electronic filings (``.fec``) in Python.

A thin wrapper over the Rust crate `hardmoney <https://crates.io/crates/hardmoney>`_:
parse a filing (spec 3.x through 8.5), read any field by its canonical
name, edit and write it back, validate it against the FEC's acceptance
rules, reconcile a cover page against its schedules, and review it the way
the FEC's Reports Analysis Division does -- with money as exact
:class:`decimal.Decimal` values, never floats.

::

    import hardmoney

    filing = hardmoney.parse_file("F3XN_2011831.fec")
    for line in filing.lines_for("SchA"):
        print(line["contributor_last_name"], line.amount("contribution_amount"))

    assert filing.validate().is_acceptable
    assert filing.reconcile().balances
"""

from hardmoney._hardmoney import (
    BUNDLED_SPEC_VERSION,
    FecError,
    Filing,
    Finding,
    Line,
    LineCheck,
    Observation,
    Reconciliation,
    Review,
    UnsupportedForm,
    Validation,
    __version__,
    fetch,
    field_spec,
    layout,
    parse,
    parse_file,
    tables,
)

__all__ = [
    "BUNDLED_SPEC_VERSION",
    "FecError",
    "Filing",
    "Finding",
    "Line",
    "LineCheck",
    "Observation",
    "Reconciliation",
    "Review",
    "UnsupportedForm",
    "Validation",
    "__version__",
    "fetch",
    "field_spec",
    "layout",
    "parse",
    "parse_file",
    "tables",
]
