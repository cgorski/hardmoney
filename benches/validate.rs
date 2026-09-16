//! `Filing::validate`, `Filing::reconcile`, and `Filing::review` on real
//! fixtures, parsed once outside the timed loop so this measures the
//! checks alone.
//!
//! The fixtures: the largest spec-8.5 F3X in the tree (many Schedule A/B
//! lines, the per-field path), the largest fixture overall (spec 3.00,
//! comma-delimited, older-format demotions), and the F3P (the reconciler's
//! biggest rule table). Set `HARDMONEY_BENCH_FILING=/path/to/big.fec` to
//! add a filing of your own -- a 135 MB presidential report is the case
//! the validator's field-profile cache was built for.
//!
//! Run with `cargo bench --bench validate`; compare two builds with
//! `cargo bench --bench validate -- --save-baseline before` and
//! `... -- --baseline before`.

use std::hint::black_box;
use std::path::{Path, PathBuf};

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use hardmoney::{Filing, ParseOptions};

fn fixtures() -> Vec<(String, Filing, u64)> {
    let mut paths: Vec<(String, PathBuf)> =
        ["F3XN_2011831.fec", "F3XA_27789_v3.fec", "F3PA_1993032.fec"]
            .into_iter()
            .map(|name| {
                (
                    name.to_string(),
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("tests/fixtures")
                        .join(name),
                )
            })
            .collect();
    if let Ok(extra) = std::env::var("HARDMONEY_BENCH_FILING") {
        let path = PathBuf::from(extra);
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "extra".to_string());
        paths.push((name, path));
    }
    paths
        .into_iter()
        .map(|(name, path)| {
            let bytes =
                std::fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            let lines = u64::try_from(bytes.iter().filter(|&&b| b == b'\n').count()).unwrap_or(0);
            let filing = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)
                .unwrap_or_else(|e| panic!("parsing {}: {e}", path.display()))
                .into_parts()
                .0;
            (name, filing, lines)
        })
        .collect()
}

fn bench_checks(c: &mut Criterion) {
    let fixtures = fixtures();

    let mut group = c.benchmark_group("validate");
    for (name, filing, lines) in &fixtures {
        group.throughput(Throughput::Elements(*lines));
        group.bench_with_input(BenchmarkId::from_parameter(name), filing, |b, f| {
            b.iter(|| black_box(f.validate()));
        });
    }
    group.finish();

    let mut group = c.benchmark_group("reconcile");
    for (name, filing, lines) in &fixtures {
        group.throughput(Throughput::Elements(*lines));
        group.bench_with_input(BenchmarkId::from_parameter(name), filing, |b, f| {
            b.iter(|| black_box(f.reconcile().ok()));
        });
    }
    group.finish();

    let mut group = c.benchmark_group("review");
    for (name, filing, lines) in &fixtures {
        group.throughput(Throughput::Elements(*lines));
        group.bench_with_input(BenchmarkId::from_parameter(name), filing, |b, f| {
            b.iter(|| black_box(f.review()));
        });
    }
    group.finish();
}

criterion_group!(benches, bench_checks);
criterion_main!(benches);
