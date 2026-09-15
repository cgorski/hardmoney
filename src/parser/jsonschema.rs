//! JSON Schema (draft 2020-12) for one record of one FEC table.
//!
//! The FEC publishes its electronic-filing format as an Excel workbook.
//! The JSON Schemas in `fecgov/fecfile-validate` cover spec 8.5 only and
//! only the tables FECfile+ writes; the FEC's own spreadsheet-to-schema
//! checker is unmaintained (fecfile-validate#302). `build.rs` compiles the
//! workbook and the FEC schemas into [`Table`], [`Layout`], and
//! [`FieldSpec`]; this module renders that compiled data as a JSON Schema
//! per table at any version that has a layout, so a filing vendor or the
//! FECfile+ team can validate a record with off-the-shelf tooling.
//! `hardmoney spec export --format json-schema` is the command-line face
//! of [`table_schema`] and [`bundle`].
//!
//! # What a schema says
//!
//! A record is an object keyed by canonical field name (the names in
//! [`Layout::fields`]), with every value a string exactly as filed
//! (trimmed); a blank field is the empty string. Per field:
//!
//! * `type: "string"` always (wire values are strings, dates and amounts
//!   included).
//! * `maxLength` from [`FieldSpec::max_len`]. For an `AMT-n` field the FEC
//!   bounds the *digits*: it accepted a twelve-digit total written in
//!   thirteen characters (`2280311229.59`, FEC-1458871), so `maxLength` is
//!   `n + 2` (one sign, one decimal point) and `x-fec.max_len` keeps `n`.
//! * `enum` from [`FieldSpec::allowed_values`] and `pattern` from
//!   [`FieldSpec::pattern`], only at the version the workbook describes
//!   ([`BUNDLED_SPEC_VERSION`]). Accepted filings in superseded versions
//!   violate the current rules (one-digit districts in a spec 3.00 filing
//!   fail `^\d{2}$`), so at any other version those two keywords are left
//!   out and `x-fec.pattern_omitted_reason` / `x-fec.enum_omitted_reason`
//!   say why. The FEC values are still there, in `x-fec`.
//! * Blank is always allowed: `enum` includes `""` and `pattern` is the
//!   FEC's pattern with `^$|` in front (`x-fec.pattern` is verbatim). Most
//!   coded and formatted fields are optional or conditionally required
//!   (a donor candidate ID is blank on nearly every Schedule A line), and
//!   the FEC's validator checks a format only when a value is present.
//!   Whether blank is acceptable is the requirement rules' business (see
//!   `required` below).
//! * `description`: the FEC's field description, followed by the rule text
//!   when the workbook has one.
//! * `x-fec`: the workbook row as data: `column` (1-based, as the workbook
//!   counts), `type` (the FEC type code, `A/N-200`), `kind`, `max_len`,
//!   `required_level` (`none`, `error`, `warning`, `conditional`) with
//!   `required_condition` for the last, `sample`, `value_reference`,
//!   `rule`, `forms`, `allowed_values`, `pattern`.
//!
//! `required` lists the fields whose requirement is [`Requirement::Error`].
//! In JSON Schema that means the key must be present, not that the value
//! is non-blank; the schema does not add `minLength: 1`, because the
//! workbook's `X (error)` rows are often conditional in their rule text
//! (`Required if NOT [IND|CAN]`), and a blank there is accepted. The
//! conditional logic lives in [`crate::Filing::validate`]; the schema
//! carries the FEC's level in `x-fec.required_level`.
//!
//! `x-hardmoney` on the document carries `spec_version` (the version the
//! record is at), `table`, `layout_versions` (every version sharing this
//! column layout), `field_rules_version` (the version the field rules
//! describe), `column_base`, `width`, and the hardmoney version.
//!
//! # What is left out
//!
//! * Cross-field rules (`= 11ai + 11aii`, "Required if NOT [IND|CAN]") are
//!   prose in `x-fec.rule`; JSON Schema would need `if`/`then` chains that
//!   hardmoney does not attempt.
//! * `additionalProperties` is not set, so a record carrying extra keys
//!   (a line number, say) still validates.
//! * Layout fields the current workbook has no row for (a dropped field at
//!   an old version, or a table the workbook no longer documents) are
//!   `type: "string"` with `x-fec: {column, spec_row: false}`.
//!
//! ```
//! use hardmoney::parser::jsonschema::table_schema;
//! use hardmoney::parser::{SpecVersion, Table};
//!
//! let schema = table_schema(Table::SchA, SpecVersion::electronic(8, 5)).unwrap();
//! assert_eq!(schema["properties"]["contribution_amount"]["type"], "string");
//! assert_eq!(schema["properties"]["contribution_amount"]["x-fec"]["column"], 21);
//! assert!(table_schema(Table::SchI, SpecVersion::electronic(8, 5)).is_none());
//! ```

use serde_json::{Map, Value, json};

use crate::parser::schema::{FieldDef, FieldKind, FieldSpec, Layout, Requirement, SpecVersion};
use crate::parser::tables::{BUNDLED_SPEC_VERSION, Table};

/// The `$schema` URI every document this module emits declares.
pub const JSON_SCHEMA_DRAFT: &str = "https://json-schema.org/draft/2020-12/schema";

/// Why `enum` and `pattern` are omitted at a version other than the one
/// the workbook describes.
const SUPERSEDED_REASON: &str = "the FEC's code lists and formats describe the bundled spec version; \
     accepted filings in earlier versions violate them (one-digit districts in spec 3.00), \
     so they are informational here";

/// Why an `AMT-n` field's `maxLength` is `n + 2`.
const AMOUNT_LENGTH_NOTE: &str = "AMT-n bounds the digits; maxLength allows one sign and one decimal point \
     (the FEC accepted 2280311229.59 in an AMT-12 field)";

/// The JSON Schema for one record of `table` at `version`: an object keyed
/// by canonical field name, with one property per field of the layout at
/// that version. `None` when the table has no layout at `version` (Schedule
/// I at 8.5, or any table at `9.9`). Pure: depends only on the bundled data.
#[must_use]
pub fn table_schema(table: Table, version: SpecVersion) -> Option<Value> {
    let layout = table.layout(version)?;
    let mut doc = Map::new();
    doc.insert("$schema".into(), Value::from(JSON_SCHEMA_DRAFT));
    for (k, v) in layout_schema(layout, version) {
        doc.insert(k, v);
    }
    Some(Value::Object(doc))
}

/// One document with a `$defs` entry per table that has a layout at
/// `version`, keyed by table name (`SchA`, `F3X`, `TEXT`). The root has no
/// `type` of its own: reference the table you want
/// (`{"$ref": "#/$defs/SchA"}`, or in Python
/// `bundle["$defs"]["SchA"]`). Tables with no layout at `version` are
/// absent; the bundle for a version no table has is valid and has an empty
/// `$defs`.
#[must_use]
pub fn bundle(version: SpecVersion) -> Value {
    let mut defs = Map::new();
    for &table in Table::ALL {
        if let Some(layout) = table.layout(version) {
            defs.insert(
                table.as_str().to_string(),
                Value::Object(layout_schema(layout, version)),
            );
        }
    }
    json!({
        "$schema": JSON_SCHEMA_DRAFT,
        "title": format!("FEC electronic filing format {version}: one schema per table"),
        "description": format!(
            "One JSON Schema per table of the FEC electronic filing format at spec version {version}, \
             under $defs keyed by table name. Each describes one record as an object keyed by \
             canonical field name; every value is a string as filed. Field rules (type, length, \
             required level, code lists, formats) are the FEC's for version {BUNDLED_SPEC_VERSION}; \
             column positions are for version {version}."
        ),
        "$defs": defs,
        "x-hardmoney": {
            "spec_version": version.to_string(),
            "field_rules_version": BUNDLED_SPEC_VERSION,
            "column_base": 1,
            "tables": defs_names(&defs),
            "hardmoney_version": env!("CARGO_PKG_VERSION"),
        },
    })
}

fn defs_names(defs: &Map<String, Value>) -> Vec<&str> {
    defs.keys().map(String::as_str).collect()
}

/// The schema body (everything but `$schema`) for one layout. Shared by
/// the standalone document and the bundle, whose `$defs` entries may not
/// carry `$schema` (draft 2020-12 core, 8.1.1).
fn layout_schema(layout: &Layout, version: SpecVersion) -> Map<String, Value> {
    let current = is_bundled(version);
    let mut defs: Vec<&FieldDef> = layout.fields.iter().collect();
    defs.sort_by_key(|d| d.column);
    let mut properties = Map::new();
    let mut required = Vec::new();
    for def in defs {
        let spec = layout.table.spec(def.name);
        properties.insert(def.name.to_string(), property(def, spec, current));
        if spec.is_some_and(|s| s.required == Requirement::Error) {
            required.push(Value::from(def.name));
        }
    }
    let mut versions: Vec<String> = layout.versions.iter().map(ToString::to_string).collect();
    versions.sort_by_key(|v| v.parse::<SpecVersion>().ok());
    let rules_note = if current {
        String::new()
    } else {
        format!(
            " Field rules are the FEC's for version {BUNDLED_SPEC_VERSION}, joined by field name; \
             code lists and formats are informational (x-fec) at this version."
        )
    };
    let mut body = Map::new();
    body.insert(
        "title".into(),
        Value::from(format!("{} record, FEC spec {version}", layout.table)),
    );
    body.insert(
        "description".into(),
        Value::from(format!(
            "One {} record at FEC electronic filing spec version {version}, as an object keyed by \
             canonical field name. Every value is a string as filed (trimmed); a blank field is \
             the empty string. x-fec.column is the 1-based position in the delimited line.{rules_note}",
            layout.table
        )),
    );
    body.insert("type".into(), Value::from("object"));
    body.insert("properties".into(), Value::Object(properties));
    body.insert("required".into(), Value::Array(required));
    body.insert(
        "x-hardmoney".into(),
        json!({
            "spec_version": version.to_string(),
            "table": layout.table.as_str(),
            "layout_versions": versions,
            "field_rules_version": BUNDLED_SPEC_VERSION,
            "column_base": 1,
            "width": layout.width,
            "hardmoney_version": env!("CARGO_PKG_VERSION"),
        }),
    );
    body
}

/// One property: the constraints JSON Schema can enforce, then the FEC's
/// row as `x-fec`. `current` is whether the record's version is the one the
/// field rules describe.
fn property(def: &FieldDef, spec: Option<&FieldSpec>, current: bool) -> Value {
    let mut prop = Map::new();
    prop.insert("type".into(), Value::from("string"));
    let mut fec = Map::new();
    fec.insert("column".into(), Value::from(display_column(def.column)));
    let Some(spec) = spec else {
        fec.insert("spec_row".into(), Value::from(false));
        prop.insert("x-fec".into(), Value::Object(fec));
        return Value::Object(prop);
    };

    prop.insert("description".into(), Value::from(describe(spec)));
    if let Some(n) = spec.max_len {
        let bound = if spec.kind == FieldKind::Amount {
            u32::from(n).saturating_add(2)
        } else {
            u32::from(n)
        };
        prop.insert("maxLength".into(), Value::from(bound));
    }
    if !spec.allowed_values.is_empty() && current {
        let mut values = vec![Value::from("")];
        values.extend(spec.allowed_values.iter().map(|&v| Value::from(v)));
        prop.insert("enum".into(), Value::Array(values));
    }
    if let Some(p) = spec.pattern
        && current
    {
        prop.insert("pattern".into(), Value::from(format!("^$|{p}")));
    }

    fec.insert("type".into(), opt_string(type_code(spec)));
    fec.insert("kind".into(), Value::from(spec.kind.to_string()));
    fec.insert("max_len".into(), opt_u16(spec.max_len));
    if spec.kind == FieldKind::Amount && spec.max_len.is_some() {
        fec.insert("max_length_note".into(), Value::from(AMOUNT_LENGTH_NOTE));
    }
    let (level, condition) = requirement_parts(spec.required);
    fec.insert("required_level".into(), Value::from(level));
    if let Some(c) = condition {
        fec.insert("required_condition".into(), Value::from(c));
    }
    fec.insert("sample".into(), opt_str(spec.sample));
    fec.insert("value_reference".into(), opt_str(spec.value_reference));
    fec.insert("rule".into(), opt_str(spec.rule));
    fec.insert(
        "forms".into(),
        Value::Array(spec.forms.iter().map(|&f| Value::from(f)).collect()),
    );
    fec.insert(
        "allowed_values".into(),
        Value::Array(
            spec.allowed_values
                .iter()
                .map(|&v| Value::from(v))
                .collect(),
        ),
    );
    fec.insert("pattern".into(), opt_str(spec.pattern));
    if !current {
        if !spec.allowed_values.is_empty() {
            fec.insert("enum_omitted_reason".into(), Value::from(SUPERSEDED_REASON));
        }
        if spec.pattern.is_some() {
            fec.insert(
                "pattern_omitted_reason".into(),
                Value::from(SUPERSEDED_REASON),
            );
        }
    }
    prop.insert("x-fec".into(), Value::Object(fec));
    Value::Object(prop)
}

/// The FEC description, then the rule text on one line when there is one.
fn describe(spec: &FieldSpec) -> String {
    match spec.rule.map(one_line).filter(|r| !r.is_empty()) {
        Some(rule) => format!("{}. Rule: {rule}", spec.description),
        None => spec.description.to_string(),
    }
}

/// Collapses the workbook's embedded line breaks and runs of spaces.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The workbook's type code, reconstructed from kind and length
/// (`A/N-200`, `AMT-12`, `NUM-8`, `A-2`). The workbook writes both `N-n`
/// and `NUM-n` for numerics; the distillation keeps only the kind, so
/// numerics come back as `NUM-n`. `None` for a row with no parseable type.
fn type_code(spec: &FieldSpec) -> Option<String> {
    let prefix = match spec.kind {
        FieldKind::Alpha => "A",
        FieldKind::AlphaNumeric => "A/N",
        FieldKind::Numeric => "NUM",
        FieldKind::Amount => "AMT",
        FieldKind::Unknown => return None,
    };
    Some(match spec.max_len {
        Some(n) => format!("{prefix}-{n}"),
        None => prefix.to_string(),
    })
}

fn requirement_parts(r: Requirement) -> (&'static str, Option<&'static str>) {
    match r {
        Requirement::None => ("none", None),
        Requirement::Error => ("error", None),
        Requirement::Warning => ("warning", None),
        Requirement::Conditional(c) => ("conditional", Some(c)),
    }
}

fn is_bundled(version: SpecVersion) -> bool {
    BUNDLED_SPEC_VERSION
        .parse::<SpecVersion>()
        .is_ok_and(|v| v == version)
}

/// 1-based column, as the FEC's workbook numbers them.
fn display_column(column: u16) -> u32 {
    u32::from(column) + 1
}

fn opt_str(s: Option<&str>) -> Value {
    s.map_or(Value::Null, Value::from)
}

fn opt_string(s: Option<String>) -> Value {
    s.map_or(Value::Null, Value::from)
}

fn opt_u16(n: Option<u16>) -> Value {
    n.map_or(Value::Null, Value::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    const V30: SpecVersion = SpecVersion::electronic(3, 0);
    const V70: SpecVersion = SpecVersion::electronic(7, 0);
    const V85: SpecVersion = SpecVersion::electronic(8, 5);

    fn props(schema: &Value) -> &Map<String, Value> {
        schema["properties"].as_object().unwrap()
    }

    #[test]
    fn schedule_a_at_8_5_is_a_record_object_in_column_order() {
        let s = table_schema(Table::SchA, V85).unwrap();
        assert_eq!(s["$schema"], JSON_SCHEMA_DRAFT);
        assert_eq!(s["type"], "object");
        assert_eq!(s["title"], "SchA record, FEC spec 8.5");
        let layout = Table::SchA.layout(V85).unwrap();
        let p = props(&s);
        assert_eq!(p.len(), layout.fields.len());
        let columns: Vec<u64> = p
            .values()
            .map(|v| v["x-fec"]["column"].as_u64().unwrap())
            .collect();
        assert!(columns.windows(2).all(|w| w[0] < w[1]), "{columns:?}");
        assert_eq!(columns.first(), Some(&1));
        assert_eq!(p.keys().next().map(String::as_str), Some("form_type"));

        let amount = &p["contribution_amount"];
        assert_eq!(amount["type"], "string");
        assert_eq!(amount["x-fec"]["type"], "AMT-12");
        assert_eq!(amount["x-fec"]["max_len"], 12);
        assert_eq!(amount["maxLength"], 14);
        assert!(amount["x-fec"]["max_length_note"].is_string());

        let id = &p["filer_committee_id_number"];
        assert_eq!(id["maxLength"], 9);
        assert_eq!(id["x-fec"]["type"], "A/N-9");
        assert_eq!(id["x-fec"]["required_level"], "error");
        assert!(id["x-fec"]["max_length_note"].is_null());

        let form_type = &p["form_type"];
        assert_eq!(
            form_type["description"],
            "FORM TYPE. Rule: Appendix C. SA3L must be used with the F3L"
        );
        assert_eq!(
            form_type["x-fec"]["rule"],
            "Appendix C.  SA3L must be used \nwith the F3L"
        );
        assert_eq!(form_type["x-fec"]["forms"][0], "F3");

        let required = s["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "form_type"));
        assert!(required.iter().any(|r| r == "transaction_id"));
        assert!(!required.iter().any(|r| r == "back_reference_tran_id"));
        for r in required {
            assert!(p.contains_key(r.as_str().unwrap()), "{r}");
        }

        let hm = &s["x-hardmoney"];
        assert_eq!(hm["spec_version"], "8.5");
        assert_eq!(hm["table"], "SchA");
        assert_eq!(hm["field_rules_version"], "8.5");
        assert_eq!(hm["column_base"], 1);
        assert_eq!(hm["width"], u64::from(layout.width));
        assert!(
            hm["layout_versions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "8.5")
        );
    }

    #[test]
    fn enum_and_pattern_appear_only_at_the_bundled_version() {
        let now = table_schema(Table::F3X, V85).unwrap();
        let ft = &props(&now)["form_type"];
        // Blank first: the schema never rejects a blank on its own.
        assert_eq!(ft["enum"], json!(["", "F3XA", "F3XN", "F3XT"]));
        assert_eq!(
            ft["x-fec"]["allowed_values"],
            json!(["F3XA", "F3XN", "F3XT"])
        );
        assert!(ft["x-fec"]["enum_omitted_reason"].is_null());
        let id = &props(&now)["filer_committee_id_number"];
        assert_eq!(
            id["pattern"],
            "^$|^[C|P][0-9]{8}$|^[H|S][0-9]{1}[A-Z]{2}[0-9]{5}$"
        );
        assert_eq!(
            id["x-fec"]["pattern"],
            "^[C|P][0-9]{8}$|^[H|S][0-9]{1}[A-Z]{2}[0-9]{5}$"
        );

        let then = table_schema(Table::F3X, V70).unwrap();
        let ft = &props(&then)["form_type"];
        assert!(ft.get("enum").is_none(), "{ft}");
        assert_eq!(
            ft["x-fec"]["allowed_values"],
            json!(["F3XA", "F3XN", "F3XT"])
        );
        assert_eq!(ft["x-fec"]["enum_omitted_reason"], SUPERSEDED_REASON);
        let id = &props(&then)["filer_committee_id_number"];
        assert!(id.get("pattern").is_none(), "{id}");
        assert_eq!(id["x-fec"]["pattern_omitted_reason"], SUPERSEDED_REASON);
        // Lengths still apply at superseded versions.
        assert_eq!(id["maxLength"], 9);
        assert!(
            then["description"]
                .as_str()
                .unwrap()
                .contains("informational")
        );
        assert!(
            !now["description"]
                .as_str()
                .unwrap()
                .contains("informational")
        );
    }

    #[test]
    fn a_field_without_a_spec_row_is_a_bare_string() {
        // Spec 8.0 dropped `contribution_purpose_code`; at 7.0 the layout
        // has it and the 8.5 workbook says nothing about it.
        let s = table_schema(Table::SchA, V70).unwrap();
        let p = &props(&s)["contribution_purpose_code"];
        assert_eq!(p["type"], "string");
        assert_eq!(p["x-fec"]["spec_row"], false);
        assert_eq!(p["x-fec"]["column"], 23);
        assert!(p.get("description").is_none());
        assert!(p.get("maxLength").is_none());
        // Schedule I has no spec rows at all, so no field is required.
        let sch_i = table_schema(Table::SchI, SpecVersion::electronic(8, 4)).unwrap();
        assert_eq!(sch_i["required"], json!([]));
        assert!(
            props(&sch_i)
                .values()
                .all(|p| p["x-fec"]["spec_row"] == false)
        );
    }

    #[test]
    fn missing_layout_is_none_not_a_panic() {
        assert!(table_schema(Table::SchI, V85).is_none());
        assert!(table_schema(Table::SchA, SpecVersion::electronic(9, 9)).is_none());
        assert!(table_schema(Table::SchA, V30).is_some());
        assert!(table_schema(Table::SchA, SpecVersion::paper(3, 4)).is_some());
    }

    #[test]
    fn bundle_has_one_def_per_table_with_a_layout_and_no_nested_schema_keyword() {
        let b = bundle(V85);
        assert_eq!(b["$schema"], JSON_SCHEMA_DRAFT);
        let defs = b["$defs"].as_object().unwrap();
        let expected = Table::ALL
            .iter()
            .filter(|t| t.layout(V85).is_some())
            .count();
        assert_eq!(defs.len(), expected);
        assert!(defs.contains_key("SchA"));
        assert!(defs.contains_key("TEXT"));
        assert!(!defs.contains_key("SchI"));
        for (name, def) in defs {
            assert!(def.get("$schema").is_none(), "{name}");
            assert_eq!(def["x-hardmoney"]["table"], name.as_str());
            assert_eq!(def["type"], "object");
        }
        let names = b["x-hardmoney"]["tables"].as_array().unwrap();
        assert_eq!(names.len(), expected);
        assert_eq!(b["x-hardmoney"]["spec_version"], "8.5");
        // A standalone document is the def plus `$schema`.
        let mut standalone = table_schema(Table::SchA, V85).unwrap();
        standalone.as_object_mut().unwrap().remove("$schema");
        assert_eq!(standalone, defs["SchA"]);

        let none = bundle(SpecVersion::electronic(9, 9));
        assert_eq!(none["$defs"], json!({}));
    }

    #[test]
    fn type_codes_round_trip_the_workbook_spelling() {
        let mk = |kind, max_len| FieldSpec {
            column: 0,
            canonical: None,
            description: "",
            kind,
            max_len,
            required: Requirement::None,
            sample: None,
            value_reference: None,
            rule: None,
            forms: &[],
            allowed_values: &[],
            pattern: None,
        };
        assert_eq!(
            type_code(&mk(FieldKind::AlphaNumeric, Some(200))).as_deref(),
            Some("A/N-200")
        );
        assert_eq!(
            type_code(&mk(FieldKind::Alpha, Some(2))).as_deref(),
            Some("A-2")
        );
        assert_eq!(
            type_code(&mk(FieldKind::Numeric, Some(8))).as_deref(),
            Some("NUM-8")
        );
        assert_eq!(
            type_code(&mk(FieldKind::Amount, Some(12))).as_deref(),
            Some("AMT-12")
        );
        assert_eq!(type_code(&mk(FieldKind::Unknown, None)), None);
        assert_eq!(
            requirement_parts(Requirement::Conditional("X (error if 15=MSM)")),
            ("conditional", Some("X (error if 15=MSM)"))
        );
        assert_eq!(requirement_parts(Requirement::Warning), ("warning", None));
    }
}
