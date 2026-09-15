//! Port of `pyfec/line.py`: version-aware column-position lookup driven by
//! the fec-csv-sources format tables (embedded at compile time, see
//! `format_data.rs`).
//!
//! CSV structure (unchanged from upstream): the header row is
//! `canonical,<regex1>,,<regex2>,,...` where each regex marks a "version
//! bucket" column. Body rows are `<canonical_field_name>,<1-indexed col for
//! regex1>,<FEC_LABEL>,<1-indexed col for regex2>,<FEC_LABEL>,...` with a
//! blank position meaning the field doesn't exist in that version.

use indexmap::IndexMap;
use regex::Regex;

use crate::parser::error::{FecError, Result};
use crate::parser::format_data::Table;
use crate::parser::utils::clean_entry;

/// One version-bucket: the compiled (start-anchored) regex that matches a
/// `fec_version` string, plus the 0-indexed column position for every
/// canonical field name available in that bucket.
struct Bucket {
    regex: Regex,
    /// Preserves the CSV's field order for deterministic iteration/output.
    columns: IndexMap<String, usize>,
}

/// A single form/schedule's format table across all FEC spec versions it has
/// ever used, e.g. "F3X" or "SchA".
#[derive(Debug)]
pub struct Line {
    table: Table,
    buckets: Vec<Bucket>,
}

impl std::fmt::Debug for Bucket {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Bucket")
            .field("regex", &self.regex.as_str())
            .field("columns", &self.columns.len())
            .finish()
    }
}

/// Python's `re.match` implicitly anchors at the start of the string even
/// when the pattern has no explicit `^`, because alternation branches are
/// each tried starting at position 0. Rust's `regex` crate performs an
/// unanchored search by default, so every pattern is wrapped in
/// `^(?:...)` to reproduce that semantics exactly.
fn compile_anchored(pattern: &str) -> Result<Regex> {
    let wrapped = format!("^(?:{})", pattern);
    Regex::new(&wrapped).map_err(|source| FecError::Regex {
        pattern: pattern.to_string(),
        source,
    })
}

/// Parses a 1-indexed column-position cell from a fec-csv-sources format
/// table into a `usize`.
///
/// Most cells are plain integers (`"7"`), but several tables --
/// `SchA.csv` and `SchB.csv` at minimum -- were exported from a
/// spreadsheet and contain float-formatted positions instead (`"7.0"`).
/// A naive `str::parse::<usize>()` silently rejects those cells (`Err`
/// is swallowed by the caller's `if let`), which drops every field past
/// the first affected row without any error -- exactly the kind of gap
/// that only shows up against real filings, not synthetic unit tests.
/// This parses the float form too, as long as it represents a whole
/// number >= 1.
fn parse_column_position(cell: &str) -> Option<usize> {
    let cell = cell.trim();
    if cell.is_empty() {
        return None;
    }
    if let Ok(n) = cell.parse::<usize>() {
        return (n >= 1).then_some(n);
    }
    if let Ok(f) = cell.parse::<f64>()
        && f >= 1.0
        && f.fract() == 0.0
    {
        return Some(f as usize);
    }
    None
}

/// A version bucket under construction: its header-column index, compiled
/// regex, and the field -> position map being filled in row by row.
struct PendingBucket {
    header_col: usize,
    header_text: String,
    regex: Regex,
    columns: IndexMap<String, usize>,
}

impl Line {
    /// Builds the `Line` for a bundled [`Table`] from its embedded CSV.
    pub fn for_table(table: Table) -> Result<Self> {
        Self::from_csv_str(table, table.csv())
    }

    /// Builds a `Line` from the raw CSV content of a fec-csv-sources format
    /// table (e.g. the contents of `F3X.csv`). `table` is only used to
    /// label errors and the resulting `Line`; callers testing synthetic
    /// CSVs can pass any variant.
    pub fn from_csv_str(table: Table, csv_content: &str) -> Result<Self> {
        let mut reader = csv::ReaderBuilder::new()
            .has_headers(false)
            .flexible(true)
            .from_reader(csv_content.as_bytes());

        let mut records = reader.records();
        let header = records
            .next()
            .transpose()?
            .ok_or_else(|| FecError::UnknownForm {
                form: table.as_str().to_string(),
            })?;

        // One pending bucket per non-empty, non-"canonical" header cell, in
        // CSV column order. Keeping them in a Vec (rather than two HashMaps
        // keyed by column index) means the later assembly step can't fail.
        let mut pending: Vec<PendingBucket> = Vec::new();
        for (i, cell) in header.iter().enumerate() {
            if !cell.is_empty() && cell != "canonical" {
                pending.push(PendingBucket {
                    header_col: i,
                    header_text: cell.to_string(),
                    regex: compile_anchored(cell)?,
                    columns: IndexMap::new(),
                });
            }
        }

        for row in records {
            let row = row?;
            let Some(canonical) = row.get(0).filter(|c| !c.is_empty()) else {
                continue;
            };
            for bucket in &mut pending {
                let Some(pos_1indexed) = row.get(bucket.header_col).and_then(parse_column_position)
                else {
                    continue;
                };
                // `parse_column_position` guarantees >= 1.
                let zero_indexed = pos_1indexed.saturating_sub(1);
                match bucket.columns.get(canonical) {
                    Some(&existing) if existing != zero_indexed => {
                        return Err(FecError::DuplicateCanonicalField {
                            form: table.as_str().to_string(),
                            version_bucket: bucket.header_text.clone(),
                            canonical: canonical.to_string(),
                            first_position: existing.saturating_add(1),
                            second_position: pos_1indexed,
                        });
                    }
                    Some(_) => {}
                    None => {
                        bucket.columns.insert(canonical.to_string(), zero_indexed);
                    }
                }
            }
        }

        let buckets = pending
            .into_iter()
            .map(|p| Bucket {
                regex: p.regex,
                columns: p.columns,
            })
            .collect();

        Ok(Line { table, buckets })
    }

    /// Which format table this `Line` parses.
    pub fn table(&self) -> Table {
        self.table
    }

    /// Whether any version bucket in this table matches `version`.
    pub fn supports_version(&self, version: &str) -> bool {
        self.find_bucket(version).is_some()
    }

    /// Finds the version bucket whose regex matches `version`. If more than
    /// one bucket matches (shouldn't happen with well-formed tables), the
    /// first one in CSV column order wins -- deterministic, unlike the
    /// original Python (which iterated an unordered dict).
    fn find_bucket(&self, version: &str) -> Option<&Bucket> {
        self.buckets.iter().find(|b| b.regex.is_match(version))
    }

    /// Port of `Line.parse_line`: parses one split raw line into a
    /// canonical-field-name -> cleaned-value map, using the column
    /// positions for the bucket matching `version`.
    pub fn parse_line(
        &self,
        line_array: &[String],
        version: &str,
    ) -> Result<IndexMap<String, String>> {
        let bucket =
            self.find_bucket(version)
                .ok_or_else(|| FecError::NoMatchingVersionBucket {
                    table: self.table.as_str(),
                    version: version.to_string(),
                    line_no: None,
                })?;

        let mut out = IndexMap::with_capacity(bucket.columns.len());
        for (field, &pos) in &bucket.columns {
            let value = line_array
                .get(pos)
                .map(|s| clean_entry(s))
                .unwrap_or_default();
            out.insert(field.clone(), value);
        }
        Ok(out)
    }

    /// Exposes the field -> 0-indexed column map for the bucket matching
    /// `version`, mirroring `Line.get_column_locations` (which in the
    /// original ignores its `version` argument and returns the whole dict;
    /// here we return just the relevant bucket, which is what every caller
    /// in pyfec actually needed).
    pub fn column_locations(&self, version: &str) -> Option<&IndexMap<String, usize>> {
        self.find_bucket(version).map(|b| &b.columns)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_csv() -> &'static str {
        "canonical,^8.2|8.1,,^6.1,\r\nform_type,1,FORM TYPE,1,FORM TYPE\r\nfiler_id,2,FILER ID,2,FILER ID\r\nnew_field,3,NEW FIELD,,\r\n"
    }

    #[test]
    fn parses_spreadsheet_float_formatted_positions() {
        // Regression test for a real fech-sources data quirk found via
        // the fec-parser integration tests: SchA.csv and SchB.csv encode
        // column positions as "7.0" instead of "7" for most (but not
        // all) buckets, because the source table was exported from a
        // spreadsheet. A strict usize parse silently drops every column
        // in the affected bucket, producing lines with zero fields.
        let csv = "canonical,^8.5,,^1,\r\nform_type,1.0,FORM TYPE,1,FORM TYPE\r\namount,21.0,AMOUNT,15,AMOUNT\r\n";
        let line = Line::from_csv_str(Table::SchA, csv).unwrap();
        let row = vec![
            "SA11AI".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "".to_string(),
            "250.00".to_string(),
        ];
        let parsed = line.parse_line(&row, "8.5").unwrap();
        assert_eq!(parsed.get("form_type").unwrap(), "SA11AI");
        assert_eq!(parsed.get("amount").unwrap(), "250.00");
    }

    #[test]
    fn parses_buckets_and_positions() {
        let line = Line::from_csv_str(Table::F3X, sample_csv()).unwrap();
        assert_eq!(line.buckets.len(), 2);

        let row = vec!["F3X".to_string(), "C00123".to_string(), "extra".to_string()];
        let parsed = line.parse_line(&row, "8.2").unwrap();
        assert_eq!(parsed.get("form_type").unwrap(), "F3X");
        assert_eq!(parsed.get("filer_id").unwrap(), "C00123");
        assert_eq!(parsed.get("new_field").unwrap(), "EXTRA");

        let parsed_old = line.parse_line(&row, "6.1").unwrap();
        assert!(!parsed_old.contains_key("new_field"));
    }

    #[test]
    fn missing_version_errors() {
        let line = Line::from_csv_str(Table::F3X, sample_csv()).unwrap();
        let row = vec!["F3X".to_string()];
        assert!(!line.supports_version("3.0"));
        match line.parse_line(&row, "3.0") {
            Err(FecError::NoMatchingVersionBucket { table, version, .. }) => {
                assert_eq!(table, "F3X");
                assert_eq!(version, "3.0");
            }
            other => panic!("expected NoMatchingVersionBucket, got {other:?}"),
        }
    }

    #[test]
    fn every_bundled_table_builds() {
        for &t in Table::ALL {
            let line = Line::for_table(t).unwrap_or_else(|e| panic!("{t}: {e}"));
            assert_eq!(line.table(), t);
            assert!(!line.buckets.is_empty(), "{t} has no version buckets");
        }
    }
}
