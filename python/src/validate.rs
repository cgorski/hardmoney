//! `hardmoney.Validation` and `hardmoney.Finding`: the FEC's acceptance
//! rules, as `Filing.validate()` reports them.

use hardmoney::parser::{Finding as RustFinding, Severity, Validation as RustValidation};
use pyo3::prelude::*;
use pyo3::types::{PyIterator, PyList};

/// The result of `Filing.validate()`: every finding in file order.
///
/// `len(v)` is the number of findings, `str(v)` prints one finding per
/// line in the WebCheck style the CLI uses, and iterating yields
/// `Finding`s.
#[pyclass(frozen, name = "Validation", module = "hardmoney")]
pub(crate) struct Validation {
    inner: RustValidation,
}

impl From<RustValidation> for Validation {
    fn from(inner: RustValidation) -> Self {
        Self { inner }
    }
}

impl Validation {
    fn collect(&self, severity: Option<Severity>) -> Vec<Finding> {
        self.inner
            .iter()
            .filter(|f| severity.is_none_or(|s| f.severity == s))
            .cloned()
            .map(Finding::from)
            .collect()
    }
}

#[pymethods]
impl Validation {
    /// Every finding, in line order.
    #[getter]
    fn findings(&self) -> Vec<Finding> {
        self.collect(None)
    }

    /// The error-severity findings: the FEC would reject the filing.
    #[getter]
    fn errors(&self) -> Vec<Finding> {
        self.collect(Some(Severity::Error))
    }

    /// The warning-severity findings: reported, but the filing is accepted.
    #[getter]
    fn warnings(&self) -> Vec<Finding> {
        self.collect(Some(Severity::Warning))
    }

    /// True when there are no error-severity findings.
    #[getter]
    fn is_acceptable(&self) -> bool {
        self.inner.is_acceptable()
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __iter__<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyIterator>> {
        PyList::new(py, self.findings())?.try_iter()
    }

    /// One finding per line: `ERROR line 12 SA11AI contributor_last_name: ...`.
    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "<hardmoney.Validation {} error(s), {} warning(s)>",
            self.inner.error_count(),
            self.inner.warning_count()
        )
    }
}

/// One validation message, tied to a line (and usually a field).
#[pyclass(frozen, name = "Finding", module = "hardmoney")]
pub(crate) struct Finding {
    inner: RustFinding,
}

impl From<RustFinding> for Finding {
    fn from(inner: RustFinding) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl Finding {
    /// `"error"` (the FEC rejects the filing) or `"warning"`.
    #[getter]
    fn severity(&self) -> String {
        self.inner.severity.to_string()
    }

    /// The rule's stable snake_case name, e.g. `"required_field_empty"`.
    #[getter]
    fn rule(&self) -> &'static str {
        self.inner.rule.into()
    }

    /// 1-based physical line in the `.fec` file (1 = `HDR`, 2 = cover).
    #[getter]
    fn line_no(&self) -> u64 {
        self.inner.line_no
    }

    /// The record's form-type token, upper-cased (`"HDR"`, `"SA11AI"`).
    #[getter]
    fn form_type(&self) -> String {
        self.inner.form_type.clone()
    }

    /// The canonical field name when the finding is about one field.
    #[getter]
    fn field(&self) -> Option<&'static str> {
        self.inner.field
    }

    /// A complete sentence a filer could act on, worded after the FEC's.
    #[getter]
    fn message(&self) -> String {
        self.inner.message.clone()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "<hardmoney.Finding {} {} line {}{}>",
            self.inner.severity,
            self.inner.rule,
            self.inner.line_no,
            self.inner
                .field
                .map(|f| format!(" {f}"))
                .unwrap_or_default()
        )
    }
}
