//! `hardmoney efile`: follow the FEC's electronic-filing feed, backfill
//! from the daily archives, and manage the download cache.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use chrono::NaiveDate;
use clap::{Args, Subcommand};
use hardmoney::fec::efile::{EfileFeed, FeedItem, base_form_type, daily_zip_filings};
use hardmoney::fec::{Cache, FecApiError, fetch_filing_bytes};

use super::CliResult;
use super::filings::{Actions, CacheArgs, OptionalDbArgs, Outcome, apply, print_outcome};

#[derive(Args, Debug)]
pub struct EfileArgs {
    #[command(subcommand)]
    pub command: EfileCommand,
}

#[derive(Subcommand, Debug)]
pub enum EfileCommand {
    /// Poll the e-filing RSS feed and process every filing not seen before.
    Watch(WatchArgs),
    /// Walk the FEC's daily e-filing archives over a date range.
    Backfill(BackfillArgs),
    /// Show where the download cache is and what it holds.
    CacheInfo(CacheInfoArgs),
    /// Delete cached raw filings and daily archives.
    CacheClear(CacheClearArgs),
}

pub async fn run(args: EfileArgs) -> CliResult {
    match args.command {
        EfileCommand::Watch(a) => watch(a).await,
        EfileCommand::Backfill(a) => backfill(a).await,
        EfileCommand::CacheInfo(a) => cache_info(a),
        EfileCommand::CacheClear(a) => cache_clear(a),
    }
}

/// Action flags shared by `watch` and `backfill`.
#[derive(Args, Debug, Clone)]
pub struct ActionFlags {
    /// Copy each filing's raw .fec into this directory as <id>.fec.
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,

    /// Run the FEC's acceptance rules on each filing and print the
    /// error/warning counts.
    #[arg(long)]
    pub validate: bool,

    /// Recompute each report's cover-page totals from its schedules and
    /// print whether it balances.
    #[arg(long)]
    pub reconcile: bool,

    /// Ingest each filing into Postgres (needs --database-url).
    #[arg(long)]
    pub ingest: bool,

    #[command(flatten)]
    pub db: OptionalDbArgs,
}

impl ActionFlags {
    async fn actions(&self, exec: Option<PathBuf>) -> CliResult<Actions> {
        Ok(Actions {
            out_dir: self.out.clone(),
            validate: self.validate,
            reconcile: self.reconcile,
            pool: self.db.pool_if(self.ingest).await?,
            exec,
        })
    }
}

fn form_type_matches(form_type: &str, filters: &[String]) -> bool {
    filters.is_empty()
        || filters.iter().any(|f| {
            let f = f.trim();
            form_type.eq_ignore_ascii_case(f) || base_form_type(form_type).eq_ignore_ascii_case(f)
        })
}

#[derive(Args, Debug)]
pub struct WatchArgs {
    /// Seconds between polls.
    #[arg(long, default_value_t = 300, value_name = "SECONDS")]
    pub interval: u64,

    /// Poll once and exit (exit 1 if the poll or any filing failed).
    #[arg(long)]
    pub once: bool,

    /// Only these form types, comma-separated; a base type matches its
    /// amendments too (F3X matches F3XN, F3XA, F3XT).
    #[arg(long, value_name = "FORMS", value_delimiter = ',')]
    pub form_type: Vec<String>,

    /// Only filings by these committee ids, comma-separated.
    #[arg(long, value_name = "IDS", value_delimiter = ',')]
    pub committee: Vec<String>,

    /// Record every id currently in the feed as seen, without processing
    /// anything, then exit: "start following from now".
    #[arg(long)]
    pub mark_seen: bool,

    /// After the other actions, run `PROGRAM <id> <path-to-cached-fec>`
    /// with HARDMONEY_FILING_ID and HARDMONEY_FILING_PATH in its
    /// environment. A non-zero exit is reported as a failure.
    #[arg(long, value_name = "PROGRAM")]
    pub exec: Option<PathBuf>,

    /// One JSON object per new filing (JSON Lines) instead of text.
    #[arg(long)]
    pub json: bool,

    #[command(flatten)]
    pub actions: ActionFlags,

    #[command(flatten)]
    pub cache: CacheArgs,
}

pub async fn watch(args: WatchArgs) -> CliResult {
    if args.interval == 0 {
        return Err("--interval must be at least 1 second".into());
    }
    let cache = args.cache.cache();
    let feed = EfileFeed::new();
    let actions = args.actions.actions(args.exec.clone()).await?;
    let committees: Vec<String> = args
        .committee
        .iter()
        .map(|c| c.trim().to_ascii_uppercase())
        .collect();
    let mut seen: BTreeSet<u64> = cache.read_seen()?;

    if args.mark_seen {
        let items = feed.poll()?;
        let new: Vec<u64> = items
            .iter()
            .map(|i| i.filing_id)
            .filter(|id| !seen.contains(id))
            .collect();
        cache.append_seen(new.iter().copied())?;
        println!(
            "marked {} filing(s) as seen ({} already were); seen list: {}",
            new.len(),
            items.len().saturating_sub(new.len()),
            cache.seen_path().display()
        );
        return Ok(());
    }

    loop {
        let poll = feed.poll();
        let items = match poll {
            Ok(items) => items,
            Err(e) if args.once => return Err(e.into()),
            Err(e) => {
                eprintln!("poll failed: {e}; retrying in {}s", args.interval);
                std::thread::sleep(Duration::from_secs(args.interval));
                continue;
            }
        };
        // Oldest first, so amendments are processed after their originals
        // when both arrive in one poll.
        let mut new: Vec<&FeedItem> = items
            .iter()
            .filter(|i| !seen.contains(&i.filing_id))
            .collect();
        new.sort_by_key(|i| i.filing_id);

        let mut processed = 0usize;
        let mut failures = 0usize;
        for item in &new {
            let wanted = form_type_matches(&item.form_type, &args.form_type)
                && (committees.is_empty()
                    || item
                        .committee_id
                        .as_deref()
                        .is_some_and(|c| committees.iter().any(|w| w == c)));
            if !wanted {
                continue;
            }
            processed += 1;
            // Text mode prints the filing's line before running actions,
            // so an --exec hook's own output appears under it.
            if !args.json {
                print_item_line(item);
            }
            let outcome = if actions.any() {
                Some(fetch_and_apply(&actions, &cache, item.filing_id, Some(&item.url)).await)
            } else {
                None
            };
            if outcome.as_ref().is_some_and(Outcome::failed) {
                failures += 1;
            }
            if args.json {
                print_item_json(item, outcome.as_ref())?;
            } else if let Some(o) = &outcome {
                print_outcome(item.filing_id, o);
            }
        }

        // Every new id is recorded, filtered-out ones included, so a later
        // poll with different filters does not re-run the old ones.
        cache.append_seen(new.iter().map(|i| i.filing_id))?;
        seen.extend(new.iter().map(|i| i.filing_id));

        if !args.json {
            eprintln!(
                "{}: {} item(s) in feed, {} new, {} processed, {} failed",
                chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
                items.len(),
                new.len(),
                processed,
                failures
            );
        }
        if args.once {
            if failures > 0 {
                return Err(format!("{failures} filing(s) could not be processed").into());
            }
            return Ok(());
        }
        std::thread::sleep(Duration::from_secs(args.interval));
    }
}

async fn fetch_and_apply(
    actions: &Actions,
    cache: &Cache,
    id: u64,
    url_hint: Option<&str>,
) -> Outcome {
    match fetch_filing_bytes(id, cache, url_hint) {
        Ok(bytes) => apply(actions, id, &bytes, Some(&cache.filing_path(id))).await,
        Err(e) => Outcome {
            errors: vec![format!("download failed: {e}")],
            ..Outcome::default()
        },
    }
}

fn print_item_json(item: &FeedItem, outcome: Option<&Outcome>) -> CliResult {
    let mut v = serde_json::to_value(item)?;
    if let (Some(o), serde_json::Value::Object(map)) = (outcome, &mut v) {
        map.insert("actions".to_string(), serde_json::to_value(o)?);
    }
    println!("{v}");
    Ok(())
}

fn print_item_line(item: &FeedItem) {
    println!(
        "{}  {:<5} {:<9} {}  {}",
        item.filing_id,
        item.form_type,
        item.committee_id.as_deref().unwrap_or(""),
        item.published
            .map(|p| p.format("%Y-%m-%dT%H:%M:%SZ").to_string())
            .unwrap_or_else(|| " ".repeat(20)),
        item.committee_name.as_deref().unwrap_or("")
    );
}

fn parse_day(s: &str) -> Result<NaiveDate, String> {
    let s = s.trim();
    NaiveDate::parse_from_str(s, "%Y%m%d")
        .or_else(|_| NaiveDate::parse_from_str(s, "%Y-%m-%d"))
        .map_err(|_| format!("'{s}' is not a date; use YYYYMMDD or YYYY-MM-DD"))
}

#[derive(Args, Debug)]
pub struct BackfillArgs {
    /// First day to read (YYYYMMDD or YYYY-MM-DD; archives exist from
    /// 2001-02-01 through yesterday).
    #[arg(long, value_parser = parse_day, value_name = "DATE")]
    pub from: NaiveDate,

    /// Last day to read, inclusive [default: --from].
    #[arg(long, value_parser = parse_day, value_name = "DATE")]
    pub to: Option<NaiveDate>,

    /// Only these form types, comma-separated; a base type matches its
    /// amendments too (F3X matches F3XN, F3XA, F3XT).
    #[arg(long, value_name = "FORMS", value_delimiter = ',')]
    pub form_type: Vec<String>,

    /// One JSON object per filing (JSON Lines) instead of text.
    #[arg(long)]
    pub json: bool,

    #[command(flatten)]
    pub actions: ActionFlags,

    #[command(flatten)]
    pub cache: CacheArgs,
}

pub async fn backfill(args: BackfillArgs) -> CliResult {
    let to = args.to.unwrap_or(args.from);
    if to < args.from {
        return Err(format!("--to ({to}) is before --from ({})", args.from).into());
    }
    let cache = args.cache.cache();
    let actions = args.actions.actions(None).await?;

    let mut total = 0usize;
    let mut matched = 0usize;
    let mut failures = 0usize;
    let mut days_failed = 0usize;
    let mut day = args.from;
    loop {
        match daily_zip_filings(day, &cache) {
            Ok(filings) => {
                let mut day_total = 0usize;
                let mut day_matched = 0usize;
                for entry in filings {
                    let (id, bytes) = match entry {
                        Ok(e) => e,
                        Err(e) => {
                            failures += 1;
                            eprintln!("{day}: {e}");
                            continue;
                        }
                    };
                    day_total += 1;
                    let form_type = peek_form_type(&bytes);
                    if !form_type_matches(&form_type, &args.form_type) {
                        continue;
                    }
                    day_matched += 1;
                    let outcome = if actions.any() {
                        // The bytes came from the archive; cache them so
                        // a later `filings`/`watch` needs no download.
                        let path = match cache.write_filing(id, &bytes) {
                            Ok(p) => Some(p),
                            Err(e) => {
                                eprintln!("{id}: could not cache: {e}");
                                None
                            }
                        };
                        Some(apply(&actions, id, &bytes, path.as_deref()).await)
                    } else {
                        None
                    };
                    if outcome.as_ref().is_some_and(Outcome::failed) {
                        failures += 1;
                    }
                    if args.json {
                        let mut v = serde_json::json!({
                            "date": day.to_string(),
                            "filing_id": id,
                            "form_type": form_type,
                            "bytes": bytes.len(),
                        });
                        if let (Some(o), serde_json::Value::Object(map)) = (&outcome, &mut v) {
                            map.insert("actions".to_string(), serde_json::to_value(o)?);
                        }
                        println!("{v}");
                    } else {
                        println!("{day}  {id}  {form_type:<5} {} byte(s)", bytes.len());
                        if let Some(o) = &outcome {
                            print_outcome(id, o);
                        }
                    }
                }
                total += day_total;
                matched += day_matched;
                if !args.json {
                    eprintln!("{day}: {day_total} filing(s) in archive, {day_matched} matched");
                }
            }
            Err(FecApiError::Http { status: 404, .. }) => {
                days_failed += 1;
                eprintln!("{day}: the FEC has no archive for this day (HTTP 404)");
            }
            Err(e) => {
                days_failed += 1;
                eprintln!("{day}: {e}");
            }
        }
        if day >= to {
            break;
        }
        day = match day.succ_opt() {
            Some(d) => d,
            None => break,
        };
    }
    if !args.json {
        eprintln!(
            "backfill {}..{to}: {total} filing(s), {matched} matched, {failures} failed, \
             {days_failed} day(s) unavailable",
            args.from
        );
    }
    if failures > 0 || days_failed > 0 {
        return Err(format!(
            "{failures} filing(s) could not be processed and {days_failed} day(s) were unavailable"
        )
        .into());
    }
    Ok(())
}

/// The form-type token that opens the cover line (the line after `HDR`),
/// upper-cased, without parsing the whole file. Empty if the bytes have
/// no second line.
fn peek_form_type(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes.get(..4096).unwrap_or(bytes));
    let Some(line) = text.lines().nth(1) else {
        return String::new();
    };
    let token = line
        .split(['\u{1c}', ','])
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches('"');
    token.to_ascii_uppercase()
}

#[derive(Args, Debug)]
pub struct CacheInfoArgs {
    /// Print JSON instead of text.
    #[arg(long)]
    pub json: bool,

    #[command(flatten)]
    pub cache: CacheArgs,
}

pub fn cache_info(args: CacheInfoArgs) -> CliResult {
    let cache = args.cache.cache();
    let info = cache.info()?;
    if args.json {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(());
    }
    println!("cache root:      {}", info.root.display());
    println!(
        "raw filings:     {} file(s), {}  ({})",
        info.filings,
        human_bytes(info.filing_bytes),
        cache.filings_dir().display()
    );
    println!(
        "daily archives:  {} file(s), {}  ({})",
        info.daily_zips,
        human_bytes(info.daily_zip_bytes),
        cache.daily_zips_dir().display()
    );
    println!(
        "e-file seen ids: {}  ({})",
        info.seen_ids,
        cache.seen_path().display()
    );
    Ok(())
}

#[derive(Args, Debug)]
pub struct CacheClearArgs {
    /// Also forget which e-filings `watch` has already processed.
    #[arg(long)]
    pub seen: bool,

    #[command(flatten)]
    pub cache: CacheArgs,
}

pub fn cache_clear(args: CacheClearArgs) -> CliResult {
    let cache = args.cache.cache();
    let before = cache.clear(args.seen)?;
    println!(
        "removed {} raw filing(s) and {} daily archive(s) ({}) from {}{}",
        before.filings,
        before.daily_zips,
        human_bytes(before.total_bytes()),
        before.root.display(),
        if args.seen {
            format!("; forgot {} seen id(s)", before.seen_ids)
        } else {
            String::new()
        }
    );
    Ok(())
}

fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = n as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{n} B")
    } else {
        format!("{value:.1} {}", UNITS.get(unit).copied().unwrap_or("B"))
    }
}
