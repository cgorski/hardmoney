//! `hardmoney.Reconciliation` and `hardmoney.LineCheck`: does the cover
//! page match the schedules?

use hardmoney::parser::{
    Column, LineCheck as RustLineCheck, Reconciliation as RustReconciliation, Relation,
};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use rust_decimal::Decimal;

/// Every cover-page line check for one filing, from `Filing.reconcile()`.
///
/// `str(r)` prints one check per line followed by a one-line summary, as
/// `hardmoney reconcile` does.
#[pyclass(frozen, name = "Reconciliation", module = "hardmoney")]
pub(crate) struct Reconciliation {
    inner: RustReconciliation,
}

impl From<RustReconciliation> for Reconciliation {
    fn from(inner: RustReconciliation) -> Self {
        Self { inner }
    }
}

fn parse_column(column: &str) -> PyResult<Column> {
    match column.trim() {
        "A" | "a" => Ok(Column::A),
        "B" | "b" => Ok(Column::B),
        other => Err(PyValueError::new_err(format!(
            "column must be 'A' (this period) or 'B' (year/cycle to date), not '{other}'"
        ))),
    }
}

#[pymethods]
impl Reconciliation {
    /// The cover form the rules came from: `"F3X"`, `"F3"`, or `"F3P"`.
    #[getter]
    fn form(&self) -> &'static str {
        self.inner.form.as_str()
    }

    /// Every line check, Column A first, in rule-table order.
    #[getter]
    fn checks(&self) -> Vec<LineCheck> {
        self.inner
            .checks
            .iter()
            .cloned()
            .map(LineCheck::from)
            .collect()
    }

    /// True when every line agrees exactly with its rule.
    #[getter]
    fn balances(&self) -> bool {
        self.inner.balances()
    }

    /// The checks that do not match (`delta != 0`, or a floor that is
    /// undershot).
    fn mismatches(&self) -> Vec<LineCheck> {
        self.inner
            .mismatches()
            .cloned()
            .map(LineCheck::from)
            .collect()
    }

    /// The check for one line label in one column (`"A"` or `"B"`), e.g.
    /// `r.line("A", "11(a)(i)")`, or `None` if the form has no such rule
    /// (or this spec version lacks the line).
    fn line(&self, column: &str, line: &str) -> PyResult<Option<LineCheck>> {
        let column = parse_column(column)?;
        Ok(self.inner.line(column, line).cloned().map(LineCheck::from))
    }

    fn __len__(&self) -> usize {
        self.inner.checks.len()
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "<hardmoney.Reconciliation {} {} check(s), {} mismatch(es)>",
            self.inner.form,
            self.inner.checks.len(),
            self.inner.mismatches().count()
        )
    }
}

/// The outcome of checking one cover-page line.
#[pyclass(frozen, name = "LineCheck", module = "hardmoney")]
pub(crate) struct LineCheck {
    inner: RustLineCheck,
}

impl From<RustLineCheck> for LineCheck {
    fn from(inner: RustLineCheck) -> Self {
        Self { inner }
    }
}

#[pymethods]
impl LineCheck {
    /// The FEC's line label, e.g. `"11(a)(i)"`, `"6(c)"`, `"31"`.
    #[getter]
    fn line(&self) -> &'static str {
        self.inner.line
    }

    /// The canonical cover-page field holding the reported value.
    #[getter]
    fn field(&self) -> &'static str {
        self.inner.field
    }

    /// `"A"` (this period) or `"B"` (year/cycle to date).
    #[getter]
    fn column(&self) -> String {
        self.inner.column.to_string()
    }

    /// The rule in the FEC's notation, e.g. `"= 11ai + 11aii"`.
    #[getter]
    fn rule(&self) -> String {
        self.inner.rule.clone()
    }

    /// The value on the cover page, or `None` if blank or not a valid
    /// amount (`reported_unparseable` tells the two apart); treated as 0
    /// in `delta`.
    #[getter]
    fn reported(&self) -> Option<Decimal> {
        self.inner.reported
    }

    /// The value the schedules or formula imply.
    #[getter]
    fn expected(&self) -> Decimal {
        self.inner.expected
    }

    /// `reported - expected` (a blank `reported` counts as 0).
    #[getter]
    fn delta(&self) -> Decimal {
        self.inner.delta
    }

    /// `"equal"` (must match exactly) or `"at_least"` (the itemized sum is
    /// a floor: sub-$200 items may be reported unitemized).
    #[getter]
    fn relation(&self) -> String {
        match self.inner.relation {
            Relation::Equal => "equal".to_owned(),
            Relation::AtLeast => "at_least".to_owned(),
        }
    }

    /// True when the cover page satisfies the rule.
    #[getter]
    fn matches(&self) -> bool {
        self.inner.matches()
    }

    /// How far the rule is violated: `|delta|` for an equality, the
    /// shortfall for a floor, `0` when it matches.
    #[getter]
    fn violation(&self) -> Decimal {
        self.inner.violation()
    }

    /// For schedule sums, how many body lines contributed.
    #[getter]
    fn lines_summed(&self) -> usize {
        self.inner.lines_summed
    }

    /// True when the cover page carries a value that is not a valid FEC
    /// amount (e.g. `$5,500.00`). A blank value is `reported=None` with
    /// this `False`.
    #[getter]
    fn reported_unparseable(&self) -> bool {
        self.inner.reported_unparseable
    }

    fn __str__(&self) -> String {
        self.inner.to_string()
    }

    fn __repr__(&self) -> String {
        format!(
            "<hardmoney.LineCheck col {} line {} {}>",
            self.inner.column,
            self.inner.line,
            if self.inner.matches() { "ok" } else { "DIFF" }
        )
    }
}
