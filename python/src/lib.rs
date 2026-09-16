//! `_hardmoney`: the extension module behind the `hardmoney` Python package.
//!
//! The Python-facing surface (names, signatures, docstrings) is described
//! in `python/hardmoney/_hardmoney.pyi`; `python/hardmoney/__init__.py`
//! re-exports everything here. The two must match exactly.
//!
//! Design notes:
//!
//! * Every class is `#[pyclass(frozen)]`. A `Line` is a handle into its
//!   `Filing` (shared `Arc<RwLock<_>>`), so `line.set(...)` is visible to
//!   `filing.to_fec()`; nothing else mutates.
//! * No `unwrap`/`expect`/indexing anywhere: every failure becomes a Python
//!   exception (`FecError` with `line_no`, `KeyError`, `ValueError`,
//!   `OSError`), never a panic.
//! * Money is `decimal.Decimal` built from the exact string form of the
//!   Rust `Decimal`; dates are `datetime.date`. No `float` anywhere.

mod error;
mod filing;
mod reconcile;
mod review;
mod spec;
mod validate;

use pyo3::prelude::*;

/// FEC electronic filings (`.fec`) in Python: parse, edit, write back,
/// validate against the FEC's acceptance rules, and reconcile a cover page
/// against its schedules -- with exact `decimal.Decimal` money.
#[pymodule(name = "_hardmoney")]
mod _hardmoney {
    use pyo3::prelude::*;
    use pyo3::types::PyModule;

    #[pymodule_export]
    use crate::error::{FecError, UnsupportedForm};
    #[pymodule_export]
    use crate::filing::{Filing, Line, fetch, parse, parse_file};
    #[pymodule_export]
    use crate::reconcile::{LineCheck, Reconciliation};
    #[pymodule_export]
    use crate::review::{Observation, Review};
    #[pymodule_export]
    use crate::spec::{field_spec, layout, tables};
    #[pymodule_export]
    use crate::validate::{Finding, Validation};

    /// The FEC spec version whose field specifications are bundled
    /// (`field_spec` describes fields at this version).
    #[pymodule_export]
    const BUNDLED_SPEC_VERSION: &str = hardmoney::parser::BUNDLED_SPEC_VERSION;

    #[pymodule_init]
    fn init(m: &Bound<'_, PyModule>) -> PyResult<()> {
        crate::error::install_defaults(m.py())?;
        m.add("__version__", env!("CARGO_PKG_VERSION"))
    }
}
