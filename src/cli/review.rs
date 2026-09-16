//! `hardmoney review`: the checks a Reports Analysis Division analyst
//! makes before sending a Request for Additional Information, on one or
//! more filings (files or filing ids), grouped by concern with the amounts
//! at issue. Advisory: the exit status is 0 whatever is observed; only a
//! file that cannot be read or parsed exits 1.
//!
//! `--eval` measures the checks against the FEC's own RFAI letters: it
//! samples reports whose committee received an RFAI in a cycle and reports
//! from committees that received none, reviews both sets, and prints
//! precision, recall, and F1 of "any observation" against "received an
//! RFAI", with per-concern lift. See the book chapter "Reviewing a filing"
//! for the method, the numbers, and the caveats.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use chrono::{NaiveDate, NaiveDateTime};
use clap::{Args, ValueEnum};
use hardmoney::Cycle;
use hardmoney::fec::openfec::{FilingRecord, FilingsQuery, OpenFec, Page};
use hardmoney::fec::{Cache, docquery_url, fetch_filing_bytes};
use hardmoney::parser::reconcile::ReportChain;
use hardmoney::parser::review::{Concern, Recipient, Review, ReviewOptions};
use hardmoney::{Filing, ParseOptions};
use serde::{Deserialize, Serialize};
use strum::IntoEnumIterator;

use super::CliResult;
use super::filings::CacheArgs;
use super::shared::columns;

#[derive(Args, Debug)]
pub struct ReviewArgs {
    /// `.fec` files, or filing ids (file numbers) to download cache-first.
    #[arg(value_name = "FILE|ID", required_unless_present = "eval")]
    pub inputs: Vec<String>,

    /// The same committee's earlier reports on the same form (files),
    /// for the checks one file cannot make: cash on hand carried forward,
    /// Column B against the period's schedules, aggregates crossing $200
    /// across reports, and contributions repeated from an earlier report.
    /// Takes one current report.
    #[arg(long, value_name = "FILE", num_args = 1.., conflicts_with = "eval")]
    pub with_prior: Vec<PathBuf>,

    /// What kind of committee filed the report, for the contribution
    /// limit. Default: candidate for Forms 3 and 3P; no limit checked on
    /// a Form 3X, since the file does not say whether the filer is a PAC,
    /// a party, or unlimited.
    #[arg(long, value_enum)]
    pub recipient: Option<RecipientArg>,

    /// One JSON array with an object per input: form, committee, recipient,
    /// observations, summary.
    #[arg(long)]
    pub json: bool,

    /// Print every observation; without it, observations beyond the first
    /// twenty per concern are counted, not listed.
    #[arg(long)]
    pub all: bool,

    /// Measure the checks against the FEC's RFAI letters for --cycle
    /// (needs an openFEC key in FEC_API_KEY or ~/fec_api_key.txt).
    #[arg(long)]
    pub eval: bool,

    /// Two-year cycle to evaluate, e.g. 2024.
    #[arg(long, requires = "eval")]
    pub cycle: Option<Cycle>,

    /// How many RFAI'd reports and how many clean reports to review.
    #[arg(long, default_value_t = 100, requires = "eval")]
    pub sample: usize,

    /// Where to write the evaluation as JSON (per-report rows and the
    /// metrics). Default: hardmoney-review-eval-<cycle>.json.
    #[arg(long, value_name = "PATH", requires = "eval")]
    pub out: Option<PathBuf>,

    /// Re-review the reports listed in an earlier --eval JSON (same
    /// labels, same versions, read from the filing cache) and recompute
    /// the metrics. No openFEC requests: for measuring a rule change on
    /// the same sample.
    #[arg(long, value_name = "PATH", requires = "eval", conflicts_with = "cycle")]
    pub rescore: Option<PathBuf>,

    /// Stop the evaluation once this many openFEC requests have been made
    /// (the API allows 1,000 per hour per key).
    #[arg(long, default_value_t = 450, requires = "eval")]
    pub max_requests: usize,

    /// Read every Nth page of the RFAI list and of the clean-report
    /// lists. Default: chosen so the pages read are spread over the whole
    /// list, not its last weeks.
    #[arg(long, requires = "eval")]
    pub page_stride: Option<u32>,

    #[command(flatten)]
    pub cache: CacheArgs,
}

/// [`Recipient`] as a command-line value.
#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecipientArg {
    /// A candidate's committee: the per-election limit.
    Candidate,
    /// A PAC: $5,000 per calendar year.
    Pac,
    /// A national party committee: the four accounts' limits together.
    NationalParty,
    /// A state, district, or local party committee: $10,000 per year.
    StateParty,
    /// Independent-expenditure-only or hybrid: no limit checked.
    Unlimited,
}

impl From<RecipientArg> for Recipient {
    fn from(r: RecipientArg) -> Self {
        match r {
            RecipientArg::Candidate => Recipient::Candidate,
            RecipientArg::Pac => Recipient::Pac,
            RecipientArg::NationalParty => Recipient::NationalParty,
            RecipientArg::StateParty => Recipient::StateParty,
            RecipientArg::Unlimited => Recipient::Unlimited,
        }
    }
}

/// Where an input came from, for messages and JSON.
#[derive(Debug, Clone, Serialize)]
struct Source {
    /// The path, or `<id>.fec` for a downloaded filing.
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    filing_id: Option<u64>,
}

/// Reads a path or downloads an id, leniently, warning on stderr about
/// skipped lines (they are outside every check).
fn load(input: &str, cache: &Cache) -> CliResult<(Source, Filing)> {
    let (source, bytes) = match input.parse::<u64>() {
        Ok(id) if !Path::new(input).exists() => (
            Source {
                name: format!("{id}.fec"),
                filing_id: Some(id),
            },
            fetch_filing_bytes(id, cache, None).map_err(|e| format!("filing {id}: {e}"))?,
        ),
        _ => (
            Source {
                name: input.to_string(),
                filing_id: None,
            },
            std::fs::read(input).map_err(|e| format!("could not read {input}: {e}"))?,
        ),
    };
    let (filing, skipped) = Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT)
        .map_err(|e| format!("{}: {e}", source.name))?
        .into_parts();
    if !skipped.is_empty() {
        eprintln!(
            "warning: {}: {} line(s) skipped and outside every check",
            source.name,
            skipped.len()
        );
    }
    Ok((source, filing))
}

fn committee_id(filing: &Filing) -> String {
    filing
        .summary
        .get_non_empty("filer_committee_id_number")
        .unwrap_or("?")
        .to_string()
}

/// How many observations per concern the text output lists in full.
const LIST_LIMIT: usize = 20;

fn print_review(
    source: &Source,
    filing: &Filing,
    recipient: Option<Recipient>,
    review: &Review,
    all: bool,
) {
    println!(
        "{} {} {} ({}..{}){}",
        source.name,
        filing.raw_form_type,
        committee_id(filing),
        filing.summary.get("coverage_from_date").unwrap_or(""),
        filing.summary.get("coverage_through_date").unwrap_or(""),
        recipient.map_or_else(String::new, |r| format!(", limits as {r}"))
    );
    for (concern, total) in &review.summary.by_concern {
        println!(
            "  {concern}: {} observation(s), {} at issue{}",
            total.count,
            total.amount,
            if concern.is_heuristic() {
                " (heuristic)"
            } else {
                ""
            }
        );
        let mut listed = 0usize;
        for o in review.by_concern(*concern) {
            if !all && listed >= LIST_LIMIT {
                println!(
                    "    ... {} more (--all lists them)",
                    total.count.saturating_sub(listed)
                );
                break;
            }
            listed = listed.saturating_add(1);
            let where_ = match o.line_no {
                Some(n) => format!("line {n}"),
                None => "report".to_string(),
            };
            let id = o
                .transaction_id
                .as_deref()
                .map_or_else(String::new, |id| format!(" {id}"));
            let amount = o.amount.map_or_else(String::new, |a| format!(" {a}"));
            println!("    {where_}{id}{amount}: {}", o.detail);
        }
    }
    if review.is_empty() {
        println!("  no observations");
    } else {
        println!(
            "  {} observation(s) in {} concern(s); amount at issue {}",
            review.summary.observations,
            review.summary.by_concern.len(),
            review.summary.amount_at_issue
        );
    }
}

pub fn run(args: ReviewArgs) -> CliResult {
    if args.eval {
        return eval(args);
    }
    let cache = args.cache.cache();
    let options = args.recipient.map_or_else(ReviewOptions::new, |r| {
        ReviewOptions::new().recipient(Recipient::from(r))
    });
    if !args.with_prior.is_empty() && args.inputs.len() != 1 {
        return Err("--with-prior takes exactly one current report".into());
    }
    let mut out = Vec::new();
    for input in &args.inputs {
        let (source, filing) = load(input, &cache)?;
        let priors: Vec<Filing> = args
            .with_prior
            .iter()
            .map(|p| load(&p.display().to_string(), &cache).map(|(_, f)| f))
            .collect::<Result<_, _>>()?;
        let (review, chain_note) = if priors.is_empty() {
            (filing.review_with(&options), None)
        } else {
            let chain = ReportChain::new(&filing, priors.iter())?;
            for gap in chain.gaps() {
                eprintln!(
                    "warning: no report in the chain covers {gap}; Column B sums and \
                     cross-report aggregates are short by that period's activity"
                );
            }
            let note = serde_json::json!({
                "reports": chain.prior().len() + 1,
                "reports_summed": chain.period_reports().len(),
                "basis": chain.basis(),
                "gaps": chain.gaps(),
            });
            (chain.review_with(&options), Some(note))
        };
        let recipient = options
            .recipient
            .or_else(|| Recipient::implied_by_form(filing.summary.table()));
        if args.json {
            let mut o = serde_json::json!({
                "file": source.name,
                "filing_id": source.filing_id,
                "form_type": filing.raw_form_type,
                "committee_id": committee_id(&filing),
                "coverage_from_date": filing.summary.get("coverage_from_date"),
                "coverage_through_date": filing.summary.get("coverage_through_date"),
                "recipient": recipient,
                "observations": review.observations,
                "summary": review.summary,
            });
            if let (Some(note), serde_json::Value::Object(map)) = (chain_note, &mut o) {
                map.insert("chain".to_string(), note);
            }
            out.push(o);
        } else {
            print_review(&source, &filing, recipient, &review, args.all);
        }
    }
    if args.json {
        println!("{}", serde_json::to_string_pretty(&out)?);
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// --eval
// ---------------------------------------------------------------------------

/// One reviewed report in the evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct EvalRow {
    file_number: u64,
    committee_id: String,
    committee_type: Option<String>,
    recipient: Option<Recipient>,
    form_type: String,
    report_type: Option<String>,
    coverage_start: Option<NaiveDate>,
    coverage_end: Option<NaiveDate>,
    amendment_indicator: Option<String>,
    /// True for the RFAI'd set: an RFAI letter (request type 2) names this
    /// report's coverage period.
    rfai: bool,
    /// The RFAI letter's receipt date (RFAI'd set only).
    rfai_receipt_date: Option<NaiveDate>,
    /// How many RFAI letters of any type the committee received in the
    /// cycle; `Some(0)` marks a committee that received none.
    committee_rfai_count: Option<u64>,
    observations: usize,
    strict_observations: usize,
    by_concern: BTreeMap<String, usize>,
    /// Why the report could not be reviewed (download or parse failure);
    /// such rows are excluded from the metrics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

/// Confusion counts and the derived rates for one predicate.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct Metrics {
    tp: usize,
    fp: usize,
    fn_: usize,
    tn: usize,
    precision: Option<f64>,
    recall: Option<f64>,
    f1: Option<f64>,
}

impl Metrics {
    fn compute(rows: &[&EvalRow], fired: impl Fn(&EvalRow) -> bool) -> Self {
        let mut m = Metrics::default();
        for r in rows.iter().filter(|r| r.error.is_none()) {
            match (fired(r), r.rfai) {
                (true, true) => m.tp += 1,
                (true, false) => m.fp += 1,
                (false, true) => m.fn_ += 1,
                (false, false) => m.tn += 1,
            }
        }
        m.precision = ratio(m.tp, m.tp + m.fp);
        m.recall = ratio(m.tp, m.tp + m.fn_);
        m.f1 = match (m.precision, m.recall) {
            (Some(p), Some(r)) if p + r > 0.0 => Some(2.0 * p * r / (p + r)),
            _ => None,
        };
        m
    }
}

fn ratio(num: usize, den: usize) -> Option<f64> {
    (den > 0).then(|| num as f64 / den as f64)
}

/// Per-concern agreement with the RFAI label.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConcernMetrics {
    concern: String,
    heuristic: bool,
    /// Reports in the RFAI'd set on which the concern fired.
    fired_rfai: usize,
    /// Reports in the clean set on which it fired.
    fired_clean: usize,
    precision: Option<f64>,
    recall: Option<f64>,
    /// P(RFAI | fired) / P(RFAI): 1.0 is no information, 2.0 (with equal
    /// sets) is every firing on an RFAI'd report.
    lift: Option<f64>,
}

impl MetricSet {
    fn compute(rows: &[&EvalRow]) -> Self {
        MetricSet {
            negatives: rows.iter().filter(|r| !r.rfai && r.error.is_none()).count(),
            any_observation: Metrics::compute(rows, |r| r.observations > 0),
            any_strict_observation: Metrics::compute(rows, |r| r.strict_observations > 0),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct EvalReport {
    cycle: u16,
    generated_at: NaiveDateTime,
    sample_requested: usize,
    page_stride: u32,
    api_requests: usize,
    /// The earlier evaluation this one re-scored, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    rescored_from: Option<PathBuf>,
    /// FRQ letters read, and how many were usable (request type 2 with
    /// coverage dates and a committee type that files Form 3, 3P, or 3X).
    frq_read: usize,
    frq_usable: usize,
    /// Committees whose RFAI'd report could not be found among their
    /// filings for the cycle.
    positives_unmatched: usize,
    /// Clean candidates rejected because an RFAI names their coverage
    /// period.
    negatives_rejected_rfai: usize,
    positives: usize,
    negatives: usize,
    reviewed: usize,
    errors: usize,
    /// Clean set: reports no RFAI letter names (the committee may have
    /// received letters about other reports).
    report_level: MetricSet,
    /// Clean set: reports from committees that received no RFAI letter of
    /// any kind in the cycle.
    committee_level: MetricSet,
    per_concern: Vec<ConcernMetrics>,
    reports: Vec<EvalRow>,
}

/// The two predicates, over one definition of the clean set.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct MetricSet {
    /// Reports in the clean set under this definition.
    negatives: usize,
    any_observation: Metrics,
    any_strict_observation: Metrics,
}

/// The openFEC client with a request budget.
struct Api {
    client: OpenFec,
    requests: usize,
    max_requests: usize,
}

impl Api {
    fn filings(&mut self, query: &FilingsQuery) -> CliResult<Page<FilingRecord>> {
        if self.requests >= self.max_requests {
            return Err(format!(
                "request budget of {} reached; raise --max-requests or lower --sample",
                self.max_requests
            )
            .into());
        }
        self.requests += 1;
        Ok(self.client.filings(query)?)
    }
}

/// The base form a committee type files its periodic reports on.
fn form_for_committee_type(code: &str) -> Option<&'static str> {
    match code.trim().to_ascii_uppercase().as_str() {
        "H" | "S" => Some("F3"),
        "P" => Some("F3P"),
        "N" | "Q" | "O" | "V" | "W" | "X" | "Y" | "Z" | "D" => Some("F3X"),
        _ => None,
    }
}

/// An RFAI letter that names a report: committee, form, coverage.
#[derive(Debug, Clone)]
struct Rfai {
    committee_id: String,
    committee_type: String,
    form_type: &'static str,
    report_type: Option<String>,
    coverage_start: NaiveDate,
    coverage_end: NaiveDate,
    receipt_date: Option<NaiveDateTime>,
}

fn rfai_from_record(r: &FilingRecord) -> Option<Rfai> {
    if r.request_type.as_deref() != Some("2") {
        return None;
    }
    let committee_type = r.committee_type.clone()?;
    Some(Rfai {
        committee_id: r.committee_id.clone()?,
        form_type: form_for_committee_type(&committee_type)?,
        committee_type,
        report_type: r.report_type.clone(),
        coverage_start: r.coverage_start_date?,
        coverage_end: r.coverage_end_date?,
        receipt_date: r.receipt_date,
    })
}

/// The version of the report an RFAI concerned: among the committee's
/// filings on the form with the letter's coverage period, the latest one
/// received on or before the letter (RAD reviewed that version), else the
/// earliest.
fn target_report<'a>(rfai: &Rfai, filings: &'a [FilingRecord]) -> Option<&'a FilingRecord> {
    let same_period: Vec<&FilingRecord> = filings
        .iter()
        .filter(|f| !f.is_paper() && f.filing_id().is_some())
        .filter(|f| {
            f.coverage_start_date == Some(rfai.coverage_start)
                && f.coverage_end_date == Some(rfai.coverage_end)
        })
        .collect();
    let before_letter = same_period
        .iter()
        .copied()
        .filter(|f| match (f.receipt_date, rfai.receipt_date) {
            (Some(filed), Some(letter)) => filed.date() <= letter.date(),
            _ => true,
        })
        .max_by_key(|f| f.receipt_date);
    before_letter.or_else(|| same_period.iter().copied().min_by_key(|f| f.receipt_date))
}

/// Reviews the cached filing behind `row` and fills in its counts.
fn score_row(row: &mut EvalRow, url_hint: Option<&str>, cache: &Cache) {
    row.observations = 0;
    row.strict_observations = 0;
    row.by_concern.clear();
    row.error = None;
    let bytes = match fetch_filing_bytes(row.file_number, cache, url_hint) {
        Ok(b) => b,
        Err(e) => {
            row.error = Some(format!("download failed: {e}"));
            return;
        }
    };
    let filing = match Filing::parse_bytes_with(&bytes, &ParseOptions::LENIENT) {
        Ok(l) => l.into_parts().0,
        Err(e) => {
            row.error = Some(format!("parse failed: {e}"));
            return;
        }
    };
    let options = row
        .recipient
        .map_or_else(ReviewOptions::new, |r| ReviewOptions::new().recipient(r));
    let review = filing.review_with(&options);
    row.observations = review.len();
    row.strict_observations = review.strict().count();
    for (concern, total) in &review.summary.by_concern {
        row.by_concern.insert(concern.to_string(), total.count);
    }
}

fn review_record(
    record: &FilingRecord,
    filing_id: u64,
    url_hint: Option<&str>,
    cache: &Cache,
    rfai: Option<&Rfai>,
) -> EvalRow {
    let committee_type = record
        .committee_type
        .clone()
        .or_else(|| rfai.map(|r| r.committee_type.clone()));
    let recipient = committee_type
        .as_deref()
        .and_then(Recipient::from_openfec_committee_type);
    let mut row = EvalRow {
        file_number: filing_id,
        committee_id: record
            .committee_id
            .clone()
            .or_else(|| rfai.map(|r| r.committee_id.clone()))
            .unwrap_or_default(),
        committee_type,
        recipient,
        form_type: record.form_type.clone().unwrap_or_default(),
        report_type: record.report_type.clone(),
        coverage_start: record.coverage_start_date,
        coverage_end: record.coverage_end_date,
        amendment_indicator: record.amendment_indicator.clone(),
        rfai: rfai.is_some(),
        rfai_receipt_date: rfai.and_then(|r| r.receipt_date).map(|d| d.date()),
        committee_rfai_count: None,
        observations: 0,
        strict_observations: 0,
        by_concern: BTreeMap::new(),
        error: None,
    };
    score_row(&mut row, url_hint, cache);
    row
}

/// The metrics over `rows`: both clean-set cuts and the per-concern table.
fn metrics(rows: &[EvalRow]) -> (MetricSet, MetricSet, Vec<ConcernMetrics>) {
    let usable: Vec<&EvalRow> = rows.iter().filter(|r| r.error.is_none()).collect();
    let report_level = MetricSet::compute(&usable);
    let committee_clean: Vec<&EvalRow> = usable
        .iter()
        .copied()
        .filter(|r| r.rfai || r.committee_rfai_count == Some(0))
        .collect();
    let committee_level = MetricSet::compute(&committee_clean);
    let base_rate = ratio(usable.iter().filter(|r| r.rfai).count(), usable.len());
    let mut per_concern = Vec::new();
    for concern in Concern::iter() {
        let name = concern.to_string();
        let fired = |r: &EvalRow| r.by_concern.contains_key(&name);
        let fired_rfai = usable.iter().filter(|r| r.rfai && fired(r)).count();
        let fired_clean = usable.iter().filter(|r| !r.rfai && fired(r)).count();
        let precision = ratio(fired_rfai, fired_rfai + fired_clean);
        let recall = ratio(fired_rfai, usable.iter().filter(|r| r.rfai).count());
        let lift = match (precision, base_rate) {
            (Some(p), Some(b)) if b > 0.0 => Some(p / b),
            _ => None,
        };
        per_concern.push(ConcernMetrics {
            concern: name,
            heuristic: concern.is_heuristic(),
            fired_rfai,
            fired_clean,
            precision,
            recall,
            lift,
        });
    }
    (report_level, committee_level, per_concern)
}

/// `--rescore`: the same sample, the current rules, no network.
fn rescore(args: ReviewArgs, from: &Path) -> CliResult {
    let cache = args.cache.cache();
    let text = std::fs::read_to_string(from)
        .map_err(|e| format!("could not read {}: {e}", from.display()))?;
    let mut earlier: EvalReport = serde_json::from_str(&text)
        .map_err(|e| format!("{} is not a review --eval result: {e}", from.display()))?;
    for row in &mut earlier.reports {
        // Cached by the earlier run; the docquery URL is only a fallback.
        let hint = docquery_url(row.file_number);
        score_row(row, Some(&hint), &cache);
    }
    let (report_level, committee_level, per_concern) = metrics(&earlier.reports);
    let usable = earlier.reports.iter().filter(|r| r.error.is_none()).count();
    let report = EvalReport {
        generated_at: chrono::Utc::now().naive_utc(),
        api_requests: 0,
        rescored_from: Some(from.to_path_buf()),
        reviewed: usable,
        errors: earlier.reports.len().saturating_sub(usable),
        report_level,
        committee_level,
        per_concern,
        ..earlier
    };
    let out = args
        .out
        .unwrap_or_else(|| PathBuf::from(format!("hardmoney-review-eval-{}.json", report.cycle)));
    std::fs::write(&out, serde_json::to_string_pretty(&report)?)?;
    print_eval(&report, &out);
    Ok(())
}

/// The stride that spreads `pages_wanted` page reads over `total` pages,
/// at least 1.
fn spread_stride(total: u32, pages_wanted: u32) -> u32 {
    (total / pages_wanted.max(1)).max(1)
}

/// How many RFAI letters (any type) a committee received in the cycle,
/// and the coverage periods the report letters name.
struct CommitteeLetters {
    count: u64,
    periods: HashSet<(NaiveDate, NaiveDate)>,
}

fn committee_letters(api: &mut Api, committee: &str, year: u16) -> CliResult<CommitteeLetters> {
    let page = api.filings(
        &FilingsQuery::new()
            .committee_id(committee)
            .form_type("FRQ")
            .cycle(year)
            .per_page(100),
    )?;
    Ok(CommitteeLetters {
        count: page.pagination.count,
        periods: page
            .results
            .iter()
            .filter(|r| r.request_type.as_deref() == Some("2"))
            .filter_map(|r| Some((r.coverage_start_date?, r.coverage_end_date?)))
            .collect(),
    })
}

fn eval(args: ReviewArgs) -> CliResult {
    if let Some(from) = args.rescore.clone() {
        return rescore(args, &from);
    }
    let cycle = args
        .cycle
        .ok_or("--eval needs --cycle (e.g. --cycle 2024), or --rescore FILE")?;
    let cache = args.cache.cache();
    let mut api = Api {
        client: OpenFec::from_env()?,
        requests: 0,
        max_requests: args.max_requests,
    };
    let year = cycle.year();
    // Letters usable as positives run about a third of a page, so a
    // sample of N needs about N/33 pages; spread them over the list.
    let pages_wanted = u32::try_from(args.sample.div_ceil(33)).unwrap_or(u32::MAX);

    // 1. RFAI letters about reports, one committee each.
    let mut rfais: Vec<Rfai> = Vec::new();
    let mut seen_committees: HashSet<String> = HashSet::new();
    let (mut frq_read, mut frq_usable) = (0usize, 0usize);
    let mut page = 1u32;
    let mut pages: Option<u32> = None;
    let mut stride = args.page_stride.unwrap_or(1).max(1);
    while rfais.len() < args.sample && pages.is_none_or(|p| page <= p) {
        let q = FilingsQuery::new()
            .form_type("FRQ")
            .cycle(year)
            .sort(Some("-receipt_date"))
            .per_page(100)
            .page(page);
        let result = api.filings(&q)?;
        if pages.is_none() {
            pages = Some(result.pagination.pages);
            if args.page_stride.is_none() {
                stride = spread_stride(result.pagination.pages, pages_wanted);
            }
        }
        eprintln!(
            "FRQ page {page} of {}: {} letter(s)",
            result.pagination.pages,
            result.results.len()
        );
        if result.results.is_empty() {
            break;
        }
        for r in &result.results {
            frq_read += 1;
            let Some(rfai) = rfai_from_record(r) else {
                continue;
            };
            frq_usable += 1;
            if rfais.len() < args.sample && seen_committees.insert(rfai.committee_id.clone()) {
                rfais.push(rfai);
            }
        }
        page = page.saturating_add(stride);
    }

    // 2. The report each letter concerned.
    let mut rows: Vec<EvalRow> = Vec::new();
    let mut positives_unmatched = 0usize;
    let mut needed: BTreeMap<(&'static str, Option<String>), usize> = BTreeMap::new();
    for rfai in &rfais {
        let q = FilingsQuery::new()
            .committee_id(rfai.committee_id.clone())
            .form_type(rfai.form_type)
            .cycle(year)
            .per_page(100);
        let filings = api.filings(&q)?.results;
        let Some(target) = target_report(rfai, &filings) else {
            positives_unmatched += 1;
            continue;
        };
        let Some(id) = target.filing_id() else {
            positives_unmatched += 1;
            continue;
        };
        eprintln!(
            "RFAI'd: {} {} {} {}..{} -> filing {id}",
            rfai.committee_id,
            rfai.form_type,
            rfai.report_type.as_deref().unwrap_or("?"),
            rfai.coverage_start,
            rfai.coverage_end
        );
        let row = review_record(target, id, target.fec_url.as_deref(), &cache, Some(rfai));
        *needed
            .entry((rfai.form_type, target.report_type.clone()))
            .or_default() += 1;
        rows.push(row);
    }
    let positives = rows.len();

    // 3. Clean reports: the same forms and report types, from other
    //    committees, whose coverage period no RFAI letter names; reviewed
    //    at the original version, as RAD first saw it. The committee's
    //    letter count is kept so the metrics can also be cut at committee
    //    level (committees with no letter at all).
    let mut negatives = 0usize;
    let mut negatives_rejected_rfai = 0usize;
    'kinds: for ((form, report_type), want) in needed {
        let mut got = 0usize;
        let mut page = 1u32;
        let mut pages: Option<u32> = None;
        let mut stride = args.page_stride.unwrap_or(1).max(1);
        while got < want && pages.is_none_or(|p| page <= p) {
            let mut q = FilingsQuery::new()
                .form_type(form)
                .cycle(year)
                .most_recent(true)
                .sort(Some("-receipt_date"))
                .per_page(100)
                .page(page);
            if let Some(rt) = &report_type {
                q = q.report_type(rt.clone());
            }
            let result = match api.filings(&q) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("stopping the clean sample: {e}");
                    break 'kinds;
                }
            };
            if pages.is_none() {
                pages = Some(result.pagination.pages);
                if args.page_stride.is_none() {
                    // One page usually yields the handful wanted per kind.
                    stride = spread_stride(result.pagination.pages, 2);
                }
            }
            if result.results.is_empty() {
                break;
            }
            for record in &result.results {
                if got >= want {
                    break;
                }
                let Some(committee) = record.committee_id.clone() else {
                    continue;
                };
                if record.is_paper() || seen_committees.contains(&committee) {
                    continue;
                }
                let Some(ct) = record.committee_type.as_deref() else {
                    continue;
                };
                if form_for_committee_type(ct) != Some(form) {
                    continue;
                }
                let (Some(start), Some(end)) =
                    (record.coverage_start_date, record.coverage_end_date)
                else {
                    continue;
                };
                let letters = match committee_letters(&mut api, &committee, year) {
                    Ok(l) => l,
                    Err(e) => {
                        eprintln!("stopping the clean sample: {e}");
                        break 'kinds;
                    }
                };
                seen_committees.insert(committee.clone());
                if letters.periods.contains(&(start, end)) {
                    negatives_rejected_rfai += 1;
                    continue;
                }
                let original = record
                    .amendment_chain
                    .first()
                    .copied()
                    .and_then(|n| u64::try_from(n).ok())
                    .filter(|&n| n > 0)
                    .or_else(|| record.filing_id());
                let Some(id) = original else {
                    continue;
                };
                let hint = if Some(id) == record.filing_id() {
                    record.fec_url.clone()
                } else {
                    Some(docquery_url(id))
                };
                eprintln!(
                    "clean: {committee} {form} {} {start}..{end} -> filing {id} ({} letter(s) to \
                     the committee)",
                    record.report_type.as_deref().unwrap_or("?"),
                    letters.count
                );
                let mut row = review_record(record, id, hint.as_deref(), &cache, None);
                row.committee_rfai_count = Some(letters.count);
                rows.push(row);
                got += 1;
                negatives += 1;
            }
            page = page.saturating_add(stride);
        }
    }

    // 4. Metrics.
    let (report_level, committee_level, per_concern) = metrics(&rows);
    let errors = rows.iter().filter(|r| r.error.is_some()).count();
    let report = EvalReport {
        cycle: year,
        generated_at: chrono::Utc::now().naive_utc(),
        sample_requested: args.sample,
        page_stride: stride,
        api_requests: api.requests,
        rescored_from: None,
        frq_read,
        frq_usable,
        positives_unmatched,
        negatives_rejected_rfai,
        positives,
        negatives,
        reviewed: rows.len().saturating_sub(errors),
        errors,
        report_level,
        committee_level,
        per_concern,
        reports: rows,
    };

    // 5. Output.
    let out = args
        .out
        .unwrap_or_else(|| PathBuf::from(format!("hardmoney-review-eval-{year}.json")));
    std::fs::write(&out, serde_json::to_string_pretty(&report)?)?;
    print_eval(&report, &out);
    Ok(())
}

fn pct(v: Option<f64>) -> String {
    v.map_or_else(|| "n/a".to_string(), |v| format!("{:.1}%", v * 100.0))
}

fn x(v: Option<f64>) -> String {
    v.map_or_else(|| "n/a".to_string(), |v| format!("{v:.2}x"))
}

fn print_eval(r: &EvalReport, out: &Path) {
    println!(
        "cycle {}: {} RFAI'd report(s) and {} clean report(s) reviewed ({} could not be; {} \
         letters read, {} usable, {} unmatched; {} clean candidates rejected for having an \
         RFAI); {} openFEC request(s)",
        r.cycle,
        r.reports
            .iter()
            .filter(|x| x.rfai && x.error.is_none())
            .count(),
        r.reports
            .iter()
            .filter(|x| !x.rfai && x.error.is_none())
            .count(),
        r.errors,
        r.frq_read,
        r.frq_usable,
        r.positives_unmatched,
        r.negatives_rejected_rfai,
        r.api_requests
    );
    let metric_rows = |name: &str, m: &Metrics| -> [String; 8] {
        [
            name.to_string(),
            m.tp.to_string(),
            m.fp.to_string(),
            m.fn_.to_string(),
            m.tn.to_string(),
            pct(m.precision),
            pct(m.recall),
            pct(m.f1),
        ]
    };
    println!(
        "{}",
        columns(
            &[
                "predicate",
                "tp",
                "fp",
                "fn",
                "tn",
                "precision",
                "recall",
                "f1"
            ],
            &[
                metric_rows(
                    "any observation, clean = report not named by a letter",
                    &r.report_level.any_observation
                ),
                metric_rows(
                    "any non-heuristic observation, same clean set",
                    &r.report_level.any_strict_observation
                ),
                metric_rows(
                    "any observation, clean = committee with no letter",
                    &r.committee_level.any_observation
                ),
                metric_rows(
                    "any non-heuristic observation, same clean set",
                    &r.committee_level.any_strict_observation
                ),
            ]
        )
    );
    let concern_rows: Vec<[String; 6]> = r
        .per_concern
        .iter()
        .map(|c| {
            [
                format!("{}{}", c.concern, if c.heuristic { " (h)" } else { "" }),
                c.fired_rfai.to_string(),
                c.fired_clean.to_string(),
                pct(c.precision),
                pct(c.recall),
                x(c.lift),
            ]
        })
        .collect();
    println!(
        "{}",
        columns(
            &[
                "concern",
                "fired on RFAI'd",
                "fired on clean",
                "precision",
                "recall",
                "lift"
            ],
            &concern_rows
        )
    );
    let forms: BTreeSet<&str> = r.reports.iter().map(|x| x.form_type.as_str()).collect();
    println!(
        "forms: {}; written to {}",
        forms.into_iter().collect::<Vec<_>>().join(", "),
        out.display()
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(rfai: bool, concerns: &[&str], error: Option<&str>) -> EvalRow {
        EvalRow {
            file_number: 1,
            committee_id: "C1".into(),
            committee_type: None,
            recipient: None,
            form_type: "F3X".into(),
            report_type: None,
            coverage_start: None,
            coverage_end: None,
            amendment_indicator: None,
            rfai,
            rfai_receipt_date: None,
            committee_rfai_count: None,
            observations: concerns.len(),
            strict_observations: concerns.len(),
            by_concern: concerns.iter().map(|c| ((*c).to_string(), 1)).collect(),
            error: error.map(str::to_string),
        }
    }

    #[test]
    fn metrics_count_the_confusion_matrix_and_skip_errors() {
        let rows = [
            row(true, &["a"], None),
            row(true, &[], None),
            row(false, &["a"], None),
            row(false, &[], None),
            row(false, &[], None),
            row(true, &["a"], Some("download failed")),
        ];
        let refs: Vec<&EvalRow> = rows.iter().collect();
        let m = Metrics::compute(&refs, |r| r.observations > 0);
        assert_eq!((m.tp, m.fp, m.fn_, m.tn), (1, 1, 1, 2));
        assert_eq!(m.precision, Some(0.5));
        assert_eq!(m.recall, Some(0.5));
        assert_eq!(m.f1, Some(0.5));
        let empty = Metrics::compute(&[], |_| true);
        assert_eq!(empty.precision, None);
        assert_eq!(empty.f1, None);
        assert_eq!(spread_stride(187, 4), 46);
        assert_eq!(spread_stride(2, 4), 1);
        assert_eq!(spread_stride(10, 0), 10);
    }

    #[test]
    fn committee_types_map_to_forms() {
        assert_eq!(form_for_committee_type("H"), Some("F3"));
        assert_eq!(form_for_committee_type("p"), Some("F3P"));
        assert_eq!(form_for_committee_type("O"), Some("F3X"));
        assert_eq!(form_for_committee_type("I"), None);
    }

    #[test]
    fn target_report_prefers_the_version_rad_saw() {
        let d = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let rec = |n: i64, filed: &str| {
            let mut r = FilingRecord::default();
            r.file_number = Some(n);
            r.coverage_start_date = Some(d("2024-04-01"));
            r.coverage_end_date = Some(d("2024-06-30"));
            r.receipt_date = Some(d(filed).and_hms_opt(12, 0, 0).unwrap());
            r.means_filed = Some("e-file".into());
            r
        };
        let filings = vec![
            rec(3, "2024-09-01"),
            rec(1, "2024-07-15"),
            rec(2, "2024-08-01"),
        ];
        let rfai = Rfai {
            committee_id: "C1".into(),
            committee_type: "N".into(),
            form_type: "F3X",
            report_type: Some("Q2".into()),
            coverage_start: d("2024-04-01"),
            coverage_end: d("2024-06-30"),
            receipt_date: Some(d("2024-08-10").and_hms_opt(0, 0, 0).unwrap()),
        };
        assert_eq!(target_report(&rfai, &filings).unwrap().file_number, Some(2));
        let early = Rfai {
            receipt_date: Some(d("2024-07-01").and_hms_opt(0, 0, 0).unwrap()),
            ..rfai.clone()
        };
        assert_eq!(
            target_report(&early, &filings).unwrap().file_number,
            Some(1)
        );
        let other = Rfai {
            coverage_end: d("2024-03-31"),
            ..rfai
        };
        assert!(target_report(&other, &filings).is_none());
    }
}
