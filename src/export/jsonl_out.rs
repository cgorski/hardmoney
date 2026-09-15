//! JSON Lines output: one `<Table>.jsonl` per table, one JSON object per
//! row, `\n`-terminated. Keys are in column order (`filing_id`?, `line_no`,
//! then the fields in FEC column order); `filing_id` and `line_no` are JSON
//! numbers, every field is a JSON string, blanks as `""`.

use std::io::Write;
use std::path::Path;

use serde::ser::{Serialize, SerializeMap, Serializer};

use super::{Columns, CountingFile, ExportError, Row, TableFile};

pub(crate) struct JsonlTable {
    out: CountingFile,
}

impl TableFile for JsonlTable {
    fn open(path: &Path, _columns: &Columns) -> Result<Self, ExportError> {
        Ok(Self {
            out: CountingFile::create(path)?,
        })
    }

    fn write(&mut self, row: Row<'_>) -> Result<(), ExportError> {
        serde_json::to_writer(&mut self.out, &row)?;
        self.out.write_all(b"\n")?;
        Ok(())
    }

    fn finish(self) -> Result<u64, ExportError> {
        self.out.finish()
    }
}

impl Serialize for Row<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let extra = 1 + usize::from(self.filing_id.is_some());
        let mut map = serializer.serialize_map(Some(self.line.iter().len() + extra))?;
        if let Some(id) = self.filing_id {
            map.serialize_entry("filing_id", &id)?;
        }
        map.serialize_entry("line_no", &self.line_no())?;
        for (name, value) in self.line.iter() {
            map.serialize_entry(name, value)?;
        }
        map.end()
    }
}
