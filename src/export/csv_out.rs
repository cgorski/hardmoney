//! CSV output: one `<Table>.csv` per table, header row, RFC 4180 quoting
//! (a field is quoted only when it contains a comma, a quote, or a line
//! break; quotes are doubled). Every value is written as the string the
//! parser holds -- exactly as filed apart from surrounding whitespace -- so
//! ZIP codes keep leading zeros and amounts keep their digits.

use std::path::Path;

use super::{Columns, CountingFile, ExportError, Row, TableFile};

pub(crate) struct CsvTable {
    writer: csv::Writer<CountingFile>,
    /// Reused per row so a 700,000-line schedule does not allocate a record
    /// per line.
    record: csv::StringRecord,
}

impl TableFile for CsvTable {
    fn open(path: &Path, columns: &Columns) -> Result<Self, ExportError> {
        let mut writer = csv::WriterBuilder::new().from_writer(CountingFile::create(path)?);
        writer.write_record(columns.names())?;
        Ok(Self {
            writer,
            record: csv::StringRecord::new(),
        })
    }

    fn write(&mut self, row: Row<'_>) -> Result<(), ExportError> {
        self.record.clear();
        if let Some(id) = row.filing_id {
            self.record.push_field(&id.to_string());
        }
        self.record.push_field(&row.line_no().to_string());
        for (_, value) in row.line.iter() {
            self.record.push_field(value);
        }
        self.writer.write_record(&self.record)?;
        Ok(())
    }

    fn finish(self) -> Result<u64, ExportError> {
        // `into_inner` flushes the CSV buffer; the file's own flush is in
        // `CountingFile::finish`.
        let file = self.writer.into_inner().map_err(|e| e.into_error())?;
        file.finish()
    }
}
