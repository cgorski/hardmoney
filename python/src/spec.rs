//! Spec helpers: the bundled tables, per-version column layouts, and the
//! FEC's per-field specifications.

use hardmoney::Table;
use hardmoney::parser::{FecError, FieldSpec, Requirement, SpecVersion};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;
use pyo3::types::PyDict;

use crate::error::fec_error;
use crate::filing::parse_table;

fn parse_version(version: &str) -> PyResult<SpecVersion> {
    version
        .parse()
        .map_err(|e: hardmoney::parser::InvalidSpecVersion| PyValueError::new_err(e.to_string()))
}

/// Every table hardmoney knows, by FEC-style name (`"F3X"`, `"SchA"`,
/// `"TEXT"`), in name order.
#[pyfunction]
pub(crate) fn tables() -> Vec<&'static str> {
    Table::ALL.iter().map(|t| t.as_str()).collect()
}

/// The column layout of `table` at spec `version` as `(field, column)`
/// pairs in table order, `column` being 0-based. Raises `ValueError` for
/// an unknown table or a malformed version, and `FecError` when the
/// bundled data has no layout for that table at that version.
#[pyfunction]
pub(crate) fn layout(
    py: Python<'_>,
    table: &str,
    version: &str,
) -> PyResult<Vec<(&'static str, u16)>> {
    let table = parse_table(table)?;
    let version = parse_version(version)?;
    let layout = table.layout(version).ok_or_else(|| {
        fec_error(
            py,
            &FecError::NoMatchingVersionBucket {
                table,
                version,
                line_no: None,
            },
        )
    })?;
    Ok(layout.fields.iter().map(|f| (f.name, f.column)).collect())
}

/// The FEC's specification of one field of `table` at the bundled spec
/// version (`BUNDLED_SPEC_VERSION`), or `None` if the current spec does
/// not document that field. Raises `ValueError` for an unknown table.
///
/// Keys: `column`, `description`, `kind` (`alpha`, `alpha_numeric`,
/// `numeric`, `amount`, `unknown`), `max_len`, `required` (`none`,
/// `error`, `warning`, `conditional`), `condition`, `sample`,
/// `value_reference`, `rule`, `forms`, `allowed_values`, `pattern`.
#[pyfunction]
pub(crate) fn field_spec<'py>(
    py: Python<'py>,
    table: &str,
    name: &str,
) -> PyResult<Option<Bound<'py, PyDict>>> {
    let table = parse_table(table)?;
    table.spec(name).map(|s| spec_dict(py, s)).transpose()
}

fn spec_dict<'py>(py: Python<'py>, s: &FieldSpec) -> PyResult<Bound<'py, PyDict>> {
    let (required, condition) = match s.required {
        Requirement::None => ("none", None),
        Requirement::Error => ("error", None),
        Requirement::Warning => ("warning", None),
        Requirement::Conditional(c) => ("conditional", Some(c)),
        // `Requirement` is `#[non_exhaustive]`; a variant this binding
        // predates is reported by name rather than guessed at.
        _ => ("unknown", None),
    };
    let d = PyDict::new(py);
    d.set_item("column", s.column)?;
    d.set_item("description", s.description)?;
    d.set_item("kind", s.kind.to_string())?;
    d.set_item("max_len", s.max_len)?;
    d.set_item("required", required)?;
    d.set_item("condition", condition)?;
    d.set_item("sample", s.sample)?;
    d.set_item("value_reference", s.value_reference)?;
    d.set_item("rule", s.rule)?;
    d.set_item("forms", s.forms.to_vec())?;
    d.set_item("allowed_values", s.allowed_values.to_vec())?;
    d.set_item("pattern", s.pattern)?;
    Ok(d)
}
