//! `hardmoney lag`: how far behind the FEC's processed data is for a
//! filing, from three openFEC endpoints: `/efile/filings/` (when the
//! e-filing system received it), `/filings/` (whether its summary has been
//! processed), and `/operations-log/` (when the summary and the itemized
//! transactions finished loading).

use clap::Args;
use hardmoney::Cycle;
use hardmoney::fec::openfec::{
    ApiKey, OpenFec, ProcessingStage, ProcessingStatus, ScheduleEndpoint,
};
use hardmoney::fec::{Endpoints, fetch_filing_bytes_with};
use hardmoney::{Filing, ParseOptions, Table};
use serde::Serialize;

use super::CliResult;
use super::filings::CacheArgs;
use super::shared::{EndpointArgs, columns};

#[derive(Args, Debug)]
pub struct LagArgs {
    /// Filing ids (file numbers) to look up: three requests per 50 ids.
    #[arg(
        value_name = "FILING_ID",
        required_unless_present = "committee",
        conflicts_with = "committee"
    )]
    pub filing_ids: Vec<u64>,

    /// Instead of ids: every e-filed report this committee filed in
    /// --cycle (every version, not only the most recent), with the
    /// distribution of lags at the end.
    #[arg(long, value_name = "ID", requires = "cycle")]
    pub committee: Option<String>,

    /// Two-year cycle for --committee, e.g. 2026.
    #[arg(long)]
    pub cycle: Option<Cycle>,

    /// With --committee: only this base form type (F3X, F3, F3P, ...).
    #[arg(long, value_name = "FORM")]
    pub form_type: Option<String>,

    /// Also parse the raw filing (downloaded cache-first) and count its
    /// Schedule A, B, and E lines against the rows openFEC has processed
    /// for the filing's pages. Three more requests per filing.
    #[arg(long)]
    pub counts: bool,

    /// One JSON object per filing (JSON Lines) instead of a table, then a
    /// `summary` object with the distribution.
    #[arg(long)]
    pub json: bool,

    #[command(flatten)]
    pub cache: CacheArgs,

    #[command(flatten)]
    pub endpoints: EndpointArgs,
}

/// Raw versus processed row counts for one schedule of one filing.
#[derive(Debug, Clone, Serialize)]
pub struct ScheduleCount {
    pub schedule: String,
    /// Lines in the raw `.fec`, memo entries included (the FEC loads them
    /// too).
    pub raw_lines: usize,
    pub raw_non_memo: usize,
    /// openFEC's `pagination.count` for the filing's image range; `None`
    /// when the request failed.
    pub processed_rows: Option<u64>,
    pub processed_count_exact: Option<bool>,
}

/// One filing's row of output.
#[derive(Debug, Serialize)]
pub struct LagRow {
    #[serde(flatten)]
    pub status: ProcessingStatus,
    pub stage: ProcessingStage,
    pub summary_lag_days: Option<i64>,
    pub transaction_lag_days: Option<i64>,
    pub days_pending: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub counts: Vec<ScheduleCount>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

/// The distribution over a set of filings.
#[derive(Debug, Default, Serialize)]
pub struct LagSummary {
    pub filings: usize,
    pub unknown: usize,
    pub received_only: usize,
    pub summary_loaded: usize,
    pub transactions_loaded: usize,
    pub summary_lag_median_days: Option<i64>,
    pub summary_lag_p90_days: Option<i64>,
    pub transaction_lag_median_days: Option<i64>,
    pub transaction_lag_p90_days: Option<i64>,
    pub transaction_lag_max_days: Option<i64>,
    /// Filings whose transactions are not loaded yet.
    pub pending: usize,
    pub pending_over_30_days: usize,
    pub pending_over_60_days: usize,
}

/// Nearest-rank percentile of a sorted slice; `None` when empty.
fn percentile(sorted: &[i64], q: f64) -> Option<i64> {
    if sorted.is_empty() {
        return None;
    }
    // ceil(q * n) as a 1-based rank, clamped to the slice.
    let n = sorted.len();
    let rank = ((q * n as f64).ceil() as usize).clamp(1, n);
    sorted.get(rank - 1).copied()
}

fn summarize(rows: &[LagRow]) -> LagSummary {
    let mut s = LagSummary {
        filings: rows.len(),
        ..LagSummary::default()
    };
    let mut summary_lags: Vec<i64> = Vec::new();
    let mut transaction_lags: Vec<i64> = Vec::new();
    for r in rows {
        match r.stage {
            ProcessingStage::Unknown => s.unknown += 1,
            ProcessingStage::Received => s.received_only += 1,
            ProcessingStage::SummaryLoaded => s.summary_loaded += 1,
            ProcessingStage::TransactionsLoaded => s.transactions_loaded += 1,
            _ => {}
        }
        summary_lags.extend(r.summary_lag_days);
        transaction_lags.extend(r.transaction_lag_days);
        if let Some(days) = r.days_pending {
            s.pending += 1;
            if days > 30 {
                s.pending_over_30_days += 1;
            }
            if days > 60 {
                s.pending_over_60_days += 1;
            }
        }
    }
    summary_lags.sort_unstable();
    transaction_lags.sort_unstable();
    s.summary_lag_median_days = percentile(&summary_lags, 0.5);
    s.summary_lag_p90_days = percentile(&summary_lags, 0.9);
    s.transaction_lag_median_days = percentile(&transaction_lags, 0.5);
    s.transaction_lag_p90_days = percentile(&transaction_lags, 0.9);
    s.transaction_lag_max_days = transaction_lags.last().copied();
    s
}

fn row(status: ProcessingStatus, today: chrono::NaiveDate) -> LagRow {
    LagRow {
        stage: status.stage(),
        summary_lag_days: status.summary_lag_days(),
        transaction_lag_days: status.transaction_lag_days(),
        days_pending: status.days_pending(today),
        counts: Vec::new(),
        errors: Vec::new(),
        status,
    }
}

/// Fills `row.counts`: raw lines per schedule from the parsed filing,
/// processed rows from the three schedule endpoints.
fn add_counts(
    api: &OpenFec,
    cache: &hardmoney::fec::Cache,
    endpoints: &Endpoints,
    row: &mut LagRow,
) {
    let id = row.status.filing_id;
    let bytes = match fetch_filing_bytes_with(id, cache, row.status.fec_url.as_deref(), endpoints) {
        Ok(b) => b,
        Err(e) => {
            row.errors.push(format!("download failed: {e}"));
            return;
        }
    };
    let filing = match Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT) {
        Ok(lenient) => lenient.value().clone(),
        Err(e) => {
            row.errors.push(format!("parse failed: {e}"));
            return;
        }
    };
    let images = match (
        row.status.beginning_image_number.as_deref(),
        row.status.ending_image_number.as_deref(),
    ) {
        (Some(a), Some(b)) => Some((a.to_string(), b.to_string())),
        _ => None,
    };
    if images.is_none() {
        row.errors.push(
            "no image numbers yet (the filing is not in /filings/), so processed rows cannot be \
             counted"
                .to_string(),
        );
    }
    for (schedule, table) in [
        (ScheduleEndpoint::A, Table::SchA),
        (ScheduleEndpoint::B, Table::SchB),
        (ScheduleEndpoint::E, Table::SchE),
    ] {
        let raw_lines = filing.lines_for(table).count();
        let raw_non_memo = filing.lines_for(table).filter(|l| !l.is_memo()).count();
        let mut count = ScheduleCount {
            schedule: schedule.to_string(),
            raw_lines,
            raw_non_memo,
            processed_rows: None,
            processed_count_exact: None,
        };
        if let (Some((min, max)), Some(committee)) = (&images, row.status.committee_id.as_deref()) {
            match api.processed_rows(schedule, committee, (min, max), row.status.cycle) {
                Ok(p) => {
                    count.processed_rows = Some(p.count);
                    count.processed_count_exact = p.is_count_exact;
                }
                Err(e) => row
                    .errors
                    .push(format!("{schedule}: processed row count failed: {e}")),
            }
        }
        row.counts.push(count);
    }
}

fn days(d: Option<i64>) -> String {
    d.map_or_else(String::new, |d| format!("+{d}d"))
}

fn print_rows(rows: &[LagRow]) {
    let table: Vec<[String; 8]> = rows
        .iter()
        .map(|r| {
            let s = &r.status;
            let transactions = match (s.transaction_data_complete, r.days_pending) {
                (Some(d), _) => format!("{d} {}", days(r.transaction_lag_days)),
                (None, Some(p)) => format!("pending ({p}d so far)"),
                (None, None) => String::new(),
            };
            [
                s.filing_id.to_string(),
                s.form_type.clone().unwrap_or_default(),
                s.report_type.clone().unwrap_or_default(),
                s.committee_id.clone().unwrap_or_default(),
                // `/efile/filings/` records the second; `/filings/` only
                // the day.
                s.received
                    .map(|t| {
                        if s.in_efile {
                            t.format("%Y-%m-%d %H:%M").to_string()
                        } else {
                            t.date().to_string()
                        }
                    })
                    .unwrap_or_default(),
                if s.in_filings { "yes" } else { "no" }.to_string(),
                s.summary_data_complete
                    .map(|t| format!("{} {}", t.date(), days(r.summary_lag_days)))
                    .unwrap_or_default(),
                transactions,
            ]
        })
        .collect();
    println!(
        "{}",
        columns(
            &[
                "file_number",
                "form",
                "report",
                "committee",
                "received",
                "in /filings/",
                "summary loaded",
                "transactions loaded",
            ],
            &table
        )
    );
    for r in rows {
        let id = r.status.filing_id;
        for c in &r.counts {
            let processed = match (c.processed_rows, c.processed_count_exact) {
                (Some(n), Some(false)) => format!("about {n}"),
                (Some(n), _) => n.to_string(),
                (None, _) => "?".to_string(),
            };
            println!(
                "  {id}: {} raw {} line(s) ({} non-memo); processed {processed}",
                c.schedule, c.raw_lines, c.raw_non_memo
            );
        }
        for e in &r.errors {
            println!("  {id}: {e}");
        }
    }
}

fn print_summary(s: &LagSummary) {
    let lag = |median: Option<i64>, p90: Option<i64>| match (median, p90) {
        (Some(m), Some(p)) => format!("median {m} d, p90 {p} d"),
        _ => "n/a".to_string(),
    };
    println!(
        "{} filing(s): {} unknown to openFEC, {} received only, {} summary loaded, {} transactions \
         loaded",
        s.filings, s.unknown, s.received_only, s.summary_loaded, s.transactions_loaded
    );
    println!(
        "summary lag {}; transaction lag {}{}; {} pending ({} past 30 days, {} past 60 days)",
        lag(s.summary_lag_median_days, s.summary_lag_p90_days),
        lag(s.transaction_lag_median_days, s.transaction_lag_p90_days),
        s.transaction_lag_max_days
            .map_or_else(String::new, |m| format!(", max {m} d")),
        s.pending,
        s.pending_over_30_days,
        s.pending_over_60_days
    );
}

pub fn run(args: LagArgs) -> CliResult {
    let endpoints = args.endpoints.resolve()?;
    let api = OpenFec::new(ApiKey::from_env()?).with_endpoints(&endpoints);
    let today = chrono::Utc::now().date_naive();
    let statuses = match (&args.committee, args.cycle) {
        (Some(committee), Some(cycle)) => api.committee_processing_status(
            &committee.trim().to_ascii_uppercase(),
            cycle.year(),
            args.form_type
                .as_deref()
                .map(|f| f.trim().to_ascii_uppercase())
                .as_deref(),
        )?,
        _ => api.processing_status(&args.filing_ids)?,
    };
    let mut rows: Vec<LagRow> = statuses.into_iter().map(|s| row(s, today)).collect();
    if args.counts {
        let cache = args.cache.cache();
        for r in &mut rows {
            add_counts(&api, &cache, &endpoints, r);
        }
    }
    let summary = summarize(&rows);
    if args.json {
        for r in &rows {
            println!("{}", serde_json::to_string(r)?);
        }
        println!(
            "{}",
            serde_json::to_string(&serde_json::json!({ "summary": summary }))?
        );
    } else {
        print_rows(&rows);
        if rows.len() > 1 || args.committee.is_some() {
            print_summary(&summary);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nearest_rank_percentiles() {
        assert_eq!(percentile(&[], 0.5), None);
        assert_eq!(percentile(&[7], 0.5), Some(7));
        assert_eq!(percentile(&[1, 2, 3, 4], 0.5), Some(2));
        assert_eq!(percentile(&[1, 2, 3, 4, 5], 0.5), Some(3));
        assert_eq!(percentile(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10], 0.9), Some(9));
        assert_eq!(percentile(&[1, 2, 3], 1.0), Some(3));
    }
}
