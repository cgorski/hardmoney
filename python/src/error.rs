//! Python exceptions and the mapping from the parent crate's errors.
//!
//! `hardmoney.FecError` (a `ValueError`) carries `line_no: int | None`, the
//! 1-based physical line the error refers to, so a user can find the
//! offending record in a 100 MB filing. `hardmoney.UnsupportedForm` (a
//! `FecError`) is what `Filing.reconcile()` raises for a cover form without
//! a rule table.
//!
//! I/O failures (`FecError::Io`) are deliberately *not* wrapped: a missing
//! file raises `FileNotFoundError`, exactly as `open()` would, because that
//! is what Python code expects to catch.

use pyo3::PyTypeInfo;
use pyo3::create_exception;
use pyo3::exceptions::{PyKeyError, PyValueError};
use pyo3::prelude::*;

create_exception!(
    hardmoney,
    FecError,
    PyValueError,
    "A filing could not be parsed, written, or interpreted.\n\n\
     `line_no` is the 1-based physical line the error refers to, or `None` \
     when the error is not about one line (a bad header, a missing cover \
     line)."
);

create_exception!(
    hardmoney,
    UnsupportedForm,
    FecError,
    "The cover form has no reconciliation rule table (only F3X, F3, and F3P do)."
);

/// Gives every `FecError` (and subclass) instance a `line_no` attribute,
/// `None` unless the raising code sets one -- so `err.line_no` never raises
/// `AttributeError`, even on an exception constructed from Python.
pub(crate) fn install_defaults(py: Python<'_>) -> PyResult<()> {
    FecError::type_object(py).setattr("line_no", py.None())
}

/// Builds a `hardmoney.FecError` carrying `line_no`.
pub(crate) fn fec_error(py: Python<'_>, err: &hardmoney::FecError) -> PyErr {
    match err {
        hardmoney::FecError::Io(io) => io_error(io),
        other => with_line_no(py, FecError::new_err(other.to_string()), other.line_no()),
    }
}

/// Like [`fec_error`] but `FecError::UnknownField` becomes a `KeyError`
/// naming the field, for `Line.set` / `Line[...]` semantics.
pub(crate) fn field_error(py: Python<'_>, err: &hardmoney::FecError) -> PyErr {
    match err {
        hardmoney::FecError::UnknownField { field, .. } => PyKeyError::new_err(field.clone()),
        other => fec_error(py, other),
    }
}

/// Builds a `hardmoney.UnsupportedForm`.
pub(crate) fn unsupported_form(py: Python<'_>, err: &hardmoney::parser::ReconcileError) -> PyErr {
    with_line_no(py, UnsupportedForm::new_err(err.to_string()), None)
}

fn io_error(err: &std::io::Error) -> PyErr {
    // `PyErr::from(io::Error)` picks the `OSError` subclass from the kind
    // (`FileNotFoundError`, `PermissionDenied`, ...). Rebuild rather than
    // clone: `io::Error` is not `Clone`.
    PyErr::from(std::io::Error::new(err.kind(), err.to_string()))
}

fn with_line_no(py: Python<'_>, err: PyErr, line_no: Option<u64>) -> PyErr {
    // Setting an attribute on the exception *instance* cannot reasonably
    // fail; if it does, surface that error instead of silently losing it.
    match err.value(py).setattr("line_no", line_no) {
        Ok(()) => err,
        Err(set_err) => set_err,
    }
}
