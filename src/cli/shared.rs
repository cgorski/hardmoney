//! Helpers more than one subcommand module needs: the FEC endpoint flags,
//! the default dump cache directory, a fixed-width column renderer, and
//! [`blocking`] for work that must leave the async runtime.

use std::path::PathBuf;

use clap::Args;
use hardmoney::fec::Endpoints;

use super::CliResult;

/// Runs `f` on tokio's blocking pool and waits for it.
///
/// The CLI is one `#[tokio::main]` runtime. Its blocking work -- `ureq`
/// requests, parsing and checking a whole filing, waiting on a child
/// process -- must not run on a runtime worker while anything else is
/// live there: with `--ingest` a `sqlx` pool's keepalive and reaper tasks
/// share those workers, and a stalled docquery download would stall them
/// too. Everything a closure captures must be owned (`Send + 'static`);
/// the FEC client types (`Endpoints`, `Cache`, `EfileFeed`, `OpenFec`)
/// are cheap to clone for that.
pub async fn blocking<T, F>(f: F) -> CliResult<T>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|e| format!("a background task failed: {e}").into())
}

/// The flags that move a command off the FEC's production hosts, one per
/// base in [`Endpoints`]. Each reads the matching `HARDMONEY_*` variable
/// when the flag is absent, so a deployment can set them once in the
/// environment (or a `.env` file). `HARDMONEY_EFILINGAPPS_BASE` has no
/// flag; [`EndpointArgs::resolve`] reads it from the environment.
///
/// A value that is not an `http://` or `https://` URL with a host is an
/// error that names the flag and the variable; nothing falls back to
/// production silently.
#[derive(Args, Debug, Clone, Default)]
#[command(next_help_heading = "FEC endpoints")]
pub struct EndpointArgs {
    /// Base URL of www.fec.gov: bulk zips, pg_dump archives, daily e-file
    /// zips, data dictionaries. Default: https://www.fec.gov
    #[arg(long, env = Endpoints::WWW_BASE_VAR, value_name = "URL")]
    pub fec_www_base: Option<String>,

    /// Root of the openFEC API. Default: https://api.open.fec.gov/v1/
    #[arg(long, env = Endpoints::OPENFEC_BASE_VAR, value_name = "URL")]
    pub openfec_base: Option<String>,

    /// Base URL of the FEC's raw-filing document store
    /// (/dcdev/posted/<id>.fec is appended). Default:
    /// https://docquery.fec.gov
    #[arg(long, env = Endpoints::DOCQUERY_BASE_VAR, value_name = "URL")]
    pub docquery_base: Option<String>,

    /// Complete URL of the e-file RSS feed. Default:
    /// https://efilingapps.fec.gov/rss/generate?preDefinedFilingType=ALL
    #[arg(long, env = Endpoints::EFILE_RSS_URL_VAR, value_name = "URL")]
    pub efile_rss_url: Option<String>,

    /// Base URL of the FEC's WebCheck validator (/services/upload is
    /// appended). Default: https://efoservices.fec.gov/webcheck
    #[arg(long, env = Endpoints::WEBCHECK_ENDPOINT_VAR, value_name = "URL")]
    pub webcheck_endpoint: Option<String>,
}

impl EndpointArgs {
    /// The flag that stands in for `var`, for error messages.
    fn flag_for(var: &str) -> Option<&'static str> {
        match var {
            Endpoints::WWW_BASE_VAR => Some("--fec-www-base"),
            Endpoints::OPENFEC_BASE_VAR => Some("--openfec-base"),
            Endpoints::DOCQUERY_BASE_VAR => Some("--docquery-base"),
            Endpoints::EFILE_RSS_URL_VAR => Some("--efile-rss-url"),
            Endpoints::WEBCHECK_ENDPOINT_VAR => Some("--webcheck-endpoint"),
            _ => None,
        }
    }

    /// The endpoints these flags select: production, then each flag (or
    /// its variable) applied, then `HARDMONEY_EFILINGAPPS_BASE` from the
    /// environment. Fails, naming the flag and the variable, if a value is
    /// not a usable URL.
    pub fn resolve(&self) -> super::CliResult<Endpoints> {
        Endpoints::from_lookup(|var| match var {
            Endpoints::WWW_BASE_VAR => self.fec_www_base.clone(),
            Endpoints::OPENFEC_BASE_VAR => self.openfec_base.clone(),
            Endpoints::DOCQUERY_BASE_VAR => self.docquery_base.clone(),
            Endpoints::EFILE_RSS_URL_VAR => self.efile_rss_url.clone(),
            Endpoints::WEBCHECK_ENDPOINT_VAR => self.webcheck_endpoint.clone(),
            other => std::env::var(other).ok(),
        })
        .map_err(|e| {
            let var = e.var().unwrap_or("an endpoint override");
            let flag = Self::flag_for(var)
                .map(|f| format!("{f} / "))
                .unwrap_or_default();
            format!(
                "{flag}{var}: {:?} is not a usable URL: {}",
                e.value(),
                e.reason()
            )
            .into()
        })
    }
}

/// Where `bulk-restore-dump`, `bulk-dump-info`, and `dumps` keep the
/// FEC's archives unless `--cache-dir` / `HARDMONEY_CACHE_DIR` says
/// otherwise: `$XDG_CACHE_HOME/hardmoney/dumps`, else
/// `~/.cache/hardmoney/dumps`, else `<temp dir>/hardmoney/dumps`.
///
/// This is the `dumps/` sibling of the raw-filing cache
/// ([`hardmoney::fec::Cache::from_env`] resolves the same base), which is
/// why `efile cache-clear` never touches it.
pub fn default_dump_cache_dir() -> PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|v| !v.is_empty())
                .map(|h| PathBuf::from(h).join(".cache"))
        })
        .unwrap_or_else(std::env::temp_dir)
        .join("hardmoney")
        .join("dumps")
}

/// Renders `header` and `rows` as left-aligned columns, each padded to its
/// widest cell (in characters) and separated by two spaces, one line per
/// row with trailing spaces trimmed. Returns the text without a trailing
/// newline; callers print or indent it.
pub fn columns<const N: usize>(header: &[&str; N], rows: &[[String; N]]) -> String {
    let mut widths: Vec<usize> = header.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (w, cell) in widths.iter_mut().zip(row.iter()) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let line = |cells: &[&str]| -> String {
        cells
            .iter()
            .zip(widths.iter())
            .map(|(c, w)| format!("{c:<w$}"))
            .collect::<Vec<_>>()
            .join("  ")
            .trim_end()
            .to_string()
    };
    let mut out = vec![line(header)];
    for row in rows {
        let cells: Vec<&str> = row.iter().map(String::as_str).collect();
        out.push(line(&cells));
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_pad_to_the_widest_cell_and_trim_the_end() {
        let out = columns(
            &["a", "bb"],
            &[
                ["1".to_string(), "22".to_string()],
                ["333".to_string(), String::new()],
            ],
        );
        assert_eq!(out, "a    bb\n1    22\n333");
        assert_eq!(columns::<1>(&["only"], &[]), "only");
    }

    #[test]
    fn default_dump_cache_dir_ends_in_hardmoney_dumps() {
        let dir = default_dump_cache_dir();
        assert!(dir.ends_with("hardmoney/dumps"), "{}", dir.display());
    }

    #[test]
    fn endpoint_flags_resolve_and_name_themselves_on_error() {
        let production = EndpointArgs::default().resolve().unwrap();
        // Flags win over whatever the environment says for the same base,
        // and the untouched bases stay at production.
        let args = EndpointArgs {
            docquery_base: Some("https://mirror.example.gov/dq/".to_string()),
            openfec_base: Some("http://localhost:9999/v1".to_string()),
            ..EndpointArgs::default()
        };
        let e = args.resolve().unwrap();
        assert_eq!(
            e.docquery_filing(7),
            "https://mirror.example.gov/dq/dcdev/posted/7.fec"
        );
        assert_eq!(e.openfec_base.as_str(), "http://localhost:9999/v1");
        assert_eq!(e.www_base, production.www_base);
        assert_eq!(e.webcheck_endpoint, production.webcheck_endpoint);

        let bad = EndpointArgs {
            fec_www_base: Some("www.mirror.gov".to_string()),
            ..EndpointArgs::default()
        };
        let msg = bad.resolve().unwrap_err().to_string();
        assert!(
            msg.starts_with("--fec-www-base / HARDMONEY_FEC_WWW_BASE:"),
            "{msg}"
        );
        assert!(msg.contains("must start with http:// or https://"), "{msg}");
    }
}
