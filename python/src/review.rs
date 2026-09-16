//! `hardmoney.Review` and `hardmoney.Observation`: the RAD-style checks
//! that lead to a Request for Additional Information.

use hardmoney::parser::review::{
    Concern, Observation as RustObservation, Recipient, Review as RustReview, ReviewOptions,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;
use rust_decimal::Decimal;

/// Turns a Python recipient name (`"candidate"`, `"pac"`,
/// `"national-party"`, `"state-party"`, `"unlimited"`) into options.
pub(crate) fn options(recipient: Option<&str>) -> PyResult<ReviewOptions> {
    match recipient {
        None => Ok(ReviewOptions::new()),
        Some(name) => {
            let r: Recipient = name.trim().parse().map_err(|_| {
                PyValueError::new_err(format!(
                    "recipient must be one of 'candidate', 'pac', 'national-party', \
                     'state-party', or 'unlimited', not '{name}'"
                ))
            })?;
            Ok(ReviewOptions::new().recipient(r))
        }
    }
}

/// Every observation a RAD-style review makes of one filing, from
/// `Filing.review()`.
#[pyclass(frozen, name = "Review", module = "hardmoney")]
pub(crate) struct Review {
    inner: RustReview,
}

impl From<RustReview> for Review {
    fn from(inner: RustReview) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl Review {
    /// Every observation, in line order (report-level ones last).
    #[getter]
    fn observations(&self) -> Vec<Observation> {
        self.inner
            .observations
            .iter()
            .cloned()
            .map(Observation::from)
            .collect()
    }

    /// Counts and amounts: `{"observations": int, "amount_at_issue":
    /// Decimal, "by_concern": {name: {"count": int, "amount": Decimal}}}`.
    #[getter]
    fn summary<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let d = PyDict::new(py);
        d.set_item("observations", self.inner.summary.observations)?;
        d.set_item("amount_at_issue", self.inner.summary.amount_at_issue)?;
        let by = PyDict::new(py);
        for (concern, total) in &self.inner.summary.by_concern {
            let t = PyDict::new(py);
            t.set_item("count", total.count)?;
            t.set_item("amount", total.amount)?;
            by.set_item(concern.to_string(), t)?;
        }
        d.set_item("by_concern", by)?;
        Ok(d)
    }

    /// The concern names observed, in a fixed order.
    #[getter]
    fn concerns(&self) -> Vec<String> {
        self.inner
            .concerns()
            .into_iter()
            .map(|c| c.to_string())
            .collect()
    }

    /// The observations for one concern name, e.g.
    /// `r.by_concern("cover_not_supported")`. `ValueError` for a name that
    /// is not a concern.
    fn by_concern(&self, concern: &str) -> PyResult<Vec<Observation>> {
        let c: Concern = concern
            .trim()
            .parse()
            .map_err(|_| PyValueError::new_err(format!("'{concern}' is not a review concern")))?;
        Ok(self
            .inner
            .by_concern(c)
            .cloned()
            .map(Observation::from)
            .collect())
    }

    /// True when at least one observation has the concern.
    fn has(&self, concern: &str) -> PyResult<bool> {
        Ok(!self.by_concern(concern)?.is_empty())
    }

    /// The observations whose concern is not heuristic.
    fn strict(&self) -> Vec<Observation> {
        self.inner
            .strict()
            .cloned()
            .map(Observation::from)
            .collect()
    }

    fn __len__(&self) -> usize {
        self.inner.len()
    }

    fn __iter__(&self) -> ObservationIter {
        ObservationIter {
            items: self
                .inner
                .observations
                .iter()
                .cloned()
                .map(Observation::from)
                .collect::<Vec<_>>()
                .into_iter(),
        }
    }

    fn __bool__(&self) -> bool {
        !self.inner.is_empty()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "<hardmoney.Review {} observation(s) in {} concern(s)>",
            self.inner.len(),
            self.inner.summary.by_concern.len()
        )
    }
}

/// Iterator over a review's observations.
#[pyclass(name = "_ObservationIter", module = "hardmoney")]
pub(crate) struct ObservationIter {
    items: std::vec::IntoIter<Observation>,
}

#[pymethods]
impl ObservationIter {
    fn __iter__(slf: PyRef<'_, Self>) -> PyRef<'_, Self> {
        slf
    }

    fn __next__(&mut self) -> Option<Observation> {
        self.items.next()
    }
}

/// One thing a Reports Analysis Division analyst would ask about.
#[pyclass(
    frozen,
    name = "Observation",
    module = "hardmoney",
    skip_from_py_object
)]
#[derive(Clone)]
pub(crate) struct Observation {
    inner: RustObservation,
}

impl From<RustObservation> for Observation {
    fn from(inner: RustObservation) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl Observation {
    /// The concern's snake_case name, e.g. `"employer_occupation_missing"`.
    #[getter]
    fn concern(&self) -> String {
        self.inner.concern.to_string()
    }

    /// 1-based physical line in the `.fec` file (2 = cover), or `None`
    /// for an observation about the report as a whole.
    #[getter]
    fn line_no(&self) -> Option<u64> {
        self.inner.line_no
    }

    /// The line's transaction id, when it has one.
    #[getter]
    fn transaction_id(&self) -> Option<String> {
        self.inner.transaction_id.clone()
    }

    /// The dollar amount at issue (the contribution, the excess over a
    /// limit, the violation of a cover rule), or `None`.
    #[getter]
    fn amount(&self) -> Option<Decimal> {
        self.inner.amount
    }

    /// A complete sentence a treasurer or analyst could act on.
    #[getter]
    fn detail(&self) -> String {
        self.inner.detail.clone()
    }

    /// The openFEC `request_type` code of the RFAI letter this most
    /// resembles (`2` for a report), or `None`.
    #[getter]
    fn rfai_request_type(&self) -> Option<u8> {
        self.inner.rfai_request_type
    }

    /// True for concerns that fire on accepted filings often enough to be
    /// weighed rather than trusted.
    #[getter]
    fn heuristic(&self) -> bool {
        self.inner.concern.is_heuristic()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "<hardmoney.Observation {} line {}>",
            self.inner.concern,
            self.inner
                .line_no
                .map_or_else(|| "-".to_string(), |n| n.to_string())
        )
    }
}
