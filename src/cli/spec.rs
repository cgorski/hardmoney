//! `hardmoney spec`: the FEC format specification as data.
//!
//! The FEC publishes its electronic-filing format only as an Excel workbook
//! plus per-version column tables. `build.rs` compiles those into the
//! [`Table`] enum, the per-version [`Layout`]s, and the [`FieldSpec`] rows;
//! this command exposes that compiled data from the terminal so a filing
//! vendor, a researcher, or a script can list the tables, look up where a
//! field lives and what rule governs it, export the whole thing as JSON, or
//! diff two spec versions -- without reading Rust.
//!
//! Column numbering: `fields`, `diff`, `export --format json-schema`, and
//! `export --format csv` are human-facing and count columns from 1, the way
//! the FEC's workbook does. `export --format json` is the raw data model and
//! counts from 0, matching [`FieldDef::column`] and [`FieldSpec::column`]
//! (the document says so in its `column_base` key).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use clap::{Args, Subcommand, ValueEnum};
use hardmoney::parser::form::table_for_form_type;
use hardmoney::parser::jsonschema;
use hardmoney::parser::{
    BUNDLED_SPEC_VERSION, FieldDef, FieldKind, FieldSpec, Layout, Requirement, SpecVersion, Table,
};
use serde::Serialize;

#[derive(Args, Debug)]
pub struct SpecArgs {
    #[command(subcommand)]
    pub command: SpecCommand,
}

#[derive(Subcommand, Debug)]
pub enum SpecCommand {
    /// List every table: version buckets, fields, and FEC spec rows.
    Tables(TablesArgs),
    /// The fields of one table at one spec version, in column order.
    Fields(FieldsArgs),
    /// The complete machine-readable specification: JSON, JSON Schema, or
    /// CSV.
    Export(ExportArgs),
    /// Fields added, removed, and moved between two spec versions.
    Diff(DiffArgs),
}

#[derive(Args, Debug)]
pub struct TablesArgs {
    /// Print JSON instead of an aligned table.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct FieldsArgs {
    /// Table name as `hardmoney spec tables` lists it, e.g. SchA, F3X, TEXT
    /// (case-insensitive).
    #[arg(value_parser = parse_table)]
    pub table: Table,

    /// Spec version whose column layout to show, e.g. 8.5, 6.4, 3.00
    /// [default: the bundled spec version]. Description, type, length,
    /// required level, and rule come from the FEC's spec rows, which
    /// describe the bundled version; columns come from the layout for the
    /// version asked for.
    #[arg(long)]
    pub version: Option<SpecVersion>,

    /// Print a JSON array (one object per field) instead of an aligned
    /// table. Columns are 1-based in both.
    #[arg(long)]
    pub json: bool,
}

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Output format. `json`: the raw data model (every table, every
    /// version-bucketed layout, every FEC spec row; 0-based columns).
    /// `json-schema`: one JSON Schema (draft 2020-12) per table describing
    /// a record as an object keyed by canonical field name, as a `$defs`
    /// bundle or, with --table, a standalone document. `csv`: one row per
    /// table, version bucket, and field, with the FEC's spec columns, for
    /// spreadsheet users. Columns are 1-based in json-schema and csv.
    #[arg(long, value_enum, default_value_t = ExportFormat::Json)]
    pub format: ExportFormat,

    /// Only this table (e.g. SchA, F3X, TEXT; case-insensitive).
    #[arg(long, value_parser = parse_table)]
    pub table: Option<Table>,

    /// For json-schema: the spec version the record is at, e.g. 8.5, 7.0
    /// [default: the bundled spec version]; columns come from that
    /// version's layout, field rules from the bundled workbook. For json
    /// and csv: only layouts that cover this version [default: all].
    #[arg(long)]
    pub version: Option<SpecVersion>,

    /// Accepted for consistency with the other subcommands; the same as
    /// `--format json`.
    #[arg(long, conflicts_with = "format")]
    pub json: bool,
}

/// What `spec export` writes.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportFormat {
    /// The raw data model, 0-based columns.
    Json,
    /// JSON Schema draft 2020-12, one schema per table.
    JsonSchema,
    /// One flat row per table, version bucket, and field.
    Csv,
}

#[derive(Args, Debug)]
pub struct DiffArgs {
    /// The "before" spec version, e.g. 7.0.
    pub from: SpecVersion,

    /// The "after" spec version, e.g. 8.5.
    pub to: SpecVersion,

    /// Only diff this table.
    #[arg(long, value_parser = parse_table)]
    pub table: Option<Table>,

    /// Print JSON instead of the one-line-per-table summary. Columns are
    /// 1-based in both.
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: SpecArgs) -> super::CliResult {
    match args.command {
        SpecCommand::Tables(a) => tables(a),
        SpecCommand::Fields(a) => fields(a),
        SpecCommand::Export(a) => export(a),
        SpecCommand::Diff(a) => diff(a),
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// What the user asked for that the bundled data does not have.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SpecError {
    /// Not a table name. `hint` names the table a form-type token would
    /// dispatch to, when the input looks like one (`SA11AI` -> `SchA`).
    #[error("'{name}' is not a table name.{hint} Valid names are: {valid}")]
    UnknownTable {
        name: String,
        hint: String,
        valid: String,
    },
    /// The table exists but the bundled data has no layout at that version.
    #[error(
        "{table} has no column layout for spec version {version}; it has layouts for: {available}"
    )]
    NoLayout {
        table: Table,
        version: SpecVersion,
        available: String,
    },
    /// No table at all has a layout at that version.
    #[error("no table has a column layout for spec version {version}; known versions are: {known}")]
    UnknownVersion { version: SpecVersion, known: String },
}

/// The FEC's maximum length for a form-type token (`A/N-8`); anything
/// longer, or containing whitespace, is not one.
const FORM_TYPE_MAX_LEN: usize = 8;

/// Resolves a table name (`SchA`, `f3x`, `TEXT`) to a [`Table`]. Fails,
/// listing every valid name, on anything else -- including a form-type
/// token such as `SA11AI`, for which the error names the table a filing
/// line with that token is parsed with rather than guessing (dispatch is
/// prefix-based, so a typo like `ScheA` would otherwise silently become
/// Schedule C).
pub fn parse_table(name: &str) -> Result<Table, SpecError> {
    let trimmed = name.trim();
    trimmed.parse::<Table>().map_err(|_| {
        let looks_like_token = trimmed.chars().count() <= FORM_TYPE_MAX_LEN
            && !trimmed.chars().any(char::is_whitespace);
        let hint = if looks_like_token {
            table_for_form_type(trimmed)
                .map(|t| {
                    format!(" (A filing line with form type '{trimmed}' is parsed with table {t}.)")
                })
                .unwrap_or_default()
        } else {
            String::new()
        };
        SpecError::UnknownTable {
            name: name.to_string(),
            hint,
            valid: valid_table_names(),
        }
    })
}

fn valid_table_names() -> String {
    Table::ALL
        .iter()
        .map(|t| t.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every spec version that at least one table has a layout for.
fn known_versions() -> BTreeSet<SpecVersion> {
    Table::ALL
        .iter()
        .flat_map(|t| t.layouts())
        .flat_map(|l| l.versions.iter().copied())
        .collect()
}

fn version_list(versions: impl IntoIterator<Item = SpecVersion>) -> String {
    versions
        .into_iter()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

/// The layout of `table` at `version`, or an error naming the versions it
/// does have.
fn layout_or_error(table: Table, version: SpecVersion) -> Result<&'static Layout, SpecError> {
    table
        .layout(version)
        .ok_or_else(|| no_layout(table, version))
}

/// The error for `table` having no layout at `version`, naming the
/// versions it does have.
fn no_layout(table: Table, version: SpecVersion) -> SpecError {
    SpecError::NoLayout {
        table,
        version,
        available: version_list(table_versions(table)),
    }
}

/// Every spec version `table` has a layout for.
fn table_versions(table: Table) -> BTreeSet<SpecVersion> {
    table
        .layouts()
        .iter()
        .flat_map(|l| l.versions.iter().copied())
        .collect()
}

/// Fails unless some table has a layout at `version`, so a diff against a
/// version nobody has (`9.9`) is an error rather than "every table lost".
fn known_version_or_error(version: SpecVersion) -> Result<SpecVersion, SpecError> {
    let known = known_versions();
    if known.contains(&version) {
        Ok(version)
    } else {
        Err(SpecError::UnknownVersion {
            version,
            known: version_list(known),
        })
    }
}

fn bundled_version() -> Result<SpecVersion, SpecError> {
    // The generator writes this constant from a file name it already
    // parsed, so this cannot fail in a build that compiled; surface it as
    // an error anyway rather than trusting it.
    BUNDLED_SPEC_VERSION
        .parse::<SpecVersion>()
        .map_err(|_| SpecError::UnknownVersion {
            version: SpecVersion::electronic(0, 0),
            known: version_list(known_versions()),
        })
}

/// 1-based column for display, as the FEC's workbook numbers them.
fn display_column(column: u16) -> u32 {
    u32::from(column) + 1
}

// ---------------------------------------------------------------------------
// tables
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct TableRow {
    table: &'static str,
    /// Distinct column layouts (each covers a contiguous run of versions).
    version_buckets: usize,
    /// Canonical fields known across all versions.
    fields: usize,
    /// FEC spec rows at the bundled version; 0 for tables the current spec
    /// no longer documents.
    spec_rows: usize,
    /// Oldest and newest *electronic* version with a layout.
    oldest_version: Option<SpecVersion>,
    newest_version: Option<SpecVersion>,
    /// Paper-conversion (`P`-prefixed) versions with a layout.
    paper_versions: usize,
}

fn table_row(table: Table) -> TableRow {
    let versions: BTreeSet<SpecVersion> = table
        .layouts()
        .iter()
        .flat_map(|l| l.versions.iter().copied())
        .collect();
    let mut electronic = versions.iter().filter(|v| !v.is_paper());
    TableRow {
        table: table.as_str(),
        version_buckets: table.layouts().len(),
        fields: table.field_names().len(),
        spec_rows: table.specs().len(),
        oldest_version: electronic.next().copied(),
        newest_version: electronic.next_back().copied(),
        paper_versions: versions.iter().filter(|v| v.is_paper()).count(),
    }
}

fn tables(args: TablesArgs) -> super::CliResult {
    let rows: Vec<TableRow> = Table::ALL.iter().map(|&t| table_row(t)).collect();
    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.table.to_string(),
                r.version_buckets.to_string(),
                r.fields.to_string(),
                r.spec_rows.to_string(),
                r.oldest_version.map(|v| v.to_string()).unwrap_or_default(),
                r.newest_version.map(|v| v.to_string()).unwrap_or_default(),
                r.paper_versions.to_string(),
            ]
        })
        .collect();
    print!(
        "{}",
        render_table(
            &[
                "table",
                "buckets",
                "fields",
                "spec_rows",
                "oldest",
                "newest",
                "paper"
            ],
            &cells
        )
    );
    println!(
        "{} tables; spec rows describe version {BUNDLED_SPEC_VERSION}",
        rows.len()
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// fields
// ---------------------------------------------------------------------------

/// One field of one table at one version, joined with its FEC spec row.
#[derive(Debug, Serialize)]
struct FieldRow {
    /// 1-based column in the delimited record.
    column: u32,
    name: &'static str,
    description: Option<&'static str>,
    kind: Option<FieldKind>,
    max_len: Option<u16>,
    required: Option<Requirement>,
    rule: Option<&'static str>,
}

fn field_row(def: &FieldDef, spec: Option<&'static FieldSpec>) -> FieldRow {
    FieldRow {
        column: display_column(def.column),
        name: def.name,
        description: spec.map(|s| s.description),
        kind: spec.map(|s| s.kind),
        max_len: spec.and_then(|s| s.max_len),
        required: spec.map(|s| s.required),
        rule: spec.and_then(|s| s.rule),
    }
}

/// The rows of `layout` in column order, each joined with the spec row of
/// the same canonical name where the bundled spec has one.
fn field_rows(layout: &Layout) -> Vec<FieldRow> {
    let mut defs: Vec<&FieldDef> = layout.fields.iter().collect();
    defs.sort_by_key(|d| d.column);
    defs.iter()
        .map(|d| field_row(d, layout.table.spec(d.name)))
        .collect()
}

fn requirement_label(r: Requirement) -> &'static str {
    match r {
        Requirement::None => "",
        Requirement::Error => "error",
        Requirement::Warning => "warning",
        Requirement::Conditional(_) => "conditional",
        _ => "?",
    }
}

/// Collapses the workbook's embedded line breaks and runs of spaces so a
/// rule fits on one row.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn fields(args: FieldsArgs) -> super::CliResult {
    let version = match args.version {
        Some(v) => v,
        None => bundled_version()?,
    };
    let layout = layout_or_error(args.table, version)?;
    let rows = field_rows(layout);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&rows)?);
        return Ok(());
    }
    println!(
        "{} at {version}: {} fields in {} columns, {} FEC spec rows",
        args.table,
        rows.len(),
        layout.width,
        args.table.specs().len()
    );
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.column.to_string(),
                r.name.to_string(),
                r.kind.map(|k| k.to_string()).unwrap_or_default(),
                r.max_len.map(|n| n.to_string()).unwrap_or_default(),
                r.required
                    .map(requirement_label)
                    .unwrap_or_default()
                    .to_string(),
                r.description.unwrap_or_default().to_string(),
                r.rule.map(one_line).unwrap_or_default(),
            ]
        })
        .collect();
    print!(
        "{}",
        render_table(
            &[
                "col",
                "field",
                "kind",
                "len",
                "required",
                "description",
                "rule"
            ],
            &cells
        )
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// export
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct FieldExport {
    name: &'static str,
    /// 0-based, as in [`FieldDef::column`].
    column: u16,
}

#[derive(Debug, Serialize)]
struct LayoutExport {
    versions: &'static [SpecVersion],
    width: u16,
    fields: Vec<FieldExport>,
}

#[derive(Debug, Serialize)]
struct TableExport {
    table: &'static str,
    layouts: Vec<LayoutExport>,
    specs: &'static [FieldSpec],
}

#[derive(Debug, Serialize)]
struct SpecExport {
    bundled_spec_version: &'static str,
    /// Every `column` in this document, layouts and spec rows alike, counts
    /// from this number.
    column_base: u8,
    tables: Vec<TableExport>,
}

/// The layouts of `table` in bundled order (newest bucket first); only
/// those covering `version` when one is given.
fn selected_layouts(table: Table, version: Option<SpecVersion>) -> Vec<&'static Layout> {
    table
        .layouts()
        .iter()
        .filter(|l| version.is_none_or(|v| l.supports(v)))
        .collect()
}

fn export_table(table: Table, version: Option<SpecVersion>) -> TableExport {
    TableExport {
        table: table.as_str(),
        layouts: selected_layouts(table, version)
            .into_iter()
            .map(|l| LayoutExport {
                versions: l.versions,
                width: l.width,
                fields: l
                    .fields
                    .iter()
                    .map(|f| FieldExport {
                        name: f.name,
                        column: f.column,
                    })
                    .collect(),
            })
            .collect(),
        specs: table.specs(),
    }
}

/// The tables an export covers: the one asked for, or every table; with a
/// version, only tables that have a layout at it. Asking for one table at
/// a version it has no layout for is an error naming the versions it has.
fn selected_tables(
    table: Option<Table>,
    version: Option<SpecVersion>,
) -> Result<Vec<Table>, SpecError> {
    match (table, version) {
        (Some(t), Some(v)) => {
            layout_or_error(t, v)?;
            Ok(vec![t])
        }
        (Some(t), None) => Ok(vec![t]),
        (None, Some(v)) => {
            known_version_or_error(v)?;
            Ok(Table::ALL
                .iter()
                .copied()
                .filter(|t| t.supports_version(v))
                .collect())
        }
        (None, None) => Ok(Table::ALL.to_vec()),
    }
}

fn export(args: ExportArgs) -> super::CliResult {
    match args.format {
        ExportFormat::Json => {
            let tables = selected_tables(args.table, args.version)?;
            let doc = SpecExport {
                bundled_spec_version: BUNDLED_SPEC_VERSION,
                column_base: 0,
                tables: tables
                    .into_iter()
                    .map(|t| export_table(t, args.version))
                    .collect(),
            };
            println!("{}", serde_json::to_string_pretty(&doc)?);
        }
        ExportFormat::JsonSchema => {
            let version = match args.version {
                Some(v) => known_version_or_error(v)?,
                None => bundled_version()?,
            };
            let doc = match args.table {
                Some(t) => {
                    jsonschema::table_schema(t, version).ok_or_else(|| no_layout(t, version))?
                }
                None => jsonschema::bundle(version),
            };
            println!("{}", serde_json::to_string_pretty(&doc)?);
        }
        ExportFormat::Csv => {
            let tables = selected_tables(args.table, args.version)?;
            let mut w = csv::Writer::from_writer(std::io::stdout().lock());
            for row in csv_rows(&tables, args.version) {
                w.serialize(row)?;
            }
            w.flush()?;
        }
    }
    Ok(())
}

/// One field of one table in one version bucket, with its FEC spec row
/// joined by canonical name: the FEC's spreadsheet given back as a
/// spreadsheet, complete across versions. Multi-valued cells are
/// `|`-separated, the workbook's own convention (`F3|F3X`).
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CsvRow {
    pub table: &'static str,
    /// Every version this bucket covers, oldest first, space-separated.
    pub versions: String,
    pub first_version: SpecVersion,
    pub last_version: SpecVersion,
    /// Cells a record of this bucket has.
    pub width: u16,
    /// 1-based column in the delimited record.
    pub column: u32,
    pub field: &'static str,
    /// The spec version the remaining columns describe (the bundled one),
    /// or blank when the current workbook has no row for this field.
    pub rules_version: &'static str,
    pub description: String,
    /// The FEC type code, e.g. `A/N-200`.
    pub r#type: String,
    pub kind: String,
    pub max_len: Option<u16>,
    pub required_level: &'static str,
    pub required_condition: String,
    pub sample: String,
    pub value_reference: String,
    pub rule: String,
    pub forms: String,
    pub allowed_values: String,
    pub pattern: String,
}

/// The CSV rows for `tables`, in table order, newest bucket first, then by
/// column. Pure.
#[must_use]
pub fn csv_rows(tables: &[Table], version: Option<SpecVersion>) -> Vec<CsvRow> {
    let mut rows = Vec::new();
    for &table in tables {
        for layout in selected_layouts(table, version) {
            let mut versions: Vec<SpecVersion> = layout.versions.to_vec();
            versions.sort();
            let (Some(&first), Some(&last)) = (versions.first(), versions.last()) else {
                continue;
            };
            let joined = version_list(versions.iter().copied()).replace(", ", " ");
            let mut defs: Vec<&FieldDef> = layout.fields.iter().collect();
            defs.sort_by_key(|d| d.column);
            for def in defs {
                rows.push(csv_row(
                    table,
                    layout,
                    &joined,
                    first,
                    last,
                    def,
                    table.spec(def.name),
                ));
            }
        }
    }
    rows
}

fn csv_row(
    table: Table,
    layout: &Layout,
    versions: &str,
    first: SpecVersion,
    last: SpecVersion,
    def: &FieldDef,
    spec: Option<&'static FieldSpec>,
) -> CsvRow {
    let (level, condition) = match spec.map(|s| s.required) {
        Some(Requirement::Error) => ("error", ""),
        Some(Requirement::Warning) => ("warning", ""),
        Some(Requirement::Conditional(c)) => ("conditional", c),
        Some(Requirement::None) => ("none", ""),
        Some(_) => ("?", ""),
        None => ("", ""),
    };
    CsvRow {
        table: table.as_str(),
        versions: versions.to_string(),
        first_version: first,
        last_version: last,
        width: layout.width,
        column: display_column(def.column),
        field: def.name,
        rules_version: if spec.is_some() {
            BUNDLED_SPEC_VERSION
        } else {
            ""
        },
        description: spec.map(|s| s.description.to_string()).unwrap_or_default(),
        r#type: spec.and_then(type_code).unwrap_or_default(),
        kind: spec.map(|s| s.kind.to_string()).unwrap_or_default(),
        max_len: spec.and_then(|s| s.max_len),
        required_level: level,
        required_condition: one_line(condition),
        sample: spec.and_then(|s| s.sample).unwrap_or_default().to_string(),
        value_reference: spec
            .and_then(|s| s.value_reference)
            .map(one_line)
            .unwrap_or_default(),
        rule: spec.and_then(|s| s.rule).map(one_line).unwrap_or_default(),
        forms: spec.map(|s| s.forms.join("|")).unwrap_or_default(),
        allowed_values: spec.map(|s| s.allowed_values.join("|")).unwrap_or_default(),
        pattern: spec.and_then(|s| s.pattern).unwrap_or_default().to_string(),
    }
}

/// The workbook's type code from kind and length (`A/N-200`, `AMT-12`,
/// `NUM-8`); `None` for a row whose type did not parse.
fn type_code(spec: &FieldSpec) -> Option<String> {
    let prefix = match spec.kind {
        FieldKind::Alpha => "A",
        FieldKind::AlphaNumeric => "A/N",
        FieldKind::Numeric => "NUM",
        FieldKind::Amount => "AMT",
        _ => return None,
    };
    Some(match spec.max_len {
        Some(n) => format!("{prefix}-{n}"),
        None => prefix.to_string(),
    })
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

/// One field's fate between two layouts of the same table. Columns are
/// 1-based, as displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "change", rename_all = "snake_case")]
pub enum FieldChange {
    /// In `to` but not `from`.
    Added { name: &'static str, column: u32 },
    /// In `from` but not `to`.
    Removed { name: &'static str, column: u32 },
    /// In both, at a different column.
    Moved {
        name: &'static str,
        from: u32,
        to: u32,
    },
}

impl std::fmt::Display for FieldChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldChange::Added { name, column } => write!(f, "+{name}@{column}"),
            FieldChange::Removed { name, column } => write!(f, "-{name}@{column}"),
            FieldChange::Moved { name, from, to } => write!(f, "{name} {from}->{to}"),
        }
    }
}

/// Everything that differs between two layouts of one table. Empty
/// `changes` means the layouts agree field for field (the widths may still
/// differ if a trailing unnamed column was added).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LayoutDiff {
    #[serde(serialize_with = "table_name")]
    pub table: Table,
    pub from_width: u16,
    pub to_width: u16,
    /// Added first (by new column), then removed (by old column), then
    /// moved (by new column).
    pub changes: Vec<FieldChange>,
}

impl LayoutDiff {
    /// True when no field was added, removed, or moved.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Compares two layouts of the same table field by field. Pure: depends
/// only on the two layouts' `fields`, `width`, and `to.table`.
#[must_use]
pub fn diff_layouts(from: &Layout, to: &Layout) -> LayoutDiff {
    let mut added = Vec::new();
    let mut moved = Vec::new();
    for f in to.fields {
        match from.field(f.name) {
            None => added.push(FieldChange::Added {
                name: f.name,
                column: display_column(f.column),
            }),
            Some(old) if old.column != f.column => moved.push(FieldChange::Moved {
                name: f.name,
                from: display_column(old.column),
                to: display_column(f.column),
            }),
            Some(_) => {}
        }
    }
    let mut removed: Vec<FieldChange> = from
        .fields
        .iter()
        .filter(|f| to.field(f.name).is_none())
        .map(|f| FieldChange::Removed {
            name: f.name,
            column: display_column(f.column),
        })
        .collect();
    let by_column = |c: &FieldChange| match c {
        FieldChange::Added { column, .. } | FieldChange::Removed { column, .. } => *column,
        FieldChange::Moved { to, .. } => *to,
    };
    added.sort_by_key(by_column);
    removed.sort_by_key(by_column);
    moved.sort_by_key(by_column);
    let mut changes = added;
    changes.append(&mut removed);
    changes.append(&mut moved);
    LayoutDiff {
        table: to.table,
        from_width: from.width,
        to_width: to.width,
        changes,
    }
}

/// A table that has a layout at only one of the two versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct TableAtOneEnd {
    #[serde(serialize_with = "table_name")]
    pub table: Table,
    /// Fields in the layout that exists.
    pub fields: usize,
}

/// The whole-format diff between two spec versions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SpecDiff {
    pub from: SpecVersion,
    pub to: SpecVersion,
    /// Every column in this document counts from this number.
    pub column_base: u8,
    /// Tables with a layout at `to` but not `from`.
    pub gained: Vec<TableAtOneEnd>,
    /// Tables with a layout at `from` but not `to`.
    pub lost: Vec<TableAtOneEnd>,
    /// Tables with a layout at both whose fields differ.
    pub changed: Vec<LayoutDiff>,
    /// Tables with a layout at both that agree field for field.
    #[serde(serialize_with = "table_names")]
    pub unchanged: Vec<Table>,
}

/// Diffs `tables` between two versions. Tables with a layout at neither
/// version are left out entirely. Pure.
#[must_use]
pub fn diff_versions(tables: &[Table], from: SpecVersion, to: SpecVersion) -> SpecDiff {
    let mut diff = SpecDiff {
        from,
        to,
        column_base: 1,
        gained: Vec::new(),
        lost: Vec::new(),
        changed: Vec::new(),
        unchanged: Vec::new(),
    };
    for &table in tables {
        match (table.layout(from), table.layout(to)) {
            (None, None) => {}
            (None, Some(l)) => diff.gained.push(TableAtOneEnd {
                table,
                fields: l.fields.len(),
            }),
            (Some(l), None) => diff.lost.push(TableAtOneEnd {
                table,
                fields: l.fields.len(),
            }),
            (Some(a), Some(b)) => {
                let d = diff_layouts(a, b);
                if d.is_empty() {
                    diff.unchanged.push(table);
                } else {
                    diff.changed.push(d);
                }
            }
        }
    }
    diff
}

fn table_name<S: serde::Serializer>(t: &Table, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_str(t.as_str())
}

fn table_names<S: serde::Serializer>(ts: &[Table], s: S) -> Result<S::Ok, S::Error> {
    s.collect_seq(ts.iter().map(|t| t.as_str()))
}

/// The one-line-per-table summary, e.g.
/// `SchA: -contribution_purpose_code@23, contributor_employer 25->24`.
fn render_diff(diff: &SpecDiff) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{} -> {}", diff.from, diff.to);
    for d in &diff.changed {
        let changes: Vec<String> = d.changes.iter().map(ToString::to_string).collect();
        let _ = writeln!(out, "{}: {}", d.table, changes.join(", "));
    }
    for g in &diff.gained {
        let _ = writeln!(
            out,
            "{}: gained a layout ({} fields; none at {})",
            g.table, g.fields, diff.from
        );
    }
    for l in &diff.lost {
        let _ = writeln!(
            out,
            "{}: lost its layout ({} fields at {}; none at {})",
            l.table, l.fields, diff.from, diff.to
        );
    }
    if !diff.unchanged.is_empty() {
        let names: Vec<&str> = diff.unchanged.iter().map(|t| t.as_str()).collect();
        let _ = writeln!(
            out,
            "unchanged ({}): {}",
            diff.unchanged.len(),
            names.join(", ")
        );
    }
    if diff.changed.is_empty() && diff.gained.is_empty() && diff.lost.is_empty() {
        let _ = writeln!(out, "no differences");
    }
    out
}

fn diff(args: DiffArgs) -> super::CliResult {
    let from = known_version_or_error(args.from)?;
    let to = known_version_or_error(args.to)?;
    if let Some(t) = args.table
        && t.layout(from).is_none()
        && t.layout(to).is_none()
    {
        // A single table with a layout at neither end would otherwise print
        // "no differences", which is true but unhelpful; report the `from`
        // side's missing layout instead.
        layout_or_error(t, from)?;
    }
    let tables: Vec<Table> = match args.table {
        Some(t) => vec![t],
        None => Table::ALL.to_vec(),
    };
    let result = diff_versions(&tables, from, to);
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        print!("{}", render_diff(&result));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// A fixed-width table with a header row and a rule under it; trailing
/// spaces are trimmed from every row.
fn render_table(headers: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    render_row(&mut out, &widths, headers.iter().copied());
    let rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
    render_row(&mut out, &widths, rule.iter().map(String::as_str));
    for row in rows {
        render_row(&mut out, &widths, row.iter().map(String::as_str));
    }
    out
}

fn render_row<'c>(out: &mut String, widths: &[usize], cells: impl Iterator<Item = &'c str>) {
    let mut first = true;
    for (w, cell) in widths.iter().zip(cells) {
        if !first {
            out.push_str("  ");
        }
        first = false;
        let _ = write!(out, "{cell:<w$}", w = *w);
    }
    let end = out.trim_end_matches(' ').len();
    out.truncate(end);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    const V64: SpecVersion = SpecVersion::electronic(6, 4);
    const V70: SpecVersion = SpecVersion::electronic(7, 0);
    const V85: SpecVersion = SpecVersion::electronic(8, 5);

    /// Hand-built layouts so the diff logic is tested independently of the
    /// bundled data. `by_name` must list field indices sorted by name.
    static OLD: Layout = Layout {
        table: Table::SchA,
        versions: &[V70],
        fields: &[
            FieldDef {
                name: "form_type",
                column: 0,
            },
            FieldDef {
                name: "contributor_name",
                column: 1,
            },
            FieldDef {
                name: "contribution_amount",
                column: 2,
            },
            FieldDef {
                name: "memo_code",
                column: 3,
            },
        ],
        width: 4,
        by_name: &[2, 1, 0, 3],
    };

    static NEW: Layout = Layout {
        table: Table::SchA,
        versions: &[V85],
        fields: &[
            FieldDef {
                name: "form_type",
                column: 0,
            },
            FieldDef {
                name: "contributor_organization_name",
                column: 1,
            },
            FieldDef {
                name: "contributor_last_name",
                column: 2,
            },
            FieldDef {
                name: "contribution_amount",
                column: 3,
            },
            FieldDef {
                name: "memo_code",
                column: 4,
            },
        ],
        width: 5,
        by_name: &[3, 2, 1, 0, 4],
    };

    #[test]
    fn hand_built_layouts_look_themselves_up() {
        for l in [&OLD, &NEW] {
            for (i, f) in l.fields.iter().enumerate() {
                assert_eq!(l.index_of(f.name), Some(i), "{}", f.name);
            }
        }
    }

    #[test]
    fn diff_reports_added_removed_and_moved_with_one_based_columns() {
        let d = diff_layouts(&OLD, &NEW);
        assert_eq!(d.table, Table::SchA);
        assert_eq!((d.from_width, d.to_width), (4, 5));
        assert_eq!(
            d.changes,
            vec![
                FieldChange::Added {
                    name: "contributor_organization_name",
                    column: 2,
                },
                FieldChange::Added {
                    name: "contributor_last_name",
                    column: 3,
                },
                FieldChange::Removed {
                    name: "contributor_name",
                    column: 2,
                },
                FieldChange::Moved {
                    name: "contribution_amount",
                    from: 3,
                    to: 4,
                },
                FieldChange::Moved {
                    name: "memo_code",
                    from: 4,
                    to: 5,
                },
            ]
        );
        assert!(!d.is_empty());
    }

    #[test]
    fn diff_is_directional_and_empty_against_itself() {
        let back = diff_layouts(&NEW, &OLD);
        assert!(back.changes.contains(&FieldChange::Added {
            name: "contributor_name",
            column: 2,
        }));
        assert!(back.changes.contains(&FieldChange::Removed {
            name: "contributor_last_name",
            column: 3,
        }));
        assert!(diff_layouts(&OLD, &OLD).is_empty());
        assert!(diff_layouts(&NEW, &NEW).is_empty());
    }

    #[test]
    fn field_change_renders_like_the_help_text_says() {
        let d = diff_layouts(&OLD, &NEW);
        let rendered: Vec<String> = d.changes.iter().map(ToString::to_string).collect();
        assert_eq!(
            rendered,
            [
                "+contributor_organization_name@2",
                "+contributor_last_name@3",
                "-contributor_name@2",
                "contribution_amount 3->4",
                "memo_code 4->5",
            ]
        );
    }

    #[test]
    fn real_schedule_a_changed_between_7_0_and_8_5() {
        // Spec 8.0 dropped `contribution_purpose_code` (column 23, 1-based)
        // and shifted everything after it left by one.
        let from = Table::SchA.layout(V70).expect("SchA at 7.0");
        let to = Table::SchA.layout(V85).expect("SchA at 8.5");
        let d = diff_layouts(from, to);
        assert!(d.changes.contains(&FieldChange::Removed {
            name: "contribution_purpose_code",
            column: 23,
        }));
        assert!(d.changes.contains(&FieldChange::Moved {
            name: "contributor_employer",
            from: 25,
            to: 24,
        }));
        assert!(
            !d.changes
                .iter()
                .any(|c| matches!(c, FieldChange::Added { .. })),
            "{:?}",
            d.changes
        );
        assert!(!d.changes.iter().any(|c| matches!(
            c,
            FieldChange::Moved {
                name: "contribution_amount",
                ..
            }
        )));
        // 6.4 and 7.0 share a bucket, so there is nothing to report.
        assert!(diff_versions(&[Table::SchA], V64, V70).changed.is_empty());
    }

    #[test]
    fn diff_versions_sorts_tables_into_gained_lost_changed_unchanged() {
        let d = diff_versions(Table::ALL, V70, V85);
        assert_eq!(d.column_base, 1);
        let changed: Vec<Table> = d.changed.iter().map(|c| c.table).collect();
        assert!(changed.contains(&Table::SchA));
        // Schedule I was dropped in 8.5.
        assert!(
            d.lost.iter().any(|t| t.table == Table::SchI),
            "{:?}",
            d.lost
        );
        // TEXT and HDR share one bucket across 7.0 and 8.5.
        assert!(d.unchanged.contains(&Table::Text));
        assert!(d.unchanged.contains(&Table::Hdr));
        // F2S (authored locally) has layouts from 6.1 on, so it is gained
        // going from the 5.x era to the 6.x era and lost going back.
        let v53 = SpecVersion::electronic(5, 3);
        let v61 = SpecVersion::electronic(6, 1);
        let forward = diff_versions(&[Table::F2S], v53, v61);
        assert_eq!(forward.gained.len(), 1);
        assert_eq!(forward.gained.first().map(|g| g.table), Some(Table::F2S));
        assert!(forward.lost.is_empty() && forward.changed.is_empty());
        let back = diff_versions(&[Table::F2S], v61, v53);
        assert_eq!(back.lost.first().map(|l| l.table), Some(Table::F2S));
        assert!(back.gained.is_empty());
        for t in Table::ALL {
            let places = usize::from(changed.contains(t))
                + usize::from(d.unchanged.contains(t))
                + usize::from(d.gained.iter().any(|g| g.table == *t))
                + usize::from(d.lost.iter().any(|l| l.table == *t));
            assert!(places <= 1, "{t} appears in more than one bucket");
        }
        let text = render_diff(&d);
        assert!(text.starts_with("7.0 -> 8.5\n"));
        assert!(text.contains("SchA: "));
        assert!(text.contains("-contribution_purpose_code@23"));
        assert!(text.contains("SchI: lost its layout"));
    }

    #[test]
    fn diff_json_names_tables_the_fec_way() {
        let d = diff_versions(&[Table::Text, Table::Hdr], V70, V85);
        let json = serde_json::to_value(&d).unwrap();
        let unchanged = json["unchanged"].as_array().unwrap();
        assert!(unchanged.iter().any(|v| v == "TEXT"), "{unchanged:?}");
        assert!(unchanged.iter().any(|v| v == "HDR"), "{unchanged:?}");
        assert_eq!(json["from"], "7.0");
        assert_eq!(json["to"], "8.5");

        let d = diff_versions(&[Table::SchI], V70, V85);
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["lost"][0]["table"], "SchI");

        let d = diff_versions(&[Table::SchA], V70, V85);
        let json = serde_json::to_value(&d).unwrap();
        assert_eq!(json["changed"][0]["table"], "SchA");
        let first = &json["changed"][0]["changes"][0];
        assert_eq!(first["change"], "removed");
        assert_eq!(first["name"], "contribution_purpose_code");
        assert_eq!(first["column"], 23);
    }

    #[test]
    fn parse_table_accepts_names_case_insensitively() {
        assert_eq!(parse_table("SchA").unwrap(), Table::SchA);
        assert_eq!(parse_table("scha").unwrap(), Table::SchA);
        assert_eq!(parse_table(" f3x ").unwrap(), Table::F3X);
        assert_eq!(parse_table("TEXT").unwrap(), Table::Text);
        assert_eq!(parse_table("hdr").unwrap(), Table::Hdr);
    }

    #[test]
    fn parse_table_rejects_unknown_names_and_lists_valid_ones() {
        let err = parse_table("Schedule A").unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.starts_with("'Schedule A' is not a table name."),
            "{msg}"
        );
        assert!(msg.contains("SchA"), "{msg}");
        assert!(msg.contains("TEXT"), "{msg}");
        assert!(!msg.contains("form-type token"), "{msg}");
    }

    #[test]
    fn parse_table_hints_at_the_table_for_a_form_type_token() {
        let msg = parse_table("SA11AI").unwrap_err().to_string();
        assert!(
            msg.contains("A filing line with form type 'SA11AI' is parsed with table SchA"),
            "{msg}"
        );
        let msg = parse_table("F3XN").unwrap_err().to_string();
        assert!(msg.contains("is parsed with table F3X"), "{msg}");
        // A near-miss on a table name must not be silently dispatched to
        // the wrong schedule, only hinted at -- and still rejected.
        let msg = parse_table("ScheA").unwrap_err().to_string();
        assert!(msg.contains("is parsed with table SchC"), "{msg}");
        assert!(msg.starts_with("'ScheA' is not a table name."), "{msg}");
        // Too long or containing whitespace: not a token, no hint.
        let msg = parse_table("SA11AI-extra").unwrap_err().to_string();
        assert!(!msg.contains("form type"), "{msg}");
    }

    #[test]
    fn layout_lookup_errors_list_the_versions_a_table_has() {
        let v99 = SpecVersion::electronic(9, 9);
        let err = layout_or_error(Table::SchA, v99).unwrap_err();
        assert!(matches!(
            err,
            SpecError::NoLayout {
                table: Table::SchA,
                version,
                ..
            } if version == v99
        ));
        let msg = err.to_string();
        assert!(
            msg.starts_with("SchA has no column layout for spec version 9.9"),
            "{msg}"
        );
        assert!(msg.contains("8.5"), "{msg}");
        assert!(msg.contains("3.0"), "{msg}");

        // Schedule I exists but not at 8.5: the message must say what it
        // does have.
        let msg = layout_or_error(Table::SchI, V85).unwrap_err().to_string();
        assert!(
            msg.contains("SchI has no column layout for spec version 8.5"),
            "{msg}"
        );
        assert!(msg.contains("8.4"), "{msg}");

        assert!(layout_or_error(Table::SchA, V85).is_ok());
    }

    #[test]
    fn unknown_versions_are_rejected_before_diffing() {
        let v99 = SpecVersion::electronic(9, 9);
        let msg = known_version_or_error(v99).unwrap_err().to_string();
        assert!(
            msg.starts_with("no table has a column layout for spec version 9.9"),
            "{msg}"
        );
        assert!(msg.contains("8.5"), "{msg}");
        assert_eq!(known_version_or_error(V85).unwrap(), V85);
        assert_eq!(
            known_version_or_error("3.00".parse().unwrap()).unwrap(),
            SpecVersion::electronic(3, 0)
        );
        assert!(known_version_or_error(SpecVersion::paper(3, 4)).is_ok());
    }

    #[test]
    fn bundled_version_parses() {
        assert_eq!(bundled_version().unwrap(), V85);
    }

    #[test]
    fn field_rows_are_in_column_order_with_spec_data_joined() {
        let layout = Table::SchA.layout(V85).unwrap();
        let rows = field_rows(layout);
        assert_eq!(rows.len(), layout.fields.len());
        assert!(rows.windows(2).all(|w| w[0].column < w[1].column));
        let first = &rows[0];
        assert_eq!((first.column, first.name), (1, "form_type"));
        assert_eq!(first.description, Some("FORM TYPE"));
        assert_eq!(first.required, Some(Requirement::Error));
        let amount = rows
            .iter()
            .find(|r| r.name == "contribution_amount")
            .unwrap();
        assert_eq!(amount.column, 21);
        assert_eq!(amount.kind, Some(FieldKind::Amount));

        // A version the spec rows do not describe still gets columns; the
        // spec join is by canonical name so descriptions still appear.
        let old = field_rows(Table::SchA.layout(V70).unwrap());
        let employer = old
            .iter()
            .find(|r| r.name == "contributor_employer")
            .unwrap();
        assert_eq!(employer.column, 25);
        assert!(employer.description.is_some());
        // ...but a field the current spec dropped has no spec data.
        let purpose = old
            .iter()
            .find(|r| r.name == "contribution_purpose_code")
            .unwrap();
        assert_eq!(purpose.description, None);
    }

    #[test]
    fn table_rows_cover_every_table_with_spec_gaps_visible() {
        let rows: Vec<TableRow> = Table::ALL.iter().map(|&t| table_row(t)).collect();
        assert_eq!(rows.len(), Table::ALL.len());
        let sch_i = rows.iter().find(|r| r.table == "SchI").unwrap();
        assert_eq!(sch_i.spec_rows, 0);
        assert_eq!(sch_i.newest_version, Some(SpecVersion::electronic(8, 4)));
        let sch_a = rows.iter().find(|r| r.table == "SchA").unwrap();
        assert!(sch_a.spec_rows > 0);
        // Newest counts electronic versions only; paper conversions (which
        // sort after every electronic version) are counted separately.
        assert_eq!(sch_a.newest_version, Some(V85));
        assert!(sch_a.paper_versions > 0);
        assert!(sch_a.version_buckets >= 5);
    }

    #[test]
    fn export_is_self_describing_and_complete() {
        let doc = SpecExport {
            bundled_spec_version: BUNDLED_SPEC_VERSION,
            column_base: 0,
            tables: Table::ALL.iter().map(|&t| export_table(t, None)).collect(),
        };
        let json = serde_json::to_value(&doc).unwrap();
        assert_eq!(json["column_base"], 0);
        assert_eq!(json["bundled_spec_version"], "8.5");
        let tables = json["tables"].as_array().unwrap();
        assert_eq!(tables.len(), Table::ALL.len());
        let sch_a = tables.iter().find(|t| t["table"] == "SchA").unwrap();
        let layouts = sch_a["layouts"].as_array().unwrap();
        assert_eq!(layouts.len(), Table::SchA.layouts().len());
        assert!(
            layouts[0]["versions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == "8.5")
        );
        assert_eq!(layouts[0]["fields"][0]["column"], 0);
        assert_eq!(sch_a["specs"][0]["column"], 0);
        assert_eq!(sch_a["specs"][0]["kind"], "alpha_numeric");
    }

    #[test]
    fn export_selection_by_table_and_version() {
        assert_eq!(selected_tables(None, None).unwrap(), Table::ALL.to_vec());
        assert_eq!(
            selected_tables(Some(Table::SchA), None).unwrap(),
            vec![Table::SchA]
        );
        let at_85 = selected_tables(None, Some(V85)).unwrap();
        assert!(at_85.contains(&Table::SchA));
        assert!(!at_85.contains(&Table::SchI));
        assert!(matches!(
            selected_tables(Some(Table::SchI), Some(V85)),
            Err(SpecError::NoLayout { .. })
        ));
        assert!(matches!(
            selected_tables(None, Some(SpecVersion::electronic(9, 9))),
            Err(SpecError::UnknownVersion { .. })
        ));

        // A version restricts the layouts to the one bucket covering it.
        let one = export_table(Table::SchA, Some(V70));
        assert_eq!(one.layouts.len(), 1);
        assert!(one.layouts[0].versions.contains(&V70));
        let all = export_table(Table::SchA, None);
        assert_eq!(all.layouts.len(), Table::SchA.layouts().len());
        // Bundled order: the current version's bucket first.
        assert!(all.layouts[0].versions.contains(&V85));
    }

    #[test]
    fn csv_rows_cover_every_field_of_every_bucket_with_spec_joined() {
        let rows = csv_rows(Table::ALL, None);
        let expected: usize = Table::ALL
            .iter()
            .flat_map(|t| t.layouts())
            .map(|l| l.fields.len())
            .sum();
        assert_eq!(rows.len(), expected);

        let sch_a: Vec<&CsvRow> = rows.iter().filter(|r| r.table == "SchA").collect();
        let amount = sch_a
            .iter()
            .find(|r| r.field == "contribution_amount" && r.versions.contains("8.5"))
            .unwrap();
        assert_eq!(amount.column, 21);
        assert_eq!(amount.r#type, "AMT-12");
        assert_eq!(amount.kind, "amount");
        assert_eq!(amount.max_len, Some(12));
        assert_eq!(amount.required_level, "warning");
        assert_eq!(amount.rules_version, "8.5");
        let filer_id = sch_a
            .iter()
            .find(|r| r.field == "filer_committee_id_number" && r.versions.contains("8.5"))
            .unwrap();
        assert_eq!(filer_id.required_level, "error");
        assert_eq!(filer_id.r#type, "A/N-9");
        assert_eq!(amount.forms, "F3|F3X|F3P|F3L");
        assert_eq!(amount.last_version, V85);
        assert!(amount.first_version <= V85);

        // Rule text is one line; multi-line workbook cells are collapsed.
        let form_type = sch_a
            .iter()
            .find(|r| r.field == "form_type" && r.versions.contains("8.5"))
            .unwrap();
        assert_eq!(form_type.rule, "Appendix C. SA3L must be used with the F3L");
        assert!(!form_type.rule.contains('\n'));

        // A field the current workbook dropped has columns but no rules.
        let purpose = sch_a
            .iter()
            .find(|r| r.field == "contribution_purpose_code" && r.versions.contains("7.0"))
            .unwrap();
        assert_eq!(purpose.column, 23);
        assert_eq!(purpose.rules_version, "");
        assert_eq!(purpose.r#type, "");
        assert_eq!(purpose.required_level, "");

        // Enumerations and patterns are carried verbatim, `|`-joined.
        let f3x_form_type = rows
            .iter()
            .find(|r| r.table == "F3X" && r.field == "form_type" && r.versions.contains("8.5"))
            .unwrap();
        assert_eq!(f3x_form_type.allowed_values, "F3XA|F3XN|F3XT");
        let filer = rows
            .iter()
            .find(|r| {
                r.table == "F3X"
                    && r.field == "filer_committee_id_number"
                    && r.versions.contains("8.5")
            })
            .unwrap();
        assert!(
            filer.pattern.starts_with("^[C|P][0-9]{8}$"),
            "{}",
            filer.pattern
        );

        // Filtering by version keeps only the buckets covering it.
        let at_70 = csv_rows(&[Table::SchA], Some(V70));
        assert_eq!(at_70.len(), Table::SchA.layout(V70).unwrap().fields.len());
        assert!(at_70.iter().all(|r| r.versions.contains("7.0")));

        // Rows serialise with a stable header.
        let mut w = csv::Writer::from_writer(Vec::new());
        w.serialize(&rows[0]).unwrap();
        let text = String::from_utf8(w.into_inner().unwrap()).unwrap();
        assert!(
            text.starts_with(
                "table,versions,first_version,last_version,width,column,field,rules_version,description,type,kind,max_len,required_level,required_condition,sample,value_reference,rule,forms,allowed_values,pattern\n"
            ),
            "{text}"
        );
    }

    #[test]
    fn render_table_aligns_and_trims() {
        let out = render_table(
            &["a", "bb"],
            &[
                vec!["1".to_string(), String::new()],
                vec!["long".to_string(), "x".to_string()],
            ],
        );
        assert_eq!(out, "a     bb\n----  --\n1\nlong  x\n");
        assert_eq!(
            one_line("Appendix C.  SA3L must be used \nwith the F3L"),
            "Appendix C. SA3L must be used with the F3L"
        );
    }
}
