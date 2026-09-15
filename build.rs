//! Build script: compiles the bundled FEC format data into Rust.
//!
//! Two inputs, one output:
//!
//! * `data/fec-csv-sources/*.csv` -- one file per format table (form,
//!   schedule, or record type). The header row lists version-bucket
//!   regexes; each body row is `canonical_name, <1-based column in bucket
//!   1>, <FEC label>, <column in bucket 2>, <label>, ...`. A blank column
//!   cell means the field does not exist in that bucket.
//! * `data/fec-spec/spec-*.json` -- the FEC's own field specification
//!   (type, max length, required level, rule text, allowed values) for the
//!   current spec version, distilled from the FEC's workbook by
//!   `scripts/distill_fec_spec.py`.
//!
//! Output: `$OUT_DIR/tables.rs`, `include!`d by `src/parser/tables.rs`. It
//! defines the `Table` enum and, per table, a module with a zero-sized
//! marker type, one `Field<Marker>` constant per canonical field, the
//! `Layout` for every version bucket, and the `FieldSpec` rows.
//!
//! Generating at build time (rather than checking in generated source)
//! means a stale table is impossible: touch a CSV and the next `cargo
//! build` picks it up. The generator has no knowledge of the crate's types
//! beyond their names; the hand-written half lives in
//! `src/parser/schema.rs`.
//!
//! Also emits `rerun-if-changed` for `migrations/` because `sqlx::migrate!`
//! embeds SQL at compile time and Cargo would otherwise not notice edits.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

fn main() {
    let manifest_dir = PathBuf::from(env_var("CARGO_MANIFEST_DIR"));
    let out_dir = PathBuf::from(env_var("OUT_DIR"));

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=migrations");
    println!("cargo:rerun-if-changed=data/fec-csv-sources");
    println!("cargo:rerun-if-changed=data/fec-spec");

    let csv_dir = manifest_dir.join("data/fec-csv-sources");
    let spec_path = latest_spec_json(&manifest_dir.join("data/fec-spec"));

    let tables = match load_tables(&csv_dir) {
        Ok(t) => t,
        Err(e) => fail(&format!(
            "loading format tables from {}: {e}",
            csv_dir.display()
        )),
    };
    let spec = match load_spec(&spec_path) {
        Ok(s) => s,
        Err(e) => fail(&format!("loading spec from {}: {e}", spec_path.display())),
    };

    let code = generate(&tables, &spec);
    let out = out_dir.join("tables.rs");
    if let Err(e) = std::fs::write(&out, code) {
        fail(&format!("writing {}: {e}", out.display()));
    }
}

fn env_var(name: &str) -> String {
    match std::env::var(name) {
        Ok(v) => v,
        Err(_) => fail(&format!("environment variable {name} is not set")),
    }
}

fn fail(msg: &str) -> ! {
    // Build scripts communicate failures through a non-zero exit; a panic
    // would print an unhelpful backtrace pointer first.
    eprintln!("error: {msg}");
    std::process::exit(1)
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// One format table (one CSV): its name and version buckets.
struct TableDef {
    /// CSV stem, e.g. `F3X`, `SchA`, `TEXT`.
    name: String,
    /// Canonical field names in CSV row order (the union over buckets).
    fields: Vec<String>,
    buckets: Vec<BucketDef>,
}

struct BucketDef {
    /// The header-row regex, kept for documentation.
    pattern: String,
    /// Every `(paper, major, minor)` version the pattern matches.
    versions: Vec<Version>,
    /// canonical name -> 0-based column.
    columns: BTreeMap<String, u16>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    paper: bool,
    major: u8,
    minor: u8,
}

impl Version {
    fn label(self) -> String {
        format!(
            "{}{}.{}",
            if self.paper { "P" } else { "" },
            self.major,
            self.minor
        )
    }
}

/// Every two-component version the FEC could plausibly have published,
/// used to turn a bucket regex into an explicit version set at build time.
fn version_universe() -> Vec<Version> {
    let mut v = Vec::new();
    for paper in [false, true] {
        for major in 1..=9u8 {
            for minor in 0..=9u8 {
                v.push(Version {
                    paper,
                    major,
                    minor,
                });
            }
        }
    }
    v
}

fn load_tables(dir: &Path) -> Result<Vec<TableDef>, String> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| e.to_string())?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "csv"))
        .collect();
    paths.sort();
    if paths.is_empty() {
        return Err("no CSV files found".into());
    }
    let universe = version_universe();
    paths
        .iter()
        .map(|p| load_table(p, &universe).map_err(|e| format!("{}: {e}", p.display())))
        .collect()
}

fn load_table(path: &Path, universe: &[Version]) -> Result<TableDef, String> {
    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or("bad file name")?
        .to_string();
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_path(path)
        .map_err(|e| e.to_string())?;
    let mut records = reader.records();
    let header = records
        .next()
        .ok_or("empty file")?
        .map_err(|e| e.to_string())?;

    struct Pending {
        header_col: usize,
        pattern: String,
        regex: regex::Regex,
        columns: BTreeMap<String, u16>,
    }
    let mut pending: Vec<Pending> = Vec::new();
    for (i, cell) in header.iter().enumerate() {
        let cell = cell.trim();
        if cell.is_empty() || cell == "canonical" {
            continue;
        }
        let regex = regex::Regex::new(&format!("^(?:{cell})"))
            .map_err(|e| format!("bucket regex {cell:?}: {e}"))?;
        pending.push(Pending {
            header_col: i,
            pattern: cell.to_string(),
            regex,
            columns: BTreeMap::new(),
        });
    }

    let mut fields: Vec<String> = Vec::new();
    for row in records {
        let row = row.map_err(|e| e.to_string())?;
        let Some(canonical) = row.get(0).map(str::trim).filter(|c| !c.is_empty()) else {
            continue;
        };
        if !is_snake_ident(canonical) {
            return Err(format!(
                "canonical name {canonical:?} is not lower_snake_case"
            ));
        }
        if !fields.iter().any(|f| f == canonical) {
            fields.push(canonical.to_string());
        }
        for b in &mut pending {
            let cell = row.get(b.header_col).map(str::trim).unwrap_or("");
            let pos = match parse_column_position(cell) {
                Position::Absent => continue,
                Position::At(p) => p,
                Position::Invalid => {
                    return Err(format!(
                        "field {canonical:?} has non-numeric position {cell:?} in bucket {:?} (an unquoted comma in a label shifts the row -- quote it)",
                        b.pattern
                    ));
                }
            };
            let zero_based = u16::try_from(pos - 1).map_err(|_| "column position overflow")?;
            match b.columns.get(canonical) {
                Some(&existing) if existing != zero_based => {
                    return Err(format!(
                        "canonical field {canonical:?} is assigned to two columns ({} and {pos}) in bucket {:?}",
                        existing + 1,
                        b.pattern
                    ));
                }
                Some(_) => {}
                None => {
                    b.columns.insert(canonical.to_string(), zero_based);
                }
            }
        }
    }

    // Two fields must never share a column within one bucket either.
    for b in &pending {
        let mut seen: BTreeMap<u16, &str> = BTreeMap::new();
        for (name, &col) in &b.columns {
            if let Some(other) = seen.insert(col, name) {
                return Err(format!(
                    "fields {other:?} and {name:?} both occupy column {} in bucket {:?}",
                    col + 1,
                    b.pattern
                ));
            }
        }
    }

    // Each version belongs to the *first* bucket whose pattern matches it
    // (the CSV lists buckets newest-first), so version sets never overlap.
    let mut claimed: BTreeSet<Version> = BTreeSet::new();
    let buckets: Vec<BucketDef> = pending
        .into_iter()
        .map(|b| {
            let versions: Vec<Version> = universe
                .iter()
                .copied()
                .filter(|v| !claimed.contains(v) && b.regex.is_match(&v.label()))
                .collect();
            claimed.extend(versions.iter().copied());
            BucketDef {
                versions,
                pattern: b.pattern,
                columns: b.columns,
            }
        })
        .collect();

    // A row with no position in any bucket documents a field the FEC once
    // listed but no bundled version places; it cannot be parsed or written,
    // so it gets no constant.
    fields.retain(|f| buckets.iter().any(|b| b.columns.contains_key(f)));

    Ok(TableDef {
        name,
        fields,
        buckets,
    })
}

enum Position {
    /// Blank or `0`: the field does not exist in this bucket.
    Absent,
    /// 1-based column.
    At(usize),
    /// Not a number: almost always a label cell shifted left by an
    /// unquoted comma in the previous label.
    Invalid,
}

/// Column cells are usually integers but several spreadsheet-exported
/// tables carry `7.0`; both mean column 7. `0` means "not present".
fn parse_column_position(cell: &str) -> Position {
    let cell = cell.trim();
    if cell.is_empty() {
        return Position::Absent;
    }
    let whole = match cell.split_once('.') {
        Some((w, frac)) if !frac.is_empty() && frac.chars().all(|c| c == '0') => w,
        Some(_) => return Position::Invalid,
        None => cell,
    };
    match whole.parse::<usize>() {
        Ok(0) => Position::Absent,
        Ok(n) => Position::At(n),
        Err(_) => Position::Invalid,
    }
}

fn is_snake_ident(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// One row of the distilled FEC spec.
struct SpecRow {
    column: u16,
    description: String,
    kind: String,
    max_len: Option<u16>,
    required_level: String,
    required_condition: Option<String>,
    sample: Option<String>,
    value_reference: Option<String>,
    rule: Option<String>,
    forms: Vec<String>,
    allowed_values: Vec<String>,
    pattern: Option<String>,
}

struct Spec {
    version: String,
    tables: BTreeMap<String, Vec<SpecRow>>,
}

fn latest_spec_json(dir: &Path) -> PathBuf {
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("spec-") && n.ends_with(".json"))
        })
        .collect();
    candidates.sort();
    match candidates.pop() {
        Some(p) => p,
        None => fail(&format!("no spec-*.json in {}", dir.display())),
    }
}

fn load_spec(path: &Path) -> Result<Spec, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let doc: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let version = doc
        .get("spec_version")
        .and_then(|v| v.as_str())
        .ok_or("missing spec_version")?
        .to_string();
    let mut tables = BTreeMap::new();
    let table_obj = doc
        .get("tables")
        .and_then(|t| t.as_object())
        .ok_or("missing tables")?;
    for (name, rows) in table_obj {
        let rows = rows.as_array().ok_or("table rows must be an array")?;
        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let column = r
                .get("column")
                .and_then(|c| c.as_u64())
                .and_then(|c| u16::try_from(c).ok())
                .ok_or_else(|| format!("{name}: bad column"))?;
            let s = |k: &str| r.get(k).and_then(|v| v.as_str()).map(str::to_string);
            let required = r.get("required").cloned().unwrap_or_default();
            out.push(SpecRow {
                column,
                description: s("description").unwrap_or_default(),
                kind: s("kind").unwrap_or_else(|| "unknown".into()),
                max_len: r
                    .get("max_len")
                    .and_then(|v| v.as_u64())
                    .and_then(|v| u16::try_from(v).ok()),
                required_level: required
                    .get("level")
                    .and_then(|v| v.as_str())
                    .unwrap_or("none")
                    .to_string(),
                required_condition: required
                    .get("condition")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                sample: s("sample"),
                value_reference: s("value_reference"),
                rule: s("rule"),
                forms: str_array(r.get("forms")),
                allowed_values: str_array(r.get("allowed_values")),
                pattern: s("pattern"),
            });
        }
        out.sort_by_key(|r| r.column);
        tables.insert(name.clone(), out);
    }
    Ok(Spec { version, tables })
}

fn str_array(v: Option<&serde_json::Value>) -> Vec<String> {
    v.and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// Code generation
// ---------------------------------------------------------------------------

/// `F3X` -> `F3X`, `SchA3L` -> `SchA3L`, `TEXT` -> `Text`, `HDR` -> `Hdr`.
fn variant_name(table: &str) -> String {
    if table.chars().all(|c| c.is_ascii_uppercase()) {
        let mut cs = table.chars();
        match cs.next() {
            Some(first) => first.to_string() + &cs.as_str().to_ascii_lowercase(),
            None => String::new(),
        }
    } else {
        table
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect()
    }
}

/// `F3X` -> `f3x`, `SchA3L` -> `sch_a3l`, `TEXT` -> `text`, `H1` -> `h1`.
fn module_name(table: &str) -> String {
    let mut out = String::new();
    let chars: Vec<char> = table.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii_uppercase() && i > 0 && chars[i - 1].is_ascii_lowercase() {
            out.push('_');
        }
        out.push(c.to_ascii_lowercase());
    }
    out
}

/// `contribution_amount` -> `CONTRIBUTION_AMOUNT`;
/// `11_a_i_individuals_itemized` -> `LINE_11_A_I_INDIVIDUALS_ITEMIZED`.
fn const_name(canonical: &str) -> String {
    let upper = canonical.to_ascii_uppercase();
    if upper.starts_with(|c: char| c.is_ascii_digit()) {
        format!("LINE_{upper}")
    } else {
        upper
    }
}

fn rust_str(s: &str) -> String {
    format!("{s:?}")
}

fn rust_opt_str(s: Option<&str>) -> String {
    match s {
        Some(s) => format!("Some({})", rust_str(s)),
        None => "None".to_string(),
    }
}

fn rust_str_slice(items: &[String]) -> String {
    let mut out = String::from("&[");
    for (i, s) in items.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&rust_str(s));
    }
    out.push(']');
    out
}

fn generate(tables: &[TableDef], spec: &Spec) -> String {
    let mut o = String::new();
    let _ = writeln!(
        o,
        "// GENERATED by build.rs from data/fec-csv-sources/*.csv and data/fec-spec/spec-{}.json.\n\
         // Do not edit; edit the data files and rebuild.\n",
        spec.version
    );
    let _ = writeln!(
        o,
        "/// The FEC spec version whose field specifications are bundled (see [`FieldSpec`]).\n\
         pub const BUNDLED_SPEC_VERSION: &str = {};\n",
        rust_str(&spec.version)
    );

    // --- Table enum -------------------------------------------------------
    o.push_str(
        "/// One FEC format table: a form, schedule, or record type with its own\n\
         /// column layout per spec version.\n\
         ///\n\
         /// This is the type-level identity of a table. [`crate::parser::ParsedLine::table`] carries\n\
         /// it so callers match on a closed set instead of comparing strings, and\n\
         /// typed views use it to refuse lines from the wrong table.\n\
         /// `Display`/`FromStr` use the FEC-style name (`\"F3X\"`, `\"SchA\"`, `\"TEXT\"`).\n\
         ///\n\
         /// `#[non_exhaustive]`: the FEC adds record types over time.\n\
         #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]\n\
         #[derive(strum::Display, strum::EnumString, strum::EnumIter, strum::IntoStaticStr)]\n\
         #[strum(ascii_case_insensitive)]\n\
         #[cfg_attr(feature = \"serde\", derive(serde::Serialize, serde::Deserialize))]\n\
         #[non_exhaustive]\n\
         pub enum Table {\n",
    );
    for t in tables {
        let _ = writeln!(
            o,
            "    /// `{name}` -- {n} version bucket(s), {f} canonical field(s).\n    #[strum(serialize = {name_q})]\n    {var},",
            name = t.name,
            name_q = rust_str(&t.name),
            n = t.buckets.len(),
            f = t.fields.len(),
            var = variant_name(&t.name)
        );
    }
    o.push_str("}\n\n");

    // --- Table methods ----------------------------------------------------
    o.push_str("impl Table {\n");
    o.push_str("    /// Every bundled table, in name order.\n    pub const ALL: &[Table] = &[\n");
    for t in tables {
        let _ = writeln!(o, "        Table::{},", variant_name(&t.name));
    }
    o.push_str("    ];\n\n");

    o.push_str(
        "    /// The FEC-style table name, e.g. `\"F3X\"` or `\"SchA\"`.\n    pub const fn as_str(self) -> &'static str {\n        match self {\n",
    );
    for t in tables {
        let _ = writeln!(
            o,
            "            Table::{} => {},",
            variant_name(&t.name),
            rust_str(&t.name)
        );
    }
    o.push_str("        }\n    }\n\n");

    o.push_str(
        "    /// Every version-bucketed column layout for this table.\n    pub fn layouts(self) -> &'static [Layout] {\n        match self {\n",
    );
    for t in tables {
        let _ = writeln!(
            o,
            "            Table::{} => &{}::LAYOUTS,",
            variant_name(&t.name),
            module_name(&t.name)
        );
    }
    o.push_str("        }\n    }\n\n");

    o.push_str(
        "    /// The FEC's field specifications for this table at\n    /// [`BUNDLED_SPEC_VERSION`], in column order. Empty for tables the current\n    /// spec no longer documents (historical forms, Schedule I).\n    pub fn specs(self) -> &'static [FieldSpec] {\n        match self {\n",
    );
    for t in tables {
        let _ = writeln!(
            o,
            "            Table::{} => &{}::SPECS,",
            variant_name(&t.name),
            module_name(&t.name)
        );
    }
    o.push_str("        }\n    }\n\n");

    o.push_str(
        "    /// Canonical field names known for this table across all versions, in\n    /// table order.\n    pub fn field_names(self) -> &'static [&'static str] {\n        match self {\n",
    );
    for t in tables {
        let _ = writeln!(
            o,
            "            Table::{} => {}::FIELD_NAMES,",
            variant_name(&t.name),
            module_name(&t.name)
        );
    }
    o.push_str("        }\n    }\n}\n\n");

    // --- Per-table modules -------------------------------------------------
    for t in tables {
        generate_table_module(&mut o, t, spec.tables.get(&t.name).map(Vec::as_slice));
    }

    // --- Marker re-exports -------------------------------------------------
    o.push_str("/// Zero-sized marker types, one per [`Table`], for compile-time-checked field access.\npub mod markers {\n");
    for t in tables {
        let _ = writeln!(
            o,
            "    pub use super::{}::{};",
            module_name(&t.name),
            variant_name(&t.name)
        );
    }
    o.push_str("}\n");
    o
}

fn generate_table_module(o: &mut String, t: &TableDef, spec_rows: Option<&[SpecRow]>) {
    let var = variant_name(&t.name);
    let module = module_name(&t.name);
    let _ = writeln!(
        o,
        "/// Fields, layouts, and specifications for table `{}`.\n#[allow(non_upper_case_globals, clippy::doc_markdown)]\npub mod {module} {{\n    use super::*;\n",
        t.name
    );
    let _ = writeln!(
        o,
        "    /// Marker type for [`Table::{var}`]; see [`Field`] and [`crate::parser::schema::Typed`].\n    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]\n    pub struct {var};\n    impl TableMarker for {var} {{\n        const TABLE: Table = Table::{var};\n    }}\n"
    );

    // Field constants.
    let mut used: BTreeSet<String> = BTreeSet::new();
    for f in &t.fields {
        let c = const_name(f);
        if !used.insert(c.clone()) {
            fail(&format!(
                "{}: constant name collision for field {f:?}",
                t.name
            ));
        }
        let label = spec_rows
            .and_then(|rows| {
                let latest = t.buckets.iter().find(|b| {
                    b.versions
                        .iter()
                        .any(|v| !v.paper && v.major == 8 && v.minor == 5)
                })?;
                let col = *latest.columns.get(f)?;
                rows.iter().find(|r| r.column == col + 1)
            })
            .map(|r| r.description.clone());
        match label {
            Some(l) if !l.is_empty() => {
                let _ = writeln!(o, "    /// `{f}` -- FEC: \"{}\".", l.replace('`', "'"));
            }
            _ => {
                let _ = writeln!(o, "    /// `{f}`.");
            }
        }
        let _ = writeln!(
            o,
            "    pub const {c}: Field<{var}> = Field::new({});",
            rust_str(f)
        );
    }
    o.push('\n');

    // FIELD_NAMES.
    let _ = writeln!(
        o,
        "    /// Every canonical field name for this table (all versions), in table order.\n    pub const FIELD_NAMES: &[&str] = {};\n",
        rust_str_slice(&t.fields)
    );

    // Layouts.
    let _ = writeln!(
        o,
        "    /// Column layouts, one per version bucket, in the order the source table lists them.\n    pub static LAYOUTS: [Layout; {}] = [",
        t.buckets.len()
    );
    for b in &t.buckets {
        // Fields in table order that exist in this bucket.
        let present: Vec<(&String, u16)> = t
            .fields
            .iter()
            .filter_map(|f| b.columns.get(f).map(|&c| (f, c)))
            .collect();
        let max_column = present.iter().map(|(_, c)| *c).max().map_or(0, |c| c + 1);
        // Indexes into `fields` sorted by name, for binary search.
        let mut by_name: Vec<u16> = (0..present.len() as u16).collect();
        by_name.sort_by(|&a, &b2| present[a as usize].0.cmp(present[b2 as usize].0));

        let versions: Vec<String> = b
            .versions
            .iter()
            .map(|v| {
                format!(
                    "SpecVersion::{}({}, {})",
                    if v.paper { "paper" } else { "electronic" },
                    v.major,
                    v.minor
                )
            })
            .collect();
        let _ = writeln!(
            o,
            "        // bucket pattern: {}\n        Layout {{\n            table: Table::{var},\n            versions: &[{}],\n            fields: &[",
            b.pattern.replace("*/", "* /"),
            versions.join(", ")
        );
        for (f, c) in &present {
            let _ = writeln!(
                o,
                "                FieldDef {{ name: {}, column: {c} }},",
                rust_str(f)
            );
        }
        let _ = writeln!(
            o,
            "            ],\n            width: {max_column},\n            by_name: &[{}],\n        }},",
            by_name
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    o.push_str("    ];\n\n");

    // Specs.
    let rows = spec_rows.unwrap_or(&[]);
    let latest_bucket = t.buckets.iter().find(|b| {
        b.versions
            .iter()
            .any(|v| !v.paper && v.major == 8 && v.minor == 5)
    });
    let _ = writeln!(
        o,
        "    /// The FEC's field specifications for this table at the bundled spec version.\n    pub static SPECS: [FieldSpec; {}] = [",
        rows.len()
    );
    for r in rows {
        let canonical = latest_bucket.and_then(|b| {
            b.columns
                .iter()
                .find(|&(_, &c)| c + 1 == r.column)
                .map(|(name, _)| name.as_str())
        });
        let kind = match r.kind.as_str() {
            "alpha" => "FieldKind::Alpha",
            "alphanumeric" => "FieldKind::AlphaNumeric",
            "numeric" => "FieldKind::Numeric",
            "amount" => "FieldKind::Amount",
            _ => "FieldKind::Unknown",
        };
        let required = match r.required_level.as_str() {
            "error" => "Requirement::Error".to_string(),
            "warning" => "Requirement::Warning".to_string(),
            "conditional" => format!(
                "Requirement::Conditional({})",
                rust_str(r.required_condition.as_deref().unwrap_or(""))
            ),
            _ => "Requirement::None".to_string(),
        };
        let _ = writeln!(
            o,
            "        FieldSpec {{\n            column: {},\n            canonical: {},\n            description: {},\n            kind: {kind},\n            max_len: {},\n            required: {required},\n            sample: {},\n            value_reference: {},\n            rule: {},\n            forms: {},\n            allowed_values: {},\n            pattern: {},\n        }},",
            r.column.saturating_sub(1),
            rust_opt_str(canonical),
            rust_str(&r.description),
            match r.max_len {
                Some(n) => format!("Some({n})"),
                None => "None".into(),
            },
            rust_opt_str(r.sample.as_deref()),
            rust_opt_str(r.value_reference.as_deref()),
            rust_opt_str(r.rule.as_deref()),
            rust_str_slice(&r.forms),
            rust_str_slice(&r.allowed_values),
            rust_opt_str(r.pattern.as_deref()),
        );
    }
    o.push_str("    ];\n}\n\n");
}
