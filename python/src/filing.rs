//! `hardmoney.Filing` and `hardmoney.Line`.
//!
//! A `Filing` owns the parsed [`hardmoney::Filing`] behind an
//! `Arc<RwLock<_>>`; every `Line` handed to Python is a handle (the same
//! `Arc` plus a slot: the cover line or a body index) rather than a copy.
//! That is what makes `line.set(...)` followed by `filing.to_fec()` a
//! round trip: the edit lands in the one shared filing the writer reads.
//! Both classes are `frozen` on the Python side; all mutation goes through
//! the lock, which is held only for the duration of a Rust call and never
//! while calling back into Python.

use std::path::PathBuf;
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::NaiveDate;
use hardmoney::parser::{ParseOptions, ParsedLine, SkippedLine, parse_fec_date, parse_money};
use hardmoney::{Lenient, Table};
use pyo3::exceptions::{PyKeyError, PyRuntimeError, PyTypeError, PyValueError};
use pyo3::prelude::*;
use pyo3::types::{PyByteArray, PyBytes, PyDict, PyIterator, PyList, PyString};
use rust_decimal::Decimal;

use crate::error::{fec_error, field_error, unsupported_form};
use crate::reconcile::Reconciliation;
use crate::validate::Validation;

/// The parsed filing plus whatever a lenient parse skipped.
pub(crate) struct Inner {
    filing: hardmoney::Filing,
    skipped: Vec<SkippedLine>,
}

pub(crate) type Shared = Arc<RwLock<Inner>>;

/// Which record of the filing a `Line` refers to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Slot {
    /// The cover/summary line (row 2 of the file).
    Summary,
    /// `filing.lines[i]`.
    Body(usize),
}

/// Reads the shared filing. A poisoned lock (a panic while writing, which
/// the extension never does) is recovered rather than propagated, so a
/// read can never itself raise.
fn read(shared: &Shared) -> RwLockReadGuard<'_, Inner> {
    shared.read().unwrap_or_else(PoisonError::into_inner)
}

fn write(shared: &Shared) -> RwLockWriteGuard<'_, Inner> {
    shared.write().unwrap_or_else(PoisonError::into_inner)
}

fn options(lenient: bool) -> ParseOptions {
    if lenient {
        ParseOptions::LENIENT
    } else {
        ParseOptions::STRICT
    }
}

/// Parses a table name as Python passes it (`"SchA"`, `"scha"`, `"F3X"`).
pub(crate) fn parse_table(name: &str) -> PyResult<Table> {
    name.parse().map_err(|_| {
        PyValueError::new_err(format!(
            "unknown table '{name}'; hardmoney.tables() lists the {} known tables",
            Table::ALL.len()
        ))
    })
}

// ---------------------------------------------------------------------------
// parse / parse_file / fetch
// ---------------------------------------------------------------------------

fn finish(
    py: Python<'_>,
    result: hardmoney::Result<Lenient<hardmoney::Filing>>,
) -> PyResult<Filing> {
    let (filing, skipped) = result.map_err(|e| fec_error(py, &e))?.into_parts();
    Ok(Filing::new(filing, skipped))
}

/// Parses a filing from `bytes` (decoded as UTF-8, falling back to
/// Windows-1252) or `str`.
///
/// Strict by default: the first body line that cannot be parsed raises
/// `FecError`. With `lenient=True` such lines are recorded in
/// `Filing.skipped` instead.
#[pyfunction]
#[pyo3(signature = (data, *, lenient = false))]
pub(crate) fn parse(py: Python<'_>, data: &Bound<'_, PyAny>, lenient: bool) -> PyResult<Filing> {
    let opts = options(lenient);
    let result = if let Ok(b) = data.cast::<PyBytes>() {
        let bytes = b.as_bytes();
        py.detach(|| hardmoney::Filing::parse_bytes_with(bytes, &opts))
    } else if let Ok(s) = data.cast::<PyString>() {
        let text = s.to_cow()?;
        py.detach(|| hardmoney::Filing::parse_with(&text, &opts))
    } else if let Ok(b) = data.cast::<PyByteArray>() {
        let bytes = b.to_vec();
        py.detach(|| hardmoney::Filing::parse_bytes_with(&bytes, &opts))
    } else {
        return Err(PyTypeError::new_err(format!(
            "parse() expects bytes or str, not {}",
            data.get_type().name()?
        )));
    };
    finish(py, result)
}

/// Parses a `.fec` file from disk by streaming it, so peak memory is the
/// parsed lines alone rather than the file plus its decoded text.
///
/// A missing or unreadable file raises the matching `OSError`
/// (`FileNotFoundError`, ...); a malformed one raises `FecError`.
#[pyfunction]
#[pyo3(signature = (path, *, lenient = false))]
pub(crate) fn parse_file(py: Python<'_>, path: PathBuf, lenient: bool) -> PyResult<Filing> {
    let opts = options(lenient);
    let result = py.detach(|| hardmoney::Filing::open_with(path, opts));
    finish(py, result)
}

/// Downloads a filing from the FEC's document store
/// (`docquery.fec.gov/dcdev/posted/<filing_id>.fec`) and parses it
/// strictly. Network failures raise `FecError`.
#[pyfunction]
pub(crate) fn fetch(py: Python<'_>, filing_id: u64) -> PyResult<Filing> {
    let result = py.detach(|| {
        let bytes = hardmoney::Filing::fetch_bytes(filing_id)?;
        hardmoney::Filing::parse_bytes_with(&bytes, &ParseOptions::STRICT)
    });
    finish(py, result)
}

// ---------------------------------------------------------------------------
// Filing
// ---------------------------------------------------------------------------

/// A parsed FEC electronic filing: header, cover line, and every body line.
///
/// Obtain one with `hardmoney.parse`, `hardmoney.parse_file`, or
/// `hardmoney.fetch`.
#[pyclass(frozen, name = "Filing", module = "hardmoney")]
pub(crate) struct Filing {
    inner: Shared,
}

impl Filing {
    fn new(filing: hardmoney::Filing, skipped: Vec<SkippedLine>) -> Self {
        Self {
            inner: Arc::new(RwLock::new(Inner { filing, skipped })),
        }
    }

    fn line(&self, slot: Slot) -> Line {
        Line {
            inner: Arc::clone(&self.inner),
            slot,
        }
    }

    fn body_lines(&self, keep: impl Fn(&ParsedLine) -> bool) -> Vec<Line> {
        let guard = read(&self.inner);
        guard
            .filing
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| keep(l))
            .map(|(i, _)| self.line(Slot::Body(i)))
            .collect()
    }
}

#[pymethods]
impl Filing {
    /// The top-level form type as filed, upper-cased, e.g. `"F3XA"`.
    #[getter]
    fn form_type(&self) -> String {
        read(&self.inner).filing.raw_form_type.clone()
    }

    /// `form_type` with any amendment/new/termination designator stripped,
    /// e.g. `"F3X"`.
    #[getter]
    fn base_form_type(&self) -> String {
        read(&self.inner).filing.base_form_type.clone()
    }

    /// The FEC spec version every line was parsed with, e.g. `"8.5"`.
    #[getter]
    fn version(&self) -> String {
        read(&self.inner).filing.version.to_string()
    }

    /// True when the form type designates an amendment.
    #[getter]
    fn is_amendment(&self) -> bool {
        read(&self.inner).filing.is_amendment
    }

    /// The filing number this filing amends (from the header's
    /// `FEC-<n>` report id), or `None`.
    #[getter]
    fn amends_filing(&self) -> Option<u64> {
        read(&self.inner).filing.amends_filing
    }

    /// The `HDR` record as a dict keyed like the Rust `Header` struct:
    /// `record_type`, `ef_type`, `fec_version_raw`, `version`, `soft_name`,
    /// `soft_ver`, `report_id`, `report_number`, `comment`, and -- on spec
    /// 3.x-5.x filings only -- `name_delim`.
    #[getter]
    fn header<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let h = read(&self.inner).filing.header.clone();
        let d = PyDict::new(py);
        d.set_item("record_type", &h.record_type)?;
        d.set_item("ef_type", &h.ef_type)?;
        d.set_item("fec_version_raw", &h.fec_version_raw)?;
        d.set_item("version", h.version.to_string())?;
        d.set_item("soft_name", &h.soft_name)?;
        d.set_item("soft_ver", &h.soft_ver)?;
        if let Some(delim) = &h.name_delim {
            d.set_item("name_delim", delim)?;
        }
        d.set_item("report_id", &h.report_id)?;
        d.set_item("report_number", &h.report_number)?;
        d.set_item("comment", &h.comment)?;
        Ok(d)
    }

    /// The cover/summary line (row 2 of the file).
    #[getter]
    fn summary(&self) -> Line {
        self.line(Slot::Summary)
    }

    /// Every body line in file order. Builds a new list of handles on each
    /// access; prefer `iter_lines` or `lines_for` in a loop over a large
    /// filing.
    #[getter]
    fn lines(&self) -> Vec<Line> {
        self.body_lines(|_| true)
    }

    /// Body lines a lenient parse could not interpret, each a dict with
    /// `line_no`, `form_type`, and `reason`. Always empty after a strict
    /// parse.
    #[getter]
    fn skipped<'py>(&self, py: Python<'py>) -> PyResult<Vec<Bound<'py, PyDict>>> {
        let skipped = read(&self.inner).skipped.clone();
        skipped
            .iter()
            .map(|s| {
                let d = PyDict::new(py);
                d.set_item("line_no", s.line_no)?;
                d.set_item("form_type", &s.raw_form_type)?;
                d.set_item("reason", s.reason.to_string())?;
                Ok(d)
            })
            .collect()
    }

    /// The body lines belonging to one table, e.g. `"SchA"`. Raises
    /// `ValueError` for a table name hardmoney does not know.
    fn lines_for(&self, table: &str) -> PyResult<Vec<Line>> {
        let table = parse_table(table)?;
        Ok(self.body_lines(|l| l.table() == table))
    }

    /// Iterates body lines, optionally restricted to the given tables.
    /// (v1 materialises the selection; the interface is the streaming one.)
    #[pyo3(signature = (tables = None))]
    fn iter_lines<'py>(
        &self,
        py: Python<'py>,
        tables: Option<Vec<String>>,
    ) -> PyResult<Bound<'py, PyIterator>> {
        let lines = match tables {
            None => self.body_lines(|_| true),
            Some(names) => {
                let wanted = names
                    .iter()
                    .map(|n| parse_table(n))
                    .collect::<PyResult<Vec<Table>>>()?;
                self.body_lines(|l| wanted.contains(&l.table()))
            }
        };
        PyList::new(py, lines)?.try_iter()
    }

    /// The filing in canonical `.fec` form: Windows-1252 when every
    /// character is representable (the FEC's character set), else UTF-8.
    fn to_fec<'py>(&self, py: Python<'py>) -> Bound<'py, PyBytes> {
        PyBytes::new(py, &read(&self.inner).filing.to_fec())
    }

    /// The filing in canonical `.fec` form as text (CRLF line endings, full
    /// record width, one wrapping quote pair and padding removed).
    fn to_fec_string(&self) -> String {
        read(&self.inner).filing.to_fec_string()
    }

    /// Checks the filing against the FEC's acceptance rules. Never raises:
    /// a filing that parsed can always be validated. Lines skipped by a
    /// lenient parse appear as `unrecognized_form_type` warnings.
    fn validate(&self) -> Validation {
        let guard = read(&self.inner);
        let mut v = guard.filing.validate();
        v.note_skipped(&guard.skipped);
        Validation::from(v)
    }

    /// Recomputes every cover-page line from the schedules and other cover
    /// lines. Raises `UnsupportedForm` unless the cover is F3X, F3, or F3P.
    fn reconcile(&self, py: Python<'_>) -> PyResult<Reconciliation> {
        read(&self.inner)
            .filing
            .reconcile()
            .map(Reconciliation::from)
            .map_err(|e| unsupported_form(py, &e))
    }

    fn __repr__(&self) -> String {
        let g = read(&self.inner);
        format!(
            "<hardmoney.Filing {} v{} ({} body lines)>",
            g.filing.raw_form_type,
            g.filing.version,
            g.filing.lines.len()
        )
    }
}

// ---------------------------------------------------------------------------
// Line
// ---------------------------------------------------------------------------

/// One record of a filing: the cover line or a schedule / sub-form / TEXT
/// line. Behaves like a read-mostly mapping from canonical field name to
/// the value as filed (trimmed, otherwise verbatim).
#[pyclass(frozen, name = "Line", module = "hardmoney")]
pub(crate) struct Line {
    inner: Shared,
    slot: Slot,
}

impl Line {
    fn with<R>(&self, f: impl FnOnce(&ParsedLine) -> R) -> PyResult<R> {
        let guard = read(&self.inner);
        let line = match self.slot {
            Slot::Summary => &guard.filing.summary,
            Slot::Body(i) => guard.filing.lines.get(i).ok_or_else(|| {
                PyRuntimeError::new_err(format!("body line {i} no longer exists"))
            })?,
        };
        Ok(f(line))
    }

    fn with_mut<R>(&self, f: impl FnOnce(&mut ParsedLine) -> R) -> PyResult<R> {
        let mut guard = write(&self.inner);
        let line = match self.slot {
            Slot::Summary => &mut guard.filing.summary,
            Slot::Body(i) => guard.filing.lines.get_mut(i).ok_or_else(|| {
                PyRuntimeError::new_err(format!("body line {i} no longer exists"))
            })?,
        };
        Ok(f(line))
    }

    /// The field's value, or `KeyError` if the layout has no such field.
    fn value(&self, name: &str) -> PyResult<String> {
        self.with(|l| l.get(name).map(str::to_owned))?
            .ok_or_else(|| PyKeyError::new_err(name.to_owned()))
    }
}

#[pymethods]
impl Line {
    /// The format table this line was parsed with, e.g. `"SchA"`.
    #[getter]
    fn table(&self) -> PyResult<&'static str> {
        self.with(|l| l.table().as_str())
    }

    /// The form-type token from column 0, upper-cased, e.g. `"SA11AI"`.
    /// The `form_type` *field* keeps the token exactly as filed.
    #[getter]
    fn form_type(&self) -> PyResult<String> {
        self.with(|l| l.raw_form_type.clone())
    }

    /// 1-based physical line number in the source file (0 if synthetic).
    #[getter]
    fn line_no(&self) -> PyResult<u64> {
        self.with(|l| l.line_no)
    }

    /// True when `memo_code` is `X` (case-insensitive). Memo entries are
    /// excluded from every cover-page total.
    #[getter]
    fn is_memo(&self) -> PyResult<bool> {
        self.with(ParsedLine::is_memo)
    }

    /// `line[name]`: the value as filed (`""` when blank), or `KeyError`
    /// if the field does not exist in this filing's layout for the table.
    fn __getitem__(&self, name: &str) -> PyResult<String> {
        self.value(name)
    }

    /// `name in line`: whether the layout has the field.
    fn __contains__(&self, name: &str) -> PyResult<bool> {
        self.with(|l| l.get(name).is_some())
    }

    /// The value, or `default` if the field does not exist in the layout.
    /// A blank field is `""`, not the default.
    #[pyo3(signature = (name, default = None))]
    fn get<'py>(
        &self,
        py: Python<'py>,
        name: &str,
        default: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        match self.with(|l| l.get(name).map(str::to_owned))? {
            Some(v) => Ok(PyString::new(py, &v).into_any()),
            None => Ok(default.unwrap_or_else(|| py.None().into_bound(py))),
        }
    }

    /// Field names in layout (table) order.
    fn keys(&self) -> PyResult<Vec<&'static str>> {
        self.with(|l| l.field_names().collect())
    }

    /// `(name, value)` pairs in layout order, blanks included.
    fn items(&self) -> PyResult<Vec<(&'static str, String)>> {
        self.with(|l| l.iter().map(|(k, v)| (k, v.to_owned())).collect())
    }

    /// Every field as a dict in layout order, blanks included.
    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyDict>> {
        let pairs = self.items()?;
        let d = PyDict::new(py);
        for (k, v) in pairs {
            d.set_item(k, v)?;
        }
        Ok(d)
    }

    /// Sets a field (trimmed like the parser). Raises `KeyError` if the
    /// layout has no such field. Setting `form_type` updates
    /// `Line.form_type` too. The change is visible to `Filing.to_fec()`.
    fn set(&self, py: Python<'_>, name: &str, value: &str) -> PyResult<()> {
        self.with_mut(|l| l.set(name, value))?
            .map_err(|e| field_error(py, &e))
    }

    /// The field parsed as an exact dollar amount (`decimal.Decimal`, scale
    /// 2), or `None` when blank or not a valid FEC amount. `KeyError` if
    /// the field is not in the layout.
    fn amount(&self, name: &str) -> PyResult<Option<Decimal>> {
        Ok(parse_money(&self.value(name)?))
    }

    /// The field parsed as a `YYYYMMDD` date (`datetime.date`), or `None`
    /// when blank, zero-filled, or not a real date. `KeyError` if the
    /// field is not in the layout.
    fn date(&self, name: &str) -> PyResult<Option<NaiveDate>> {
        Ok(parse_fec_date(&self.value(name)?))
    }

    fn __repr__(&self) -> PyResult<String> {
        self.with(|l| {
            format!(
                "<hardmoney.Line {} ({}) line {}>",
                l.raw_form_type,
                l.table(),
                l.line_no
            )
        })
    }
}
