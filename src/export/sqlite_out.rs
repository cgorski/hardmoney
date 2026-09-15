//! SQLite output: one database file, one table per [`Table`] (named like
//! the table: `SchA`, `F3X`, `TEXT`, ...) plus a `filings` table describing
//! each export.
//!
//! Every field column is `TEXT` holding the value exactly as filed.
//! SQLite has no decimal type, and storing an amount as `REAL` would round
//! it to the nearest binary fraction, so amounts are exact decimal strings
//! (`"15.00"`); `filing_id` and `line_no` are `INTEGER`. Tables are
//! created with `IF NOT EXISTS` and rows are appended, so several filings
//! can be exported into one database (with `filing_id` to tell them apart)
//! as long as their tables share a column layout; a filing whose layout has
//! a column the existing table lacks fails rather than silently dropping it.
//!
//! The whole export is one transaction: on any error the database is left
//! as it was.

use std::path::{Path, PathBuf};

use chrono::Utc;
use rusqlite::types::{ToSqlOutput, Value, ValueRef};
use rusqlite::{Connection, ToSql, params, params_from_iter};

use crate::parser::stream::Preamble;
use crate::parser::{ParsedLine, Table};

use super::{Columns, ExportError, Row, Sink, TableStats, io_at};

pub(crate) struct SqliteSink {
    conn: Connection,
    path: PathBuf,
    filing_id: Option<i64>,
    open: Vec<OpenTable>,
}

struct OpenTable {
    table: Table,
    insert_sql: String,
    rows: u64,
}

/// One bound parameter: an integer, or a text value borrowed from the
/// record (no copy per cell).
enum Cell<'a> {
    Int(i64),
    Text(&'a str),
}

impl ToSql for Cell<'_> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self {
            Cell::Int(i) => ToSqlOutput::Owned(Value::Integer(*i)),
            Cell::Text(s) => ToSqlOutput::Borrowed(ValueRef::Text(s.as_bytes())),
        })
    }
}

/// `"name"` -- SQLite identifier quoting (table names such as `TEXT` collide
/// with keywords otherwise).
fn quote(ident: &str) -> String {
    format!("\"{}\"", ident.replace('"', "\"\""))
}

impl SqliteSink {
    pub(crate) fn open(
        path: &Path,
        filing_id: Option<i64>,
        preamble: &Preamble,
        source: Option<&Path>,
    ) -> Result<Self, ExportError> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| io_at(parent, e))?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "BEGIN;
             CREATE TABLE IF NOT EXISTS filings (
                 filing_id    INTEGER,
                 form_type    TEXT NOT NULL,
                 version      TEXT NOT NULL,
                 committee_id TEXT,
                 path         TEXT,
                 exported_at  TEXT NOT NULL
             );",
        )?;
        let committee_id = preamble
            .summary
            .get_non_empty("filer_committee_id_number")
            .or_else(|| preamble.summary.get_non_empty("candidate_id_number"));
        conn.execute(
            "INSERT INTO filings (filing_id, form_type, version, committee_id, path, exported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                filing_id,
                preamble.raw_form_type,
                preamble.version.to_string(),
                committee_id,
                source.map(|p| p.display().to_string()),
                Utc::now().to_rfc3339(),
            ],
        )?;
        Ok(Self {
            conn,
            path: path.to_path_buf(),
            filing_id,
            open: Vec::new(),
        })
    }

    fn create_table(&self, table: Table, columns: &Columns) -> Result<String, ExportError> {
        let mut defs = Vec::with_capacity(columns.layout.fields.len() + 2);
        if columns.filing_id.is_some() {
            defs.push("\"filing_id\" INTEGER".to_string());
        }
        defs.push("\"line_no\" INTEGER NOT NULL".to_string());
        defs.extend(
            columns
                .layout
                .fields
                .iter()
                .map(|f| format!("{} TEXT", quote(f.name))),
        );
        self.conn.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {} ({})",
            quote(table.as_str()),
            defs.join(", ")
        ))?;

        let names: Vec<String> = columns.names().map(quote).collect();
        let placeholders = vec!["?"; names.len()].join(", ");
        Ok(format!(
            "INSERT INTO {} ({}) VALUES ({})",
            quote(table.as_str()),
            names.join(", "),
            placeholders
        ))
    }
}

/// Binds and runs one table's `INSERT` for `row`. The statement is cached
/// by its SQL text, so each table's is prepared once per export.
fn insert(conn: &Connection, insert_sql: &str, row: Row<'_>) -> Result<(), ExportError> {
    let cells = row
        .filing_id
        .map(Cell::Int)
        .into_iter()
        .chain(std::iter::once(Cell::Int(row.line_no())))
        .chain(row.line.iter().map(|(_, v)| Cell::Text(v)));
    conn.prepare_cached(insert_sql)?
        .execute(params_from_iter(cells))?;
    Ok(())
}

impl Sink for SqliteSink {
    fn write(&mut self, line: &ParsedLine) -> Result<(), ExportError> {
        let row = Row {
            filing_id: self.filing_id,
            line,
        };
        if let Some(open) = self.open.iter_mut().find(|o| o.table == line.table()) {
            insert(&self.conn, &open.insert_sql, row)?;
            open.rows += 1;
            return Ok(());
        }
        let columns = Columns {
            filing_id: self.filing_id,
            layout: line.layout(),
        };
        let insert_sql = self.create_table(line.table(), &columns)?;
        insert(&self.conn, &insert_sql, row)?;
        self.open.push(OpenTable {
            table: line.table(),
            insert_sql,
            rows: 1,
        });
        Ok(())
    }

    fn finish(self: Box<Self>) -> Result<(Vec<TableStats>, u64), ExportError> {
        self.conn.execute_batch("COMMIT")?;
        let stats = self
            .open
            .iter()
            .map(|o| TableStats {
                table: o.table,
                rows: o.rows,
                bytes: None,
            })
            .collect();
        drop(self.conn);
        let bytes = std::fs::metadata(&self.path)
            .map_err(|e| io_at(&self.path, e))?
            .len();
        Ok((stats, bytes))
    }
}
