//! `hardmoney query ...`: search the loaded data from the terminal without
//! curl.
//!
//! Every subcommand builds the *same* request the REST API serves and runs
//! it one of two ways:
//!
//! * **In-process** (default): the Axum router from [`hardmoney::api`] is
//!   built against a database pool and the request is dispatched to it
//!   directly. No server needs to be running, and results are guaranteed
//!   identical to the API's because it *is* the API.
//! * **Over HTTP** (`--api-url`): the request is sent to a running
//!   `hardmoney serve` (with `--api-key` if it needs one). Useful when the
//!   database isn't reachable from your machine but the API is.
//!
//! Output is a table by default, or `--json` for the API's raw JSON.

use std::fmt::Write as _;

use axum::body::Body;
use axum::http::Request;
use clap::{Args, Subcommand};
use hardmoney::api::ApiConfig;
use tower::ServiceExt as _;

use super::db_args::DbArgs;

#[derive(Args, Debug)]
pub struct QueryArgs {
    /// Postgres connection URL (used for in-process queries). Ignored when
    /// --api-url is set.
    #[arg(global = true, long, env = "DATABASE_URL")]
    pub database_url: Option<String>,

    /// Namespace (Postgres schema) to query. Ignored when --api-url is set.
    #[arg(
        global = true,
        long,
        env = "HARDMONEY_SCHEMA",
        default_value = "public"
    )]
    pub schema: hardmoney::db::Namespace,

    /// Query a running `hardmoney serve` at this base URL instead of the
    /// database, e.g. http://localhost:8080
    #[arg(global = true, long, env = "HARDMONEY_API_URL")]
    pub api_url: Option<String>,

    /// API key for --api-url (sent as X-Api-Key).
    #[arg(global = true, long, env = "HARDMONEY_API_KEY", hide_env_values = true)]
    pub api_key: Option<String>,

    /// Print the raw JSON the API returns instead of a table.
    #[arg(global = true, long)]
    pub json: bool,

    /// Maximum rows (1-500).
    #[arg(
        global = true,
        long,
        default_value_t = 25,
        value_parser = clap::value_parser!(i64).range(1..=hardmoney::api::pagination::Pagination::MAX_LIMIT)
    )]
    pub limit: i64,

    /// Skip this many rows (for paging).
    #[arg(global = true, long, default_value_t = 0, value_parser = clap::value_parser!(i64).range(0..))]
    pub offset: i64,

    #[command(subcommand)]
    pub what: What,
}

#[derive(Subcommand, Debug)]
#[allow(clippy::large_enum_variant)] // clap subcommand tree; boxing hurts ergonomics for no gain
pub enum What {
    /// Find candidates by name, state, office, or cycle.
    Candidates {
        /// Substring of the candidate's name (case-insensitive).
        #[arg(short = 'q', long)]
        name: Option<String>,
        #[arg(long)]
        cycle: Option<i32>,
        /// Two-letter state.
        #[arg(long)]
        state: Option<String>,
        /// H, S, or P.
        #[arg(long)]
        office: Option<String>,
    },
    /// Show one candidate (all cycles) by ID, e.g. S6NC00407.
    Candidate { cand_id: String },
    /// Find committees by name, type, or cycle.
    Committees {
        #[arg(short = 'q', long)]
        name: Option<String>,
        #[arg(long)]
        cycle: Option<i32>,
        /// FEC committee type code, e.g. P (presidential), H, S, N, O (super PAC).
        #[arg(long = "type")]
        cmte_tp: Option<String>,
    },
    /// Show one committee (all cycles) by ID, e.g. C00913566.
    Committee { cmte_id: String },
    /// Itemized contributions (Schedule A) from the bulk `indiv` file.
    Contributions {
        /// Committee that received the money.
        #[arg(long)]
        committee: Option<String>,
        #[arg(long)]
        cycle: Option<i32>,
        /// Substring of contributor name.
        #[arg(short = 'q', long)]
        name: Option<String>,
        #[arg(long)]
        employer: Option<String>,
        #[arg(long)]
        occupation: Option<String>,
        #[arg(long)]
        state: Option<String>,
        /// ZIP prefix.
        #[arg(long)]
        zip: Option<String>,
        #[arg(long)]
        min_amount: Option<String>,
        #[arg(long)]
        max_amount: Option<String>,
        /// YYYY-MM-DD
        #[arg(long)]
        since: Option<String>,
        /// YYYY-MM-DD
        #[arg(long)]
        until: Option<String>,
    },
    /// Operating expenditures (Schedule B) from the bulk `oppexp` file.
    Disbursements {
        #[arg(long)]
        committee: Option<String>,
        #[arg(long)]
        cycle: Option<i32>,
        /// Substring of payee name.
        #[arg(short = 'q', long)]
        name: Option<String>,
        #[arg(long)]
        city: Option<String>,
        #[arg(long)]
        state: Option<String>,
        /// Substring of the stated purpose.
        #[arg(long)]
        purpose: Option<String>,
        #[arg(long)]
        min_amount: Option<String>,
        #[arg(long)]
        max_amount: Option<String>,
        #[arg(long)]
        since: Option<String>,
        #[arg(long)]
        until: Option<String>,
    },
    /// Independent expenditures (super PAC spending) for or against a
    /// candidate, from the FEC's Schedule E pg_dump.
    Ies {
        /// Candidate ID the spending supports or opposes.
        #[arg(long)]
        candidate: Option<String>,
        /// Committee doing the spending.
        #[arg(long)]
        committee: Option<String>,
        /// S (support) or O (oppose).
        #[arg(long)]
        support_oppose: Option<String>,
    },
    /// A directly-ingested filing's header/summary.
    Filing { filing_id: i64 },
    /// A directly-ingested filing's Schedule E lines.
    FilingIes { filing_id: i64 },
    /// What's loaded in this namespace.
    Schema,
}

impl What {
    /// The API path and query string for this request.
    fn to_request(&self, limit: i64, offset: i64) -> (String, Vec<(&'static str, String)>) {
        let mut q: Vec<(&'static str, String)> = Vec::new();
        fn push(q: &mut Vec<(&'static str, String)>, k: &'static str, v: &Option<String>) {
            if let Some(v) = v {
                q.push((k, v.clone()));
            }
        }
        fn push_i(q: &mut Vec<(&'static str, String)>, k: &'static str, v: &Option<i32>) {
            if let Some(v) = v {
                q.push((k, v.to_string()));
            }
        }
        let path = match self {
            What::Candidates {
                name,
                cycle,
                state,
                office,
            } => {
                push(&mut q, "q", name);
                push_i(&mut q, "cycle", cycle);
                push(&mut q, "state", state);
                push(&mut q, "office", office);
                "/candidates".to_string()
            }
            What::Candidate { cand_id } => format!("/candidates/{}", urlencode(cand_id)),
            What::Committees {
                name,
                cycle,
                cmte_tp,
            } => {
                push(&mut q, "q", name);
                push_i(&mut q, "cycle", cycle);
                push(&mut q, "cmte_tp", cmte_tp);
                "/committees".to_string()
            }
            What::Committee { cmte_id } => format!("/committees/{}", urlencode(cmte_id)),
            What::Contributions {
                committee,
                cycle,
                name,
                employer,
                occupation,
                state,
                zip,
                min_amount,
                max_amount,
                since,
                until,
            } => {
                push(&mut q, "cmte_id", committee);
                push_i(&mut q, "cycle", cycle);
                push(&mut q, "name", name);
                push(&mut q, "employer", employer);
                push(&mut q, "occupation", occupation);
                push(&mut q, "state", state);
                push(&mut q, "zip_code", zip);
                push(&mut q, "min_amount", min_amount);
                push(&mut q, "max_amount", max_amount);
                push(&mut q, "min_date", since);
                push(&mut q, "max_date", until);
                "/schedule-a".to_string()
            }
            What::Disbursements {
                committee,
                cycle,
                name,
                city,
                state,
                purpose,
                min_amount,
                max_amount,
                since,
                until,
            } => {
                push(&mut q, "cmte_id", committee);
                push_i(&mut q, "cycle", cycle);
                push(&mut q, "name", name);
                push(&mut q, "city", city);
                push(&mut q, "state", state);
                push(&mut q, "purpose", purpose);
                push(&mut q, "min_amount", min_amount);
                push(&mut q, "max_amount", max_amount);
                push(&mut q, "min_date", since);
                push(&mut q, "max_date", until);
                "/disbursements".to_string()
            }
            What::Ies {
                candidate,
                committee,
                support_oppose,
            } => {
                push(&mut q, "candidate_id", candidate);
                push(&mut q, "cmte_id", committee);
                push(&mut q, "support_oppose_code", support_oppose);
                "/independent-expenditures".to_string()
            }
            What::Filing { filing_id } => format!("/filings/{filing_id}"),
            What::FilingIes { filing_id } => format!("/filings/{filing_id}/schedule-e"),
            What::Schema => "/schema".to_string(),
        };
        let is_list = matches!(
            self,
            What::Candidates { .. }
                | What::Committees { .. }
                | What::Contributions { .. }
                | What::Disbursements { .. }
                | What::Ies { .. }
        );
        if is_list {
            q.push(("limit", limit.to_string()));
            q.push(("offset", offset.to_string()));
        }
        (path, q)
    }

    /// Columns to show in table mode (in order). Anything else is hidden
    /// unless `--json`.
    fn columns(&self) -> &'static [&'static str] {
        match self {
            What::Candidates { .. } | What::Candidate { .. } => &[
                "cand_id",
                "cycle",
                "cand_name",
                "cand_pty_affiliation",
                "cand_office",
                "cand_office_st",
                "cand_office_district",
                "cand_pcc",
            ],
            What::Committees { .. } | What::Committee { .. } => &[
                "cmte_id",
                "cycle",
                "cmte_nm",
                "cmte_tp",
                "cmte_dsgn",
                "cmte_pty_affiliation",
                "cmte_st",
                "cand_id",
            ],
            What::Contributions { .. } => &[
                "transaction_date",
                "transaction_amt",
                "name",
                "city",
                "state",
                "employer",
                "occupation",
                "cmte_id",
            ],
            What::Disbursements { .. } => &[
                "transaction_date",
                "transaction_amt",
                "name",
                "city",
                "state",
                "purpose",
                "cmte_id",
            ],
            What::Ies { .. } => &[
                "expenditure_date",
                "expenditure_amt",
                "support_oppose_code",
                "candidate_name",
                "committee_name",
                "payee_name",
            ],
            What::Filing { .. } => &[
                "filing_id",
                "form_type",
                "fec_version",
                "committee_id",
                "is_amendment",
                "amends_filing_id",
                "skipped_lines",
            ],
            What::FilingIes { .. } => &[
                "line_index",
                "expenditure_date",
                "expenditure_amt",
                "support_oppose_code",
                "candidate_name",
                "payee_name",
            ],
            What::Schema => &[],
        }
    }
}

pub async fn run(args: QueryArgs) -> super::CliResult {
    let (path, query) = args.what.to_request(args.limit, args.offset);
    let query_string = if query.is_empty() {
        String::new()
    } else {
        let pairs: Vec<String> = query
            .iter()
            .map(|(k, v)| format!("{k}={}", urlencode(v)))
            .collect();
        format!("?{}", pairs.join("&"))
    };
    let uri = format!("{path}{query_string}");

    let (status, body) = match &args.api_url {
        Some(base) => over_http(base, &uri, args.api_key.as_deref())?,
        None => {
            let url = args.database_url.clone().ok_or(
                "set DATABASE_URL / --database-url for in-process queries, or --api-url to query a running server",
            )?;
            let db = DbArgs {
                database_url: url,
                schema: args.schema.clone(),
            };
            let pool = db.connect_current().await?;
            hardmoney::db::ensure_views(&pool).await?;
            in_process(pool, &uri).await?
        }
    };

    if !status.is_success() {
        let msg = body
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("request failed");
        return Err(format!("{} {}: {msg}", status.as_u16(), uri).into());
    }

    if args.json || matches!(args.what, What::Schema) {
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }

    let rows: Vec<&serde_json::Value> = match &body {
        serde_json::Value::Array(a) => a.iter().collect(),
        other => vec![other],
    };
    print!("{}", render_table(&rows, args.what.columns()));
    if rows.is_empty() {
        eprintln!("(no rows)");
    } else if i64::try_from(rows.len()).is_ok_and(|n| n == args.limit) {
        eprintln!(
            "(showing {} rows; use --limit / --offset {} for more, or --json)",
            rows.len(),
            args.offset.saturating_add(args.limit)
        );
    }
    Ok(())
}

async fn in_process(
    pool: sqlx::PgPool,
    uri: &str,
) -> super::CliResult<(axum::http::StatusCode, serde_json::Value)> {
    let config = ApiConfig::new("127.0.0.1:0".parse()?);
    let app = hardmoney::api::router(pool, &config);
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty())?)
        .await?;
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), 64 << 20).await?;
    let body = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
    Ok((status, body))
}

fn over_http(
    base: &str,
    uri: &str,
    api_key: Option<&str>,
) -> super::CliResult<(axum::http::StatusCode, serde_json::Value)> {
    let full = format!("{}{}", base.trim_end_matches('/'), uri);
    let mut req = ureq::get(&full);
    if let Some(k) = api_key {
        req = req.header("x-api-key", k);
    }
    // ureq treats 4xx/5xx as errors by default; we want the body either way.
    let resp = req.config().http_status_as_error(false).build().call()?;
    let status = axum::http::StatusCode::from_u16(resp.status().as_u16())?;
    let text = resp.into_body().read_to_string().unwrap_or_default();
    let body: serde_json::Value = serde_json::from_str(&text).unwrap_or(serde_json::Value::Null);
    Ok((status, body))
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// Renders rows as a fixed-width table over the given columns.
fn render_table(rows: &[&serde_json::Value], columns: &[&str]) -> String {
    let cell = |row: &serde_json::Value, col: &str| -> String {
        match row.get(col) {
            None | Some(serde_json::Value::Null) => String::new(),
            Some(serde_json::Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
        }
    };
    let mut widths: Vec<usize> = columns.iter().map(|c| c.len()).collect();
    let cells: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            columns
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let v = truncate(&cell(r, c), 40);
                    widths[i] = widths[i].max(v.chars().count());
                    v
                })
                .collect()
        })
        .collect();
    let mut out = String::new();
    let line = |out: &mut String, cols: &[String]| {
        let parts: Vec<String> = cols
            .iter()
            .enumerate()
            .map(|(i, c)| format!("{c:<w$}", w = widths[i]))
            .collect();
        out.push_str(parts.join("  ").trim_end());
        out.push('\n');
    };
    line(
        &mut out,
        &columns.iter().map(|c| c.to_string()).collect::<Vec<_>>(),
    );
    line(
        &mut out,
        &widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>(),
    );
    for row in &cells {
        line(&mut out, row);
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_same_request_the_api_serves() {
        let w = What::Contributions {
            committee: Some("C00913566".into()),
            cycle: Some(2026),
            name: Some("SMITH, J".into()),
            employer: None,
            occupation: None,
            state: None,
            zip: None,
            min_amount: Some("1000".into()),
            max_amount: None,
            since: Some("2025-01-01".into()),
            until: None,
        };
        let (path, q) = w.to_request(25, 0);
        assert_eq!(path, "/schedule-a");
        assert!(q.contains(&("cmte_id", "C00913566".to_string())));
        assert!(q.contains(&("min_date", "2025-01-01".to_string())));
        assert!(q.contains(&("limit", "25".to_string())));
        let (path, q) = What::Candidate {
            cand_id: "S6NC00407".into(),
        }
        .to_request(25, 0);
        assert_eq!(path, "/candidates/S6NC00407");
        assert!(q.is_empty());
    }

    #[test]
    fn urlencodes_spaces_and_commas() {
        assert_eq!(urlencode("SMITH, JANE"), "SMITH%2C%20JANE");
        assert_eq!(urlencode("abc-123_x.y~"), "abc-123_x.y~");
    }

    #[test]
    fn renders_a_table() {
        let a = serde_json::json!({"x": "1", "y": null, "z": 3});
        let b = serde_json::json!({"x": "long value here", "y": "b"});
        let t = render_table(&[&a, &b], &["x", "y"]);
        let lines: Vec<&str> = t.lines().collect();
        assert_eq!(lines[0].trim_end(), "x                y");
        assert!(lines[2].starts_with("1"));
        assert!(lines[3].starts_with("long value here  b"));
    }
}
