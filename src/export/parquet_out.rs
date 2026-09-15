//! Parquet output: one `<Table>.parquet` per table, zstd-compressed, with
//! columns typed from the FEC's field specifications.
//!
//! | FEC type | Arrow type | Blank / unparseable |
//! |---|---|---|
//! | `AMT-n` | `Decimal128(38, 2)` | null |
//! | `NUM-8` (every such field is a `YYYYMMDD` date) | `Date32` | null |
//! | anything else | `Utf8` (non-null, blank as `""`) | -- |
//! | `filing_id`, `line_no` | `Int64` | -- |
//!
//! Decimal, not float: `Decimal128(38, 2)` stores the amount as an integer
//! number of cents, so `15.00` is exactly 1500 and a `SUM` over a schedule is
//! exact in DuckDB, Spark, and pyarrow alike. Other numerics stay text
//! because they include ZIP codes and committee ids with leading zeros, and
//! the spec's `NUM` columns are not all integers in practice.
//!
//! Rows are accumulated in Arrow builders and encoded one
//! [`RecordBatch`] of [`BATCH_ROWS`] at a time, so memory per open table is
//! bounded regardless of the filing's size. Row groups are the `parquet`
//! crate's defaults (up to 1,048,576 rows); smaller row groups did not
//! lower the peak measurably and cost ~15% in file size, and smaller
//! batches than 16,384 gained nothing, so those are the settings here.

use std::path::Path;
use std::sync::Arc;

use arrow::array::{ArrayRef, Date32Builder, Decimal128Builder, Int64Builder, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use chrono::NaiveDate;
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;

use crate::parser::schema::{FieldKind, Layout};
use crate::parser::{parse_fec_date, parse_money};

use super::{Columns, CountingFile, ExportError, Row, TableFile};

/// Rows per [`RecordBatch`] handed to the Parquet writer.
pub(crate) const BATCH_ROWS: usize = 16_384;

/// Precision of the amount columns: the maximum `Decimal128` allows, so no
/// filed amount can overflow it (an `AMT-12` is at most 12 digits).
const AMOUNT_PRECISION: u8 = 38;
/// Scale of the amount columns: FEC amounts have at most two decimals.
const AMOUNT_SCALE: i8 = 2;

pub(crate) struct ParquetTable {
    writer: ArrowWriter<CountingFile>,
    schema: SchemaRef,
    filing_id: Option<Int64Builder>,
    line_no: Int64Builder,
    /// Parallel to `layout.fields`.
    fields: Vec<ColumnBuilder>,
    pending: usize,
}

/// A column's Arrow builder, chosen from the field's FEC type.
enum ColumnBuilder {
    Text(StringBuilder),
    Amount(Decimal128Builder),
    Date(Date32Builder),
}

impl ColumnBuilder {
    fn for_field(layout: &Layout, name: &str) -> Self {
        match layout.spec(name) {
            Some(spec) if spec.kind == FieldKind::Amount => Self::Amount(
                Decimal128Builder::new()
                    .with_data_type(DataType::Decimal128(AMOUNT_PRECISION, AMOUNT_SCALE)),
            ),
            Some(spec) if spec.kind == FieldKind::Numeric && spec.max_len == Some(8) => {
                Self::Date(Date32Builder::new())
            }
            _ => Self::Text(StringBuilder::new()),
        }
    }

    fn arrow_field(&self, name: &str) -> Field {
        match self {
            Self::Text(_) => Field::new(name, DataType::Utf8, false),
            Self::Amount(_) => Field::new(
                name,
                DataType::Decimal128(AMOUNT_PRECISION, AMOUNT_SCALE),
                true,
            ),
            Self::Date(_) => Field::new(name, DataType::Date32, true),
        }
    }

    fn append(&mut self, value: &str) {
        match self {
            Self::Text(b) => b.append_value(value),
            Self::Amount(b) => b.append_option(amount_cents(value)),
            Self::Date(b) => b.append_option(parse_fec_date(value).and_then(days_since_epoch)),
        }
    }

    fn finish(&mut self) -> ArrayRef {
        match self {
            Self::Text(b) => Arc::new(b.finish()),
            Self::Amount(b) => Arc::new(b.finish()),
            Self::Date(b) => Arc::new(b.finish()),
        }
    }
}

/// The amount as an integer number of cents (the `Decimal128(38, 2)`
/// representation), or `None` if blank or not a valid FEC amount.
fn amount_cents(value: &str) -> Option<i128> {
    let mut amount = parse_money(value)?;
    amount.rescale(u32::try_from(AMOUNT_SCALE).ok()?);
    Some(amount.mantissa())
}

/// Days since 1970-01-01, the `Date32` representation.
fn days_since_epoch(date: NaiveDate) -> Option<i32> {
    let epoch = NaiveDate::from_ymd_opt(1970, 1, 1)?;
    i32::try_from(date.signed_duration_since(epoch).num_days()).ok()
}

impl ParquetTable {
    fn flush_batch(&mut self) -> Result<(), ExportError> {
        if self.pending == 0 {
            return Ok(());
        }
        let mut arrays: Vec<ArrayRef> = Vec::with_capacity(self.fields.len() + 2);
        if let Some(b) = &mut self.filing_id {
            arrays.push(Arc::new(b.finish()));
        }
        arrays.push(Arc::new(self.line_no.finish()));
        arrays.extend(self.fields.iter_mut().map(ColumnBuilder::finish));
        let batch = RecordBatch::try_new(Arc::clone(&self.schema), arrays)?;
        self.writer.write(&batch)?;
        self.pending = 0;
        Ok(())
    }
}

impl TableFile for ParquetTable {
    fn open(path: &Path, columns: &Columns) -> Result<Self, ExportError> {
        let fields: Vec<ColumnBuilder> = columns
            .layout
            .fields
            .iter()
            .map(|f| ColumnBuilder::for_field(columns.layout, f.name))
            .collect();

        let mut arrow_fields = Vec::with_capacity(fields.len() + 2);
        if columns.filing_id.is_some() {
            arrow_fields.push(Field::new("filing_id", DataType::Int64, false));
        }
        arrow_fields.push(Field::new("line_no", DataType::Int64, false));
        arrow_fields.extend(
            columns
                .layout
                .fields
                .iter()
                .zip(&fields)
                .map(|(f, b)| b.arrow_field(f.name)),
        );
        let schema: SchemaRef = Arc::new(Schema::new(arrow_fields));

        let props = WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::default()))
            .build();
        let writer = ArrowWriter::try_new(
            CountingFile::create(path)?,
            Arc::clone(&schema),
            Some(props),
        )?;

        Ok(Self {
            writer,
            schema,
            filing_id: columns.filing_id.is_some().then(Int64Builder::new),
            line_no: Int64Builder::new(),
            fields,
            pending: 0,
        })
    }

    fn write(&mut self, row: Row<'_>) -> Result<(), ExportError> {
        if let (Some(b), Some(id)) = (&mut self.filing_id, row.filing_id) {
            b.append_value(id);
        }
        self.line_no.append_value(row.line_no());
        for (builder, (_, value)) in self.fields.iter_mut().zip(row.line.iter()) {
            builder.append(value);
        }
        self.pending += 1;
        if self.pending >= BATCH_ROWS {
            self.flush_batch()?;
        }
        Ok(())
    }

    fn finish(mut self) -> Result<u64, ExportError> {
        self.flush_batch()?;
        // Writes the footer and hands the file back.
        let file = self.writer.into_inner()?;
        file.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amounts_become_exact_cents_or_null() {
        assert_eq!(amount_cents("15.00"), Some(1500));
        assert_eq!(amount_cents("15"), Some(1500));
        assert_eq!(amount_cents("0.5"), Some(50));
        assert_eq!(amount_cents("-75.25"), Some(-7525));
        assert_eq!(amount_cents(""), None);
        assert_eq!(amount_cents("abc"), None);
        assert_eq!(amount_cents("1.234"), None);
    }

    #[test]
    fn dates_become_days_since_epoch_or_null() {
        let d = parse_fec_date("19700101").and_then(days_since_epoch);
        assert_eq!(d, Some(0));
        let d = parse_fec_date("20260515").and_then(days_since_epoch);
        assert_eq!(d, Some(20588));
        assert_eq!(parse_fec_date("00000000").and_then(days_since_epoch), None);
        assert_eq!(parse_fec_date("").and_then(days_since_epoch), None);
    }
}
