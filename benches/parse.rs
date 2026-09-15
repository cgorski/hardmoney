//! Eager (`Filing::parse_bytes`) vs. streaming (`FilingReader`) parsing of
//! real fixtures.
//!
//! Both sides start from bytes already in memory (a `&[u8]` is a
//! `BufRead`), so this measures parsing, not disk. Three shapes of the
//! streaming side are timed: collecting every line into a `Filing` (the
//! like-for-like comparison with the eager parser), iterating without
//! keeping anything (what a one-pass job pays), and iterating only
//! Schedule A through `filter_tables`.
//!
//! Run with `cargo bench --bench parse`; HTML reports land in
//! `target/criterion/`.

use std::hint::black_box;
use std::path::Path;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use hardmoney::parser::stream::FilingReader;
use hardmoney::{Filing, Table};

/// `(name, bytes)` for each fixture benchmarked: the spec-8.5 ASCII-28
/// fixture the task names, plus the largest fixture in the tree, which
/// happens to exercise the comma-delimited (spec 3.00) path.
fn fixtures() -> Vec<(&'static str, Vec<u8>)> {
    ["F3XN_2011831.fec", "F3XA_27789_v3.fec"]
        .into_iter()
        .map(|name| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name);
            let bytes = std::fs::read(&path)
                .unwrap_or_else(|e| panic!("reading fixture {}: {e}", path.display()));
            (name, bytes)
        })
        .collect()
}

fn bench_parse(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse");
    for (name, bytes) in fixtures() {
        group.throughput(Throughput::Bytes(bytes.len() as u64));

        group.bench_with_input(
            BenchmarkId::new("eager/parse_bytes", name),
            &bytes,
            |b, bytes| {
                b.iter(|| {
                    let filing = Filing::parse_bytes(black_box(bytes)).expect("fixture parses");
                    black_box(filing.lines.len())
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("stream/into_filing", name),
            &bytes,
            |b, bytes| {
                b.iter(|| {
                    let filing = FilingReader::new(black_box(bytes.as_slice()))
                        .expect("fixture opens")
                        .into_filing()
                        .expect("fixture parses")
                        .into_strict()
                        .expect("nothing skipped");
                    black_box(filing.lines.len())
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("stream/iterate", name),
            &bytes,
            |b, bytes| {
                b.iter(|| {
                    let reader =
                        FilingReader::new(black_box(bytes.as_slice())).expect("fixture opens");
                    let mut count = 0usize;
                    for line in reader {
                        let line = line.expect("fixture parses");
                        count += usize::from(!line.is_memo());
                    }
                    black_box(count)
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("stream/iterate_sch_a_only", name),
            &bytes,
            |b, bytes| {
                b.iter(|| {
                    let reader = FilingReader::new(black_box(bytes.as_slice()))
                        .expect("fixture opens")
                        .filter_tables([Table::SchA]);
                    let mut count = 0usize;
                    for line in reader {
                        let line = line.expect("fixture parses");
                        count += usize::from(line.table() == Table::SchA);
                    }
                    black_box(count)
                });
            },
        );
    }
    group.finish();
}

criterion_group!(benches, bench_parse);
criterion_main!(benches);
