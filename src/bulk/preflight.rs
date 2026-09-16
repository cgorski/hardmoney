//! Prerequisite checks and rough resource estimates for importing the
//! FEC's Postgres dump files ([`super::dump`]).
//!
//! `hardmoney dumps check` and `hardmoney dumps import` call [`run`] and
//! print the [`Preflight`] it returns: one [`Check`] per prerequisite,
//! each with a status, a plain-language detail, and, when it is not a
//! pass, what to do about it. Nothing here changes the database
//! (connecting creates the namespace's schema if it is missing, as every
//! hardmoney database command does).
//!
//! # Estimates
//!
//! [`estimate`] scales the timings the FEC published in the README next
//! to the dumps (a 4-CPU, 50 GB server restoring the February 2021
//! files: Schedule A 5 h data-only or 35 h with indexes, Schedule B
//! 1.5 to 2 h or 12 h; 300 GB of disk data-only or 2 TB with indexes
//! for both) to today's file sizes. They are order-of-magnitude guides
//! for deciding whether to start an import before dinner or before a
//! long weekend, not promises.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;

use super::dump::{DumpSource, fmt_bytes};
use crate::Cycle;
use crate::db::{self, DbConfig, Namespace};

/// The Postgres major version whose `pg_dump` writes the FEC's archives
/// (every archive header says `Dumped by pg_dump version: 15.x`). An
/// older `pg_restore` rejects them with "unsupported version".
pub const DUMP_PG_MAJOR: u32 = 15;

// ---------------------------------------------------------------------------
// Results
// ---------------------------------------------------------------------------

/// How one check came out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    /// Ready.
    Pass,
    /// Will work, but something is worth knowing.
    Warn,
    /// The import will not work until this is fixed.
    Fail,
}

impl Status {
    /// `✓`, `!`, or `✗`.
    #[must_use]
    pub fn symbol(self) -> &'static str {
        match self {
            Self::Pass => "✓",
            Self::Warn => "!",
            Self::Fail => "✗",
        }
    }
}

/// One prerequisite and how it came out.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct Check {
    /// Short label (`pg_restore`, `connection`, `disk space for downloads`).
    pub name: String,
    pub status: Status,
    /// What was found, in a sentence a beginner can read.
    pub detail: String,
    /// What to do about it; present whenever `status` is not `Pass`.
    pub fix: Option<String>,
}

impl Check {
    #[must_use]
    pub fn pass(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: Status::Pass,
            detail: detail.into(),
            fix: None,
        }
    }

    #[must_use]
    pub fn warn(
        name: impl Into<String>,
        detail: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            status: Status::Warn,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }

    #[must_use]
    pub fn fail(
        name: impl Into<String>,
        detail: impl Into<String>,
        fix: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            status: Status::Fail,
            detail: detail.into(),
            fix: Some(fix.into()),
        }
    }
}

/// The outcome of [`run`]: every check, in the order it was made.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct Preflight {
    pub checks: Vec<Check>,
}

impl Preflight {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, check: Check) {
        self.checks.push(check);
    }

    /// The worst status present; `Pass` when there are no checks.
    #[must_use]
    pub fn worst(&self) -> Status {
        self.checks
            .iter()
            .map(|c| c.status)
            .max()
            .unwrap_or(Status::Pass)
    }

    /// Whether nothing failed (warnings do not block an import).
    #[must_use]
    pub fn passed(&self) -> bool {
        self.worst() != Status::Fail
    }

    /// The checks that failed.
    pub fn failures(&self) -> impl Iterator<Item = &Check> {
        self.checks.iter().filter(|c| c.status == Status::Fail)
    }

    /// The check named `name`, if present.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&Check> {
        self.checks.iter().find(|c| c.name == name)
    }
}

impl fmt::Display for Preflight {
    /// One line per check (`✓ name: detail`), with the fix indented under
    /// any check that is not a pass.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for c in &self.checks {
            writeln!(f, "{} {}: {}", c.status.symbol(), c.name, c.detail)?;
            if let Some(fix) = &c.fix {
                for (i, line) in fix.lines().enumerate() {
                    let lead = if i == 0 { "fix: " } else { "     " };
                    writeln!(f, "  {lead}{line}")?;
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Postgres versions
// ---------------------------------------------------------------------------

/// A Postgres major.minor version (`18.3`; `9.6` for the old scheme).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize)]
pub struct PgVersion {
    pub major: u32,
    pub minor: u32,
}

impl PgVersion {
    /// From `server_version_num` (`180003` -> 18.3, `90624` -> 9.6).
    #[must_use]
    pub fn from_num(n: u32) -> Self {
        let major = n / 10_000;
        let rest = n % 10_000;
        // Before 10 the middle two digits were the minor version and the
        // last two the patch level; from 10 on the last four are the minor.
        let minor = if major >= 10 { rest } else { rest / 100 };
        Self { major, minor }
    }

    /// Whether a `pg_restore` of this version can read the FEC's archives.
    #[must_use]
    pub fn can_read_dumps(self) -> bool {
        self.major >= DUMP_PG_MAJOR
    }
}

impl fmt::Display for PgVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

/// Pulls the version out of what a Postgres client tool prints for
/// `--version`: `pg_restore (PostgreSQL) 15.4`,
/// `pg_restore (PostgreSQL) 16.1 (Homebrew)`, `psql (PostgreSQL) 9.6.24`,
/// `17beta1`. The first whitespace-separated token that starts with a
/// digit is taken; `None` if there is none (an empty string, or output
/// from something that is not Postgres).
#[must_use]
pub fn parse_pg_version(text: &str) -> Option<PgVersion> {
    for token in text.split_whitespace() {
        if !token.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let end = token
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .unwrap_or(token.len());
        let Some(numeric) = token.get(..end) else {
            continue;
        };
        let mut parts = numeric.split('.');
        let major: u32 = parts.next()?.parse().ok()?;
        let minor: u32 = parts.next().and_then(|m| m.parse().ok()).unwrap_or(0);
        return Some(PgVersion { major, minor });
    }
    None
}

/// Why `pg_restore --version` gave no usable answer.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ToolVersionError {
    /// The program could not be started (almost always: not installed, or
    /// not on `PATH`).
    #[error("could not run `{tool} --version`: {detail}")]
    NotRunnable { tool: &'static str, detail: String },
    /// It ran, but printed nothing that looks like a version.
    #[error("`{tool} --version` printed {output:?}, which does not contain a version")]
    Unparsable { tool: &'static str, output: String },
}

/// Runs `pg_restore --version` and parses it. Blocking, sub-second.
pub fn pg_restore_version() -> Result<PgVersion, ToolVersionError> {
    let output = std::process::Command::new("pg_restore")
        .arg("--version")
        .output()
        .map_err(|e| ToolVersionError::NotRunnable {
            tool: "pg_restore",
            detail: e.to_string(),
        })?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    parse_pg_version(&text).ok_or(ToolVersionError::Unparsable {
        tool: "pg_restore",
        output: text,
    })
}

/// How to get a current `pg_restore` on this operating system.
#[must_use]
pub fn pg_restore_install_hint() -> String {
    if cfg!(target_os = "macos") {
        "install the Postgres client tools with Homebrew: brew install postgresql@18\n\
         (then open a new terminal so `pg_restore` is on your PATH)"
            .to_string()
    } else {
        "install the Postgres client tools (version 15 or newer):\n\
         Ubuntu/Debian: sudo apt install postgresql-client\n\
         Fedora/RHEL:   sudo dnf install postgresql\n\
         macOS:         brew install postgresql@18"
            .to_string()
    }
}

/// Checks that `pg_restore` is on `PATH` and new enough to read the
/// FEC's archives (major >= [`DUMP_PG_MAJOR`]).
#[must_use]
pub fn check_pg_restore() -> Check {
    const NAME: &str = "pg_restore";
    match pg_restore_version() {
        Ok(v) if v.can_read_dumps() => Check::pass(
            NAME,
            format!("version {v} is installed (the FEC's files need {DUMP_PG_MAJOR} or newer)"),
        ),
        Ok(v) => Check::fail(
            NAME,
            format!(
                "version {v} is installed, but the FEC's files were written by Postgres {DUMP_PG_MAJOR} and an older pg_restore cannot read them"
            ),
            pg_restore_install_hint(),
        ),
        Err(ToolVersionError::NotRunnable { .. }) => Check::fail(
            NAME,
            "not found. pg_restore is the Postgres program that loads a dump file; it comes with the Postgres client tools",
            pg_restore_install_hint(),
        ),
        Err(e @ ToolVersionError::Unparsable { .. }) => Check::warn(
            NAME,
            format!("found, but its version could not be read ({e})"),
            "run `pg_restore --version` yourself; it should say 15 or newer",
        ),
    }
}

// ---------------------------------------------------------------------------
// Database URL and connection
// ---------------------------------------------------------------------------

/// The parts of a connection URL worth showing a person.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct ConnectionSummary {
    pub host: String,
    pub port: u16,
    pub user: String,
    /// The database name; Postgres defaults it to the user name when the
    /// URL has none.
    pub database: String,
    /// Whether the server is on this machine (a Unix socket, `localhost`,
    /// or a loopback address), so local disk checks apply to it.
    pub local: bool,
}

/// Parses `url` the way sqlx will. `None` if it is not a Postgres URL.
#[must_use]
pub fn summarize_url(url: &str) -> Option<ConnectionSummary> {
    let opts: PgConnectOptions = url.parse().ok()?;
    let host = opts.get_host().to_string();
    let local = opts.get_socket().is_some()
        || host.is_empty()
        || host == "localhost"
        || host == "127.0.0.1"
        || host == "::1"
        || host.starts_with('/');
    Some(ConnectionSummary {
        user: opts.get_username().to_string(),
        database: opts
            .get_database()
            .unwrap_or(opts.get_username())
            .to_string(),
        port: opts.get_port(),
        host,
        local,
    })
}

/// `url` with any password replaced by `***`, for printing.
#[must_use]
pub fn redact_url(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_string();
    };
    // The authority ends at the first '/', '?' or '#'; the userinfo is what
    // precedes the last '@' inside it.
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let Some(authority) = rest.get(..authority_end) else {
        return url.to_string();
    };
    let Some(at) = authority.rfind('@') else {
        return url.to_string();
    };
    let Some(userinfo) = authority.get(..at) else {
        return url.to_string();
    };
    let Some((user, _password)) = userinfo.split_once(':') else {
        return url.to_string();
    };
    let Some(tail) = rest.get(at..) else {
        return url.to_string();
    };
    format!("{scheme}://{user}:***{tail}")
}

/// `url` with its password removed, and the password itself,
/// percent-decoded (`p%40ss` is `p@ss`), which is the form libpq reads
/// from `PGPASSWORD`. A URL without a password (or with an empty one)
/// comes back unchanged with `None`.
///
/// For handing a connection to a child process such as `pg_restore`: a
/// password in the argument list is visible to every user on the machine
/// through `ps`, while one in the environment is not, and libpq falls back
/// to `PGPASSWORD` when the URL carries none.
#[must_use]
pub fn strip_password(url: &str) -> (String, Option<String>) {
    let unchanged = || (url.to_string(), None);
    let Some((scheme, rest)) = url.split_once("://") else {
        return unchanged();
    };
    let authority_end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    let Some(authority) = rest.get(..authority_end) else {
        return unchanged();
    };
    let Some(at) = authority.rfind('@') else {
        return unchanged();
    };
    let Some(userinfo) = authority.get(..at) else {
        return unchanged();
    };
    let Some((user, password)) = userinfo.split_once(':') else {
        return unchanged();
    };
    let Some(tail) = rest.get(at..) else {
        return unchanged();
    };
    let password = percent_decode(password);
    (
        format!("{scheme}://{user}{tail}"),
        Some(password).filter(|p| !p.is_empty()),
    )
}

/// `%XX` escapes decoded to bytes; everything else passes through. Bytes
/// that do not form UTF-8 afterwards are replaced.
fn percent_decode(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        char::from(b)
            .to_digit(16)
            .and_then(|d| u8::try_from(d).ok())
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while let Some(&b) = bytes.get(i) {
        if b == b'%'
            && let Some(hi) = bytes.get(i.saturating_add(1)).copied().and_then(hex)
            && let Some(lo) = bytes.get(i.saturating_add(2)).copied().and_then(hex)
        {
            out.push(hi << 4 | lo);
            i = i.saturating_add(3);
        } else {
            out.push(b);
            i = i.saturating_add(1);
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The login name of the person running this, for suggested commands
/// (`USER`, then `USERNAME`, else `you`).
#[must_use]
pub fn local_username() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .ok()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| "you".to_string())
}

/// The two commands that create a local database and point hardmoney at
/// it, for people who have neither.
#[must_use]
pub fn suggest_database_setup() -> String {
    [
        "if Postgres is installed on this computer, create a database and tell hardmoney where it is:".to_string(),
        "  createdb fec".to_string(),
        format!("  export DATABASE_URL=postgres://{}@localhost/fec", local_username()),
        "(put the export line in ~/.zshrc or ~/.bashrc, or in a .env file in this folder, so it is remembered)"
            .to_string(),
    ]
    .join("\n")
}

/// Checks that a database URL was given and parses.
#[must_use]
pub fn check_database_url(url: Option<&str>) -> Check {
    const NAME: &str = "database";
    match url {
        None => Check::fail(
            NAME,
            "none configured: DATABASE_URL is not set and --database-url was not given",
            suggest_database_setup(),
        ),
        Some(url) => match summarize_url(url) {
            Some(s) => Check::pass(
                NAME,
                format!(
                    "{} on {}:{} as user {}",
                    s.database,
                    if s.host.is_empty() {
                        "localhost"
                    } else {
                        s.host.as_str()
                    },
                    s.port,
                    s.user
                ),
            ),
            None => Check::fail(
                NAME,
                format!("{} is not a Postgres URL", redact_url(url)),
                "use the form postgres://USER@HOST:5432/DBNAME, for example postgres://alice@localhost/fec",
            ),
        },
    }
}

/// Turns a connection error into (what happened, what to do), in words
/// for someone who has not run Postgres before. `summary` fills in the
/// host, user, and database names when known.
#[must_use]
pub fn explain_connect_error(summary: Option<&ConnectionSummary>, error: &str) -> (String, String) {
    let lower = error.to_ascii_lowercase();
    let host = summary
        .map(|s| {
            if s.host.is_empty() {
                "localhost".to_string()
            } else {
                s.host.clone()
            }
        })
        .unwrap_or_else(|| "the server".to_string());
    let port = summary.map(|s| s.port).unwrap_or(5432);
    let user = summary
        .map(|s| s.user.clone())
        .unwrap_or_else(|| "that user".to_string());
    let database = summary
        .map(|s| s.database.clone())
        .unwrap_or_else(|| "that database".to_string());

    if lower.contains("connection refused")
        || lower.contains("could not connect")
        || lower.contains("os error 61")
        || lower.contains("os error 111")
    {
        return (
            format!("Postgres is not running at {host}:{port}, or is not accepting connections there."),
            "start it (macOS with Homebrew: brew services start postgresql@18; Ubuntu: sudo systemctl start postgresql), \
             or, if it runs on another machine, check the host and port in the URL"
                .to_string(),
        );
    }
    // sqlx prefixes both with "error returned from database:", so match
    // the quoted object, not the bare word.
    if lower.contains("does not exist") && lower.contains("role \"") {
        return (
            format!("there is no Postgres user named \"{user}\"."),
            format!(
                "use your own login name in the URL (postgres://{}@{host}/{database}), or create the user: createuser --superuser {user}",
                local_username()
            ),
        );
    }
    if lower.contains("does not exist") && lower.contains("database \"") {
        return (
            format!("the database \"{database}\" does not exist yet."),
            format!("create it: createdb {database}"),
        );
    }
    if lower.contains("password") || lower.contains("authentication") {
        return (
            format!("Postgres did not accept the login for user \"{user}\"."),
            format!(
                "put the password in the URL: postgres://{user}:PASSWORD@{host}:{port}/{database}"
            ),
        );
    }
    if lower.contains("timed out") || lower.contains("timeout") {
        return (
            format!("no answer from {host}:{port} (the connection timed out)."),
            "check the host name and that a firewall is not blocking the port".to_string(),
        );
    }
    if lower.contains("nodename nor servname")
        || lower.contains("failed to lookup address")
        || lower.contains("name or service not known")
        || lower.contains("could not translate host name")
    {
        return (
            format!("the host name \"{host}\" could not be found."),
            "check the spelling of the host in the URL; for a database on this computer use localhost".to_string(),
        );
    }
    (
        "could not connect to the database.".to_string(),
        "check the URL (postgres://USER@HOST:PORT/DBNAME) and that Postgres is running; the details below are what Postgres said".to_string(),
    )
}

/// Opens a pool of `max_connections` in `namespace`. On failure returns
/// the `connection` check explaining why.
///
/// A single connection is tried first: sqlx's pool retries a refused
/// connection until its acquire timeout (30 s) and then reports only the
/// timeout, whereas one direct attempt fails at once with the real reason
/// ("Connection refused", "database does not exist", ...).
pub async fn check_connection(
    url: &str,
    namespace: &Namespace,
    max_connections: u32,
) -> Result<PgPool, Check> {
    const NAME: &str = "connection";
    let failed = |e: sqlx::Error| {
        let raw = e.to_string();
        let (what, fix) = explain_connect_error(summarize_url(url).as_ref(), &raw);
        Check::fail(NAME, format!("{what} (Postgres said: {raw})"), fix)
    };
    let options: PgConnectOptions = url.parse().map_err(failed)?;
    let probe = <sqlx::PgConnection as sqlx::Connection>::connect_with(&options)
        .await
        .map_err(failed)?;
    let _ = sqlx::Connection::close(probe).await;
    let config = DbConfig::new(url)
        .namespace(namespace.clone())
        .max_connections(max_connections);
    db::connect(&config).await.map_err(failed)
}

/// The connected server's version: pass at 15 or newer, a warning for
/// 10 through 14 (the FEC asks for 9.6 or newer; hardmoney is tested on
/// 15 and newer), a failure below 10.
pub async fn check_server_version(pool: &PgPool) -> Check {
    const NAME: &str = "Postgres version";
    let num: Result<String, sqlx::Error> =
        sqlx::query_scalar("SELECT current_setting('server_version_num')")
            .fetch_one(pool)
            .await;
    let version = match num {
        Ok(s) => match s.trim().parse::<u32>() {
            Ok(n) => PgVersion::from_num(n),
            Err(_) => {
                return Check::warn(
                    NAME,
                    format!("the server reported an unexpected version number {s:?}"),
                    "run `psql --version` and `SHOW server_version;` yourself; 15 or newer is expected",
                );
            }
        },
        Err(e) => {
            return Check::warn(
                NAME,
                format!("could not read it ({e})"),
                "run `SHOW server_version;` in psql; 15 or newer is expected",
            );
        }
    };
    if version.major >= DUMP_PG_MAJOR {
        Check::pass(NAME, format!("server is Postgres {version}"))
    } else if version.major >= 10 {
        Check::warn(
            NAME,
            format!(
                "server is Postgres {version}; the FEC wrote these files from Postgres {DUMP_PG_MAJOR}. Older servers usually load them, but hardmoney is tested on {DUMP_PG_MAJOR} and newer"
            ),
            "if the import fails with a syntax error, upgrade the server to Postgres 15 or newer",
        )
    } else {
        Check::fail(
            NAME,
            format!("server is Postgres {version}, which is past its end of life"),
            "upgrade the server to Postgres 15 or newer",
        )
    }
}

/// The `disclosure` schema (the FEC's archives hard-code it) exists and
/// is writable, or can be created by this user.
pub async fn check_disclosure_schema(pool: &PgPool) -> Check {
    const NAME: &str = "disclosure schema";
    let row: Result<(bool, bool, bool, String), sqlx::Error> = sqlx::query_as(
        "SELECT to_regnamespace('disclosure') IS NOT NULL, \
                CASE WHEN to_regnamespace('disclosure') IS NOT NULL \
                     THEN has_schema_privilege('disclosure', 'CREATE') ELSE false END, \
                has_database_privilege(current_database(), 'CREATE'), \
                current_user::text",
    )
    .fetch_one(pool)
    .await;
    match row {
        Ok((true, true, _, _)) => Check::pass(
            NAME,
            "exists, and you can create tables in it (the FEC's files always load into this schema)",
        ),
        Ok((true, false, _, user)) => Check::fail(
            NAME,
            format!("exists, but user {user} may not create tables in it"),
            format!(
                "ask whoever administers the database to run: GRANT ALL ON SCHEMA disclosure TO {user};"
            ),
        ),
        Ok((false, _, true, _)) => Check::pass(
            NAME,
            "does not exist yet; it will be created on the first import (the FEC's files always load into this schema)",
        ),
        Ok((false, _, false, user)) => Check::fail(
            NAME,
            format!("does not exist, and user {user} may not create schemas in this database"),
            format!(
                "ask whoever administers the database to run: CREATE SCHEMA disclosure; GRANT ALL ON SCHEMA disclosure TO {user};"
            ),
        ),
        Err(e) => Check::warn(
            NAME,
            format!("could not check it ({e})"),
            "the first import tries CREATE SCHEMA IF NOT EXISTS disclosure and reports if that fails",
        ),
    }
}

/// `pg_trgm` and `btree_gin` are installable on this server. Both ship
/// with Postgres (the `contrib` package on some Linux distributions) and
/// are only needed for name-search indexes; without them an import still
/// works.
pub async fn check_extensions(pool: &PgPool) -> Check {
    const NAME: &str = "extensions";
    let rows: Result<Vec<(String, bool)>, sqlx::Error> = sqlx::query_as(
        "SELECT name::text, installed_version IS NOT NULL \
         FROM pg_available_extensions WHERE name IN ('pg_trgm', 'btree_gin') ORDER BY name",
    )
    .fetch_all(pool)
    .await;
    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            return Check::warn(
                NAME,
                format!("could not list them ({e})"),
                "not required; name searches are faster with pg_trgm installed",
            );
        }
    };
    let available = |name: &str| rows.iter().any(|(n, _)| n == name);
    let installed = |name: &str| rows.iter().any(|(n, i)| n == name && *i);
    let missing: Vec<&str> = ["pg_trgm", "btree_gin"]
        .into_iter()
        .filter(|n| !available(n))
        .collect();
    if missing.is_empty() {
        let state = |name: &str| {
            if installed(name) {
                "installed"
            } else {
                "available"
            }
        };
        Check::pass(
            NAME,
            format!(
                "pg_trgm is {}, btree_gin is {} (installed on the first import if needed; they speed up name searches)",
                state("pg_trgm"),
                state("btree_gin")
            ),
        )
    } else {
        Check::warn(
            NAME,
            format!(
                "{} not available on this server. Imports still work; name searches (contributor, payee) are slower and the FEC's own indexes that need them are skipped",
                missing.join(" and ")
            ),
            "optional: install the Postgres contrib package (Ubuntu: sudo apt install postgresql-contrib) and run `hardmoney dumps check` again",
        )
    }
}

// ---------------------------------------------------------------------------
// Disk space
// ---------------------------------------------------------------------------

/// Free bytes on the file system holding `path` (or its nearest existing
/// ancestor, so a cache directory that does not exist yet is fine).
/// Shells out to `df -Pk`; `None` on Windows, when `df` fails, or when
/// its output is not the POSIX portable format.
#[must_use]
pub fn free_disk_space(path: &Path) -> Option<u64> {
    let mut probe = path;
    while !probe.exists() {
        probe = match probe.parent() {
            // A relative path whose every component is missing (`cache/dumps`
            // before the first download) bottoms out at "", which is the
            // current directory as far as the file system is concerned.
            Some(p) if p.as_os_str().is_empty() => Path::new("."),
            Some(p) => p,
            None => return None,
        };
    }
    let output = std::process::Command::new("df")
        .arg("-Pk")
        .arg(probe)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    parse_df_available(&String::from_utf8_lossy(&output.stdout))
}

/// The `Available` column of `df -Pk` output (1024-byte blocks), as
/// bytes. The portable format is one header line then one line per file
/// system: `Filesystem 1024-blocks Used Available Capacity Mounted on`.
/// `None` if the text is not shaped like that.
#[must_use]
pub fn parse_df_available(text: &str) -> Option<u64> {
    let line = text.lines().skip(1).find(|l| !l.trim().is_empty())?;
    let available_kb: u64 = line.split_whitespace().nth(3)?.parse().ok()?;
    Some(available_kb.saturating_mul(1024))
}

/// Compares free space with what an import needs: pass with at least 20%
/// headroom (Postgres writes WAL and temporary files during a restore),
/// warn when it fits without headroom, fail when it does not fit.
#[must_use]
pub fn judge_space(available: u64, needed: u64) -> Status {
    let comfortable = needed.saturating_add(needed / 5);
    if available >= comfortable {
        Status::Pass
    } else if available >= needed {
        Status::Warn
    } else {
        Status::Fail
    }
}

fn space_check(
    name: &str,
    where_: &str,
    available: Option<u64>,
    needed: Option<u64>,
    fix_when_short: &str,
) -> Check {
    match (available, needed) {
        (None, _) => Check::warn(
            name,
            format!("could not measure free space at {where_}"),
            match needed {
                Some(n) => format!(
                    "make sure about {} is free there before importing",
                    fmt_bytes(n)
                ),
                None => "check free space there yourself (df -h)".to_string(),
            },
        ),
        (Some(avail), None) => Check::pass(name, format!("{} free at {where_}", fmt_bytes(avail))),
        (Some(avail), Some(need)) => match judge_space(avail, need) {
            Status::Pass => Check::pass(
                name,
                format!(
                    "{} free at {where_}; this import needs about {}",
                    fmt_bytes(avail),
                    fmt_bytes(need)
                ),
            ),
            Status::Warn => Check::warn(
                name,
                format!(
                    "{} free at {where_}; this import needs about {}, which leaves little room",
                    fmt_bytes(avail),
                    fmt_bytes(need)
                ),
                "free some space first if you can; Postgres needs working room while it loads",
            ),
            Status::Fail => Check::fail(
                name,
                format!(
                    "only {} free at {where_}, but this import needs about {}",
                    fmt_bytes(avail),
                    fmt_bytes(need)
                ),
                fix_when_short,
            ),
        },
    }
}

/// Free space where downloads are kept, against the bytes still to
/// download (`None` just reports what is free).
#[must_use]
pub fn check_cache_space(cache_dir: &Path, needed: Option<u64>) -> Check {
    space_check(
        "disk space for downloads",
        &cache_dir.display().to_string(),
        free_disk_space(cache_dir),
        needed.filter(|n| *n > 0),
        "free some space, point --cache-dir (or HARDMONEY_CACHE_DIR) at a bigger disk, or choose a smaller import (the small files first)",
    )
}

/// Free space where the server keeps its data (`SHOW data_directory`),
/// against the estimated size of the restored tables. Reading the
/// setting needs superuser or `pg_read_all_settings`; when it is denied,
/// or the server is on another machine, this is a warning that says how
/// much to check for by hand.
pub async fn check_database_space(
    pool: &PgPool,
    summary: Option<&ConnectionSummary>,
    needed: Option<u64>,
) -> Check {
    const NAME: &str = "disk space for the database";
    let need_phrase = |needed: Option<u64>| match needed {
        Some(n) if n > 0 => format!("make sure about {} is free there", fmt_bytes(n)),
        _ => "check free space there yourself (df -h)".to_string(),
    };
    let data_dir: Result<String, sqlx::Error> =
        sqlx::query_scalar("SELECT current_setting('data_directory')")
            .fetch_one(pool)
            .await;
    let data_dir = match data_dir {
        Ok(d) => d,
        Err(e) => {
            let raw = e.to_string();
            let why = if raw.to_ascii_lowercase().contains("permission") {
                "the server would not tell this user where its data directory is (that needs superuser)"
            } else {
                "could not read the server's data directory"
            };
            return Check::warn(NAME, format!("{why}: {raw}"), need_phrase(needed));
        }
    };
    if let Some(s) = summary
        && !s.local
    {
        return Check::warn(
            NAME,
            format!(
                "the database is on another machine ({}), so free space at its data directory {data_dir} cannot be measured from here",
                s.host
            ),
            need_phrase(needed),
        );
    }
    space_check(
        NAME,
        &data_dir,
        free_disk_space(Path::new(&data_dir)),
        needed.filter(|n| *n > 0),
        "free some space on the disk that holds the Postgres data directory, or choose a smaller import (fewer --cycles, or the small files first)",
    )
}

/// Whether `hardmoney schema-init` has run in `namespace` (it must, so
/// the import can be recorded and the friendly views created). A
/// warning rather than a failure, because `dumps import` offers to run
/// it.
pub async fn check_namespace(pool: &PgPool, namespace: &Namespace) -> Check {
    const NAME: &str = "namespace";
    match db::migration_status(pool).await {
        Ok(status) if status.is_current() => Check::pass(
            NAME,
            format!(
                "'{namespace}' is set up for hardmoney (schema version {})",
                status.applied.last().copied().unwrap_or(0)
            ),
        ),
        Ok(status) if status.applied.is_empty() => Check::warn(
            NAME,
            format!(
                "'{namespace}' has not been set up for hardmoney yet (a namespace is the part of the database hardmoney's own tables and views live in)"
            ),
            format!(
                "run: hardmoney schema-init --schema {namespace}\n(or let `hardmoney dumps import` do it; it asks first)"
            ),
        ),
        Ok(status) => Check::warn(
            NAME,
            format!(
                "'{namespace}' needs upgrading ({} pending migration(s))",
                status.pending.len()
            ),
            format!(
                "run: hardmoney schema-init --schema {namespace}\n(or let `hardmoney dumps import` do it; it asks first)"
            ),
        ),
        Err(e) => Check::warn(
            NAME,
            format!("could not read the state of '{namespace}' ({e})"),
            format!("run: hardmoney schema-status --schema {namespace}"),
        ),
    }
}

// ---------------------------------------------------------------------------
// Estimates
// ---------------------------------------------------------------------------

/// A slow home or office connection, for the high end of a download
/// estimate (16 Mbit/s; the FEC's own test ran at 1.75 MB/s).
const SLOW_CONNECTION_BPS: u64 = 2_000_000;
/// A good connection, for the low end (200 Mbit/s).
const FAST_CONNECTION_BPS: u64 = 25_000_000;
/// FEC README: Schedule A 5 h for 47 GB, Schedule B 1.75 h for 17 GB,
/// both data only. About 6 minutes per gigabyte of archive.
const DATA_ONLY_MINUTES_PER_GB: u64 = 6;
/// FEC README: Schedule A 35 h for 47 GB, Schedule B 12 h for 17 GB,
/// with the FEC's 30-odd indexes per child table. About 44 min/GB.
const WITH_FEC_INDEXES_MINUTES_PER_GB: u64 = 44;
/// FEC README: 300 GB of disk for both large tables data only, from
/// about 64 GB of archives.
const DATA_ONLY_DISK_FACTOR: u64 = 5;
/// FEC README: 2 TB with the FEC's indexes, from the same 64 GB.
const FEC_INDEXES_DISK_FACTOR: u64 = 30;
/// The small dumps' own indexes are few (nine btrees on committee
/// history, none on Schedule E).
const SMALL_WITH_INDEXES_DISK_FACTOR: u64 = 8;

/// What one import will take, as ranges. See the module docs for where
/// the numbers come from.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[non_exhaustive]
pub struct ImportNeeds {
    /// Bytes still to download; `0` when the archive is already on disk.
    pub download_bytes: u64,
    /// Rough size of the restored table(s) plus indexes inside Postgres.
    pub database_bytes: u64,
    /// Low and high estimate for the download; `None` when there is none.
    pub download_time: Option<(Duration, Duration)>,
    /// Low and high estimate for `pg_restore` plus hardmoney's indexes.
    pub restore_time: (Duration, Duration),
}

impl ImportNeeds {
    /// Download plus database bytes.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.download_bytes.saturating_add(self.database_bytes)
    }

    /// Low and high estimate for everything: download then restore.
    #[must_use]
    pub fn total_time(&self) -> (Duration, Duration) {
        let (dl_lo, dl_hi) = self.download_time.unwrap_or_default();
        (
            dl_lo.saturating_add(self.restore_time.0),
            dl_hi.saturating_add(self.restore_time.1),
        )
    }

    /// The needs of doing both imports, one after the other.
    #[must_use]
    pub fn plus(&self, other: &Self) -> Self {
        let download_time = match (self.download_time, other.download_time) {
            (None, None) => None,
            (Some(a), None) | (None, Some(a)) => Some(a),
            (Some(a), Some(b)) => Some((a.0.saturating_add(b.0), a.1.saturating_add(b.1))),
        };
        Self {
            download_bytes: self.download_bytes.saturating_add(other.download_bytes),
            database_bytes: self.database_bytes.saturating_add(other.database_bytes),
            download_time,
            restore_time: (
                self.restore_time.0.saturating_add(other.restore_time.0),
                self.restore_time.1.saturating_add(other.restore_time.1),
            ),
        }
    }
}

/// The rough share of a cycle-split archive that one two-year period
/// takes, in percent: 30 for the current and previous cycle, 15 for the
/// two before those, 5 for anything older. The FEC says its data doubles
/// every two and a half years, so recent periods dominate.
#[must_use]
pub fn cycle_share_percent(cycle: Cycle, today: Cycle) -> u64 {
    match today.year().saturating_sub(cycle.year()) {
        0 | 2 => 30,
        4 | 6 => 15,
        _ => 5,
    }
}

/// Estimates one import of `source`.
///
/// * `archive_bytes`: the file's size (from `HEAD`, the cache, or
///   [`DumpSource::approx_size_bytes`]).
/// * `download_needed`: false when the file is already on disk.
/// * `cycles`: the two-year periods being restored from a cycle-split
///   archive; empty means all of it. Only those periods' share of the
///   archive counts toward restore time and database size, but the whole
///   file is still downloaded (the FEC does not offer per-cycle files).
/// * `data_only`: whether the archive's own indexes are skipped. Implied
///   by a non-empty `cycles`.
/// * `today`: the current cycle, for [`cycle_share_percent`].
///
/// Restore estimates span half to double the FEC's published rate, with a
/// floor of 30 seconds to 2 minutes for the small files.
#[must_use]
pub fn estimate(
    source: &DumpSource,
    archive_bytes: u64,
    download_needed: bool,
    cycles: &[Cycle],
    data_only: bool,
    today: Cycle,
) -> ImportNeeds {
    let selective = source.partitioned_by_cycle && !cycles.is_empty();
    let data_only = data_only || selective;
    let share: u64 = if selective {
        cycles
            .iter()
            .map(|c| cycle_share_percent(*c, today))
            .sum::<u64>()
            .min(100)
    } else {
        100
    };
    let selected_bytes = mul_div(archive_bytes, share, 100);

    let disk_factor = if data_only {
        DATA_ONLY_DISK_FACTOR
    } else if source.is_large {
        FEC_INDEXES_DISK_FACTOR
    } else {
        SMALL_WITH_INDEXES_DISK_FACTOR
    };
    let database_bytes = selected_bytes.saturating_mul(disk_factor);

    // The FEC's with-indexes rate reflects 30-odd indexes per child table
    // of the large dumps; the small archives carry few or none, so their
    // whole restore runs at about the data-only rate (the FEC's own figure
    // for each is one minute).
    let minutes_per_gb = if data_only || !source.is_large {
        DATA_ONLY_MINUTES_PER_GB
    } else {
        WITH_FEC_INDEXES_MINUTES_PER_GB
    };
    // seconds = bytes * min/GB * 60 / 1e9
    let mut base_secs = mul_div(
        selected_bytes,
        minutes_per_gb.saturating_mul(60),
        1_000_000_000,
    );
    if data_only {
        // hardmoney's own few indexes afterwards.
        base_secs = base_secs.saturating_add(base_secs / 2);
    }
    let restore_time = (
        Duration::from_secs((base_secs / 2).max(30)),
        Duration::from_secs(base_secs.saturating_mul(2).max(120)),
    );

    let download_time = download_needed.then(|| {
        (
            Duration::from_secs((archive_bytes / FAST_CONNECTION_BPS).max(2)),
            Duration::from_secs((archive_bytes / SLOW_CONNECTION_BPS).max(5)),
        )
    });

    ImportNeeds {
        download_bytes: if download_needed { archive_bytes } else { 0 },
        database_bytes,
        download_time,
        restore_time,
    }
}

/// `a * b / c` without overflowing `u64` on the way.
fn mul_div(a: u64, b: u64, c: u64) -> u64 {
    if c == 0 {
        return 0;
    }
    let product = u128::from(a).saturating_mul(u128::from(b)) / u128::from(c);
    u64::try_from(product).unwrap_or(u64::MAX)
}

/// `549525` -> `549,525`.
#[must_use]
pub fn fmt_count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && digits.len().saturating_sub(i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// `549525` -> `about 550,000`; `1234` -> `about 1,200`; exact under 100.
/// For row counts that are estimates anyway.
#[must_use]
pub fn fmt_rough_count(n: u64) -> String {
    if n < 100 {
        return format!("about {n}");
    }
    let magnitude = 10u64.pow(n.ilog10().saturating_sub(1));
    let rounded = n.div_ceil(magnitude).saturating_mul(magnitude);
    format!("about {}", fmt_count(rounded))
}

/// A duration as a person would say it: `under a minute`, `3 minutes`,
/// `2 hours`, `3 days`.
#[must_use]
pub fn fmt_rough_duration(d: Duration) -> String {
    let (n, unit) = rough_units(d.as_secs());
    if n == 0 {
        return "under a minute".to_string();
    }
    format!("{n} {}", plural(unit, n))
}

/// A range of durations: `under a minute`, `about 1 to 3 minutes`,
/// `about 2 to 6 hours`, `about 1 to 4 days`. Both ends use the unit
/// suited to the high end.
#[must_use]
pub fn fmt_duration_range(low: Duration, high: Duration) -> String {
    let (hi_n, unit) = rough_units(high.as_secs());
    if hi_n == 0 {
        return "under a minute".to_string();
    }
    let lo_n = in_units(low.as_secs(), unit);
    if lo_n == 0 {
        return format!("up to {hi_n} {}", plural(unit, hi_n));
    }
    if lo_n == hi_n {
        return format!("about {hi_n} {}", plural(unit, hi_n));
    }
    format!("about {lo_n} to {hi_n} {}", plural(unit, hi_n))
}

/// Elapsed time to the second: `58 s`, `12 min 4 s`, `3 h 2 min`.
#[must_use]
pub fn fmt_elapsed(d: Duration) -> String {
    let secs = d.as_secs();
    let (h, m, s) = (secs / 3600, (secs % 3600) / 60, secs % 60);
    if h > 0 {
        format!("{h} h {m} min")
    } else if m > 0 {
        format!("{m} min {s} s")
    } else {
        format!("{s} s")
    }
}

fn rough_units(secs: u64) -> (u64, &'static str) {
    if secs < 60 {
        (0, "minute")
    } else if secs < 2 * 3600 {
        (in_units(secs, "minute"), "minute")
    } else if secs < 48 * 3600 {
        (in_units(secs, "hour"), "hour")
    } else {
        (in_units(secs, "day"), "day")
    }
}

fn in_units(secs: u64, unit: &str) -> u64 {
    let divisor = match unit {
        "minute" => 60,
        "hour" => 3600,
        _ => 86_400,
    };
    // Round to nearest, so 90 s is 2 minutes and 29 s is 0.
    secs.saturating_add(divisor / 2) / divisor
}

fn plural(unit: &str, n: u64) -> String {
    if n == 1 {
        unit.to_string()
    } else {
        format!("{unit}s")
    }
}

// ---------------------------------------------------------------------------
// Running every check
// ---------------------------------------------------------------------------

/// What [`run`] checks against.
#[derive(Debug, Clone)]
pub struct PreflightInput<'a> {
    /// `DATABASE_URL` / `--database-url`, if either was given.
    pub database_url: Option<&'a str>,
    /// The namespace the friendly views go in.
    pub namespace: &'a Namespace,
    /// Where downloads are kept.
    pub cache_dir: &'a Path,
    /// The import being sized, for the disk-space checks. `None` reports
    /// free space without judging it.
    pub needs: Option<&'a ImportNeeds>,
}

/// Runs every check. Never fails: a prerequisite that cannot be checked
/// becomes a `Warn` or `Fail` entry. Checks that need a connection are
/// skipped (not listed) when there is no URL or it cannot connect.
///
/// Order: `pg_restore`, `database` (URL), `connection`, `Postgres
/// version`, `disclosure schema`, `extensions`, `disk space for
/// downloads`, `disk space for the database`, `namespace`.
pub async fn run(input: &PreflightInput<'_>) -> Preflight {
    let mut pf = Preflight::new();
    pf.push(check_pg_restore());
    let url_check = check_database_url(input.database_url);
    let url_ok = url_check.status != Status::Fail;
    pf.push(url_check);
    let download_bytes = input.needs.map(|n| n.download_bytes);
    let database_bytes = input.needs.map(|n| n.database_bytes);

    let Some(url) = input.database_url.filter(|_| url_ok) else {
        pf.push(check_cache_space(input.cache_dir, download_bytes));
        return pf;
    };
    let pool = match check_connection(url, input.namespace, 2).await {
        Ok(pool) => pool,
        Err(check) => {
            pf.push(check);
            pf.push(check_cache_space(input.cache_dir, download_bytes));
            return pf;
        }
    };
    let summary = summarize_url(url);
    pf.push(Check::pass(
        "connection",
        format!(
            "connected to {}",
            summary
                .as_ref()
                .map(|s| {
                    format!(
                        "{} on {}",
                        s.database,
                        if s.host.is_empty() {
                            "localhost"
                        } else {
                            s.host.as_str()
                        }
                    )
                })
                .unwrap_or_else(|| "the database".to_string())
        ),
    ));
    pf.push(check_server_version(&pool).await);
    pf.push(check_disclosure_schema(&pool).await);
    pf.push(check_extensions(&pool).await);
    pf.push(check_cache_space(input.cache_dir, download_bytes));
    pf.push(check_database_space(&pool, summary.as_ref(), database_bytes).await);
    pf.push(check_namespace(&pool, input.namespace).await);
    pool.close().await;
    pf
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bulk::dump;

    #[test]
    fn parses_client_tool_versions() {
        let v = |s: &str| parse_pg_version(s).map(|v| (v.major, v.minor));
        assert_eq!(v("pg_restore (PostgreSQL) 15.4"), Some((15, 4)));
        assert_eq!(v("pg_restore (PostgreSQL) 16.1 (Homebrew)"), Some((16, 1)));
        assert_eq!(v("pg_restore (PostgreSQL) 18.3"), Some((18, 3)));
        assert_eq!(v("psql (PostgreSQL) 9.6.24"), Some((9, 6)));
        assert_eq!(v("pg_restore (PostgreSQL) 17beta1"), Some((17, 0)));
        assert_eq!(v("16.1 (Homebrew)"), Some((16, 1)));
        assert_eq!(v("  \n 15.4 \n"), Some((15, 4)));
        assert_eq!(v("garbage"), None);
        assert_eq!(v(""), None);
        assert_eq!(v("pg_restore: command not found"), None);
        assert_eq!(v("version .5"), None);
    }

    #[test]
    fn server_version_numbers() {
        assert_eq!(PgVersion::from_num(180_003).to_string(), "18.3");
        assert_eq!(PgVersion::from_num(150_010).to_string(), "15.10");
        assert_eq!(PgVersion::from_num(90_624).to_string(), "9.6");
        assert!(PgVersion::from_num(150_000).can_read_dumps());
        assert!(!PgVersion::from_num(140_012).can_read_dumps());
        assert!(
            PgVersion {
                major: 16,
                minor: 0
            } > PgVersion {
                major: 15,
                minor: 9
            }
        );
    }

    #[test]
    fn disk_space_arithmetic() {
        // 20% headroom passes, fitting without headroom warns, not
        // fitting fails.
        assert_eq!(judge_space(1_200, 1_000), Status::Pass);
        assert_eq!(judge_space(1_199, 1_000), Status::Warn);
        assert_eq!(judge_space(1_000, 1_000), Status::Warn);
        assert_eq!(judge_space(999, 1_000), Status::Fail);
        assert_eq!(judge_space(0, 0), Status::Pass);
        assert_eq!(judge_space(u64::MAX, u64::MAX), Status::Pass);

        // macOS and Linux `df -Pk` output.
        let mac = "Filesystem 1024-blocks      Used Available Capacity  Mounted on\n\
                   /dev/disk3s1s1  971350180 10944700 425108208     3%    /\n";
        assert_eq!(parse_df_available(mac), Some(425_108_208 * 1024));
        let linux = "Filesystem     1024-blocks     Used Available Capacity Mounted on\n\
                     /dev/nvme0n1p2   490691512 91260248 374425864      20% /\n";
        assert_eq!(parse_df_available(linux), Some(374_425_864 * 1024));
        assert_eq!(parse_df_available("Filesystem only\n"), None);
        assert_eq!(parse_df_available(""), None);
        assert_eq!(parse_df_available("a b c\nx y z notanumber q\n"), None);

        // The temp dir exists on every machine the tests run on.
        assert!(free_disk_space(&std::env::temp_dir()).is_some_and(|n| n > 0));
        // A path that does not exist yet is measured at its nearest
        // existing ancestor.
        assert!(free_disk_space(&std::env::temp_dir().join("hm/does/not/exist")).is_some());
        // ... including a relative one with no existing component, which
        // is measured at the current directory rather than reported as
        // unmeasurable.
        assert!(free_disk_space(Path::new("hm-relative-does-not-exist/dumps")).is_some());
    }

    #[test]
    fn space_checks_say_how_much() {
        let c = space_check(
            "x",
            "/tmp",
            Some(10_000_000_000),
            Some(1_000_000_000),
            "fix",
        );
        assert_eq!(c.status, Status::Pass);
        assert!(c.detail.contains("10.0 GB free"), "{}", c.detail);
        assert!(c.detail.contains("needs about 1.0 GB"), "{}", c.detail);
        let c = space_check(
            "x",
            "/tmp",
            Some(500_000_000),
            Some(1_000_000_000),
            "use --cache-dir",
        );
        assert_eq!(c.status, Status::Fail);
        assert_eq!(c.fix.as_deref(), Some("use --cache-dir"));
        let c = space_check("x", "/tmp", None, Some(1_000_000_000), "fix");
        assert_eq!(c.status, Status::Warn);
        assert!(c.fix.as_deref().is_some_and(|f| f.contains("1.0 GB")));
        let c = space_check("x", "/tmp", Some(1), None, "fix");
        assert_eq!(c.status, Status::Pass);
        let c = check_cache_space(&std::env::temp_dir(), Some(1));
        assert_eq!(c.status, Status::Pass, "{c:?}");
        let c = check_cache_space(&std::env::temp_dir(), None);
        assert_eq!(c.status, Status::Pass, "{c:?}");
        assert!(c.detail.contains("free at"), "{c:?}");
    }

    #[test]
    fn estimates_scale_with_size_and_cycles() {
        let today = Cycle::new(2026).unwrap();
        // Schedule E: 43 MB, whole, downloaded.
        let e = estimate(
            &dump::SCHEDULE_E,
            dump::SCHEDULE_E.approx_size_bytes,
            true,
            &[],
            false,
            today,
        );
        assert_eq!(e.download_bytes, dump::SCHEDULE_E.approx_size_bytes);
        assert!(e.database_bytes > 100_000_000 && e.database_bytes < 1_000_000_000);
        let (lo, hi) = e.download_time.unwrap();
        assert!(hi.as_secs() <= 60, "{hi:?}");
        assert!(lo <= hi);
        assert_eq!(e.restore_time.0.as_secs(), 30);
        assert_eq!(e.restore_time.1.as_secs(), 120);
        assert_eq!(fmt_duration_range(lo, hi), "under a minute");

        // Cached: nothing to download.
        let cached = estimate(&dump::SCHEDULE_E, 43_000_000, false, &[], false, today);
        assert_eq!(cached.download_bytes, 0);
        assert!(cached.download_time.is_none());

        // Schedule A, one recent cycle: the whole 90 GB downloads, but only
        // that cycle's share is restored.
        let a = estimate(
            &dump::SCHEDULE_A,
            dump::SCHEDULE_A.approx_size_bytes,
            true,
            &[today],
            false,
            today,
        );
        assert_eq!(a.download_bytes, dump::SCHEDULE_A.approx_size_bytes);
        let (dlo, dhi) = a.download_time.unwrap();
        assert!(dlo.as_secs() >= 3600, "{dlo:?}");
        assert!(dhi.as_secs() >= 10 * 3600, "{dhi:?}");
        // 30% of 90 GB * 5 = ~135 GB.
        assert!(a.database_bytes > 100_000_000_000 && a.database_bytes < 200_000_000_000);
        // 27 GB * 6 min/GB * 1.5 = ~4 h; range 2 to 8 h.
        assert!(a.restore_time.0.as_secs() > 3600, "{:?}", a.restore_time);
        assert!(
            a.restore_time.1.as_secs() < 12 * 3600,
            "{:?}",
            a.restore_time
        );
        // Two cycles cost more than one.
        let a2 = estimate(
            &dump::SCHEDULE_A,
            dump::SCHEDULE_A.approx_size_bytes,
            true,
            &[today, Cycle::new(2024).unwrap()],
            false,
            today,
        );
        assert!(a2.database_bytes > a.database_bytes);
        assert!(a2.restore_time.1 > a.restore_time.1);
        // Whole archive with the FEC's indexes is the big one.
        let a_all = estimate(
            &dump::SCHEDULE_A,
            dump::SCHEDULE_A.approx_size_bytes,
            false,
            &[],
            false,
            today,
        );
        assert!(a_all.database_bytes > 2_000_000_000_000);
        assert!(a_all.restore_time.1.as_secs() > 48 * 3600);

        // Summing.
        let both = e.plus(&cached);
        assert_eq!(both.download_bytes, e.download_bytes);
        assert_eq!(
            both.database_bytes,
            e.database_bytes + cached.database_bytes
        );
        assert_eq!(
            both.total_bytes(),
            both.download_bytes + both.database_bytes
        );
        assert_eq!(
            both.total_time().1,
            e.total_time().1 + cached.restore_time.1
        );

        assert_eq!(cycle_share_percent(Cycle::new(2026).unwrap(), today), 30);
        assert_eq!(cycle_share_percent(Cycle::new(2024).unwrap(), today), 30);
        assert_eq!(cycle_share_percent(Cycle::new(2022).unwrap(), today), 15);
        assert_eq!(cycle_share_percent(Cycle::new(2020).unwrap(), today), 15);
        assert_eq!(cycle_share_percent(Cycle::new(2000).unwrap(), today), 5);
        assert_eq!(cycle_share_percent(Cycle::new(2028).unwrap(), today), 30);
    }

    #[test]
    fn human_formats() {
        assert_eq!(fmt_count(0), "0");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1_000), "1,000");
        assert_eq!(fmt_count(549_525), "549,525");
        assert_eq!(fmt_count(90_181_919_946), "90,181,919,946");
        assert_eq!(fmt_rough_count(549_525), "about 550,000");
        assert_eq!(fmt_rough_count(242_346), "about 250,000");
        assert_eq!(fmt_rough_count(1_234), "about 1,300");
        assert_eq!(fmt_rough_count(42), "about 42");

        let s = Duration::from_secs;
        assert_eq!(fmt_rough_duration(s(10)), "under a minute");
        assert_eq!(fmt_rough_duration(s(90)), "2 minutes");
        assert_eq!(fmt_rough_duration(s(60)), "1 minute");
        assert_eq!(fmt_rough_duration(s(3 * 3600)), "3 hours");
        assert_eq!(fmt_rough_duration(s(3 * 86_400)), "3 days");
        assert_eq!(fmt_duration_range(s(2), s(20)), "under a minute");
        assert_eq!(fmt_duration_range(s(30), s(120)), "about 1 to 2 minutes");
        assert_eq!(fmt_duration_range(s(20), s(120)), "up to 2 minutes");
        assert_eq!(fmt_duration_range(s(60), s(180)), "about 1 to 3 minutes");
        assert_eq!(
            fmt_duration_range(s(3600), s(3 * 3600)),
            "about 1 to 3 hours"
        );
        assert_eq!(
            fmt_duration_range(s(2 * 3600), s(2 * 3600)),
            "about 2 hours"
        );
        assert_eq!(
            fmt_duration_range(s(86_400), s(4 * 86_400)),
            "about 1 to 4 days"
        );
        assert_eq!(fmt_elapsed(s(58)), "58 s");
        assert_eq!(fmt_elapsed(s(724)), "12 min 4 s");
        assert_eq!(fmt_elapsed(s(3 * 3600 + 120)), "3 h 2 min");
    }

    #[test]
    fn preflight_display_and_summary() {
        let mut pf = Preflight::new();
        assert_eq!(pf.worst(), Status::Pass);
        assert!(pf.passed());
        pf.push(Check::pass("pg_restore", "version 18.3 is installed"));
        pf.push(Check::warn(
            "extensions",
            "pg_trgm missing",
            "install contrib",
        ));
        assert_eq!(pf.worst(), Status::Warn);
        assert!(pf.passed());
        pf.push(Check::fail(
            "database",
            "none configured",
            "createdb fec\nexport DATABASE_URL=...",
        ));
        assert_eq!(pf.worst(), Status::Fail);
        assert!(!pf.passed());
        assert_eq!(pf.failures().count(), 1);
        assert_eq!(pf.get("extensions").map(|c| c.status), Some(Status::Warn));
        assert!(pf.get("nope").is_none());
        let text = pf.to_string();
        assert!(
            text.contains("✓ pg_restore: version 18.3 is installed"),
            "{text}"
        );
        assert!(
            text.contains("! extensions: pg_trgm missing\n  fix: install contrib"),
            "{text}"
        );
        assert!(
            text.contains(
                "✗ database: none configured\n  fix: createdb fec\n       export DATABASE_URL=..."
            ),
            "{text}"
        );
        let json = serde_json::to_value(&pf).unwrap();
        assert_eq!(json["checks"][2]["status"], "fail");
        assert_eq!(json["checks"][0]["fix"], serde_json::Value::Null);
    }

    #[test]
    fn url_checks_and_redaction() {
        let c = check_database_url(None);
        assert_eq!(c.status, Status::Fail);
        assert!(c.fix.as_deref().is_some_and(|f| f.contains("createdb fec")));
        assert!(
            c.fix
                .as_deref()
                .is_some_and(|f| f.contains("export DATABASE_URL=postgres://"))
        );
        let c = check_database_url(Some("postgres://alice:pw@db.example.org:5433/fec"));
        assert_eq!(c.status, Status::Pass);
        assert!(
            c.detail
                .contains("fec on db.example.org:5433 as user alice"),
            "{}",
            c.detail
        );
        let c = check_database_url(Some("not a url at all ://"));
        assert_eq!(c.status, Status::Fail);

        let s = summarize_url("postgres://alice:pw@db.example.org:5433/fec").unwrap();
        assert_eq!(s.host, "db.example.org");
        assert_eq!(s.port, 5433);
        assert_eq!(s.user, "alice");
        assert_eq!(s.database, "fec");
        assert!(!s.local);
        let s = summarize_url("postgres://bob@localhost/fec").unwrap();
        assert!(s.local);
        assert_eq!(s.port, 5432);
        let s = summarize_url("postgres://bob@127.0.0.1/").unwrap();
        assert!(s.local);
        assert_eq!(
            s.database, "bob",
            "Postgres defaults the database to the user"
        );

        assert_eq!(
            redact_url("postgres://alice:secret@host:5432/db?sslmode=require"),
            "postgres://alice:***@host:5432/db?sslmode=require"
        );
        assert_eq!(
            redact_url("postgres://alice@host/db"),
            "postgres://alice@host/db"
        );
        assert_eq!(redact_url("postgres://host/db"), "postgres://host/db");
        assert_eq!(
            redact_url("postgres://a:p%40ss@h/d"),
            "postgres://a:***@h/d"
        );
        assert_eq!(redact_url("nonsense"), "nonsense");
    }

    #[test]
    fn strip_password_moves_the_password_out_of_the_url_decoded() {
        assert_eq!(
            strip_password("postgres://alice:secret@host:5432/db?sslmode=require"),
            (
                "postgres://alice@host:5432/db?sslmode=require".to_string(),
                Some("secret".to_string())
            )
        );
        // Percent-escapes are decoded to what libpq wants in PGPASSWORD.
        assert_eq!(
            strip_password("postgres://a:p%40ss%3Aw%2Frd@h/d"),
            (
                "postgres://a@h/d".to_string(),
                Some("p@ss:w/rd".to_string())
            )
        );
        // A lone or truncated `%` is kept as is.
        assert_eq!(
            strip_password("postgres://a:100%@h/d"),
            ("postgres://a@h/d".to_string(), Some("100%".to_string()))
        );
        assert_eq!(
            strip_password("postgres://a:%4@h/d"),
            ("postgres://a@h/d".to_string(), Some("%4".to_string()))
        );
        // No password: unchanged.
        assert_eq!(
            strip_password("postgres://alice@host/db"),
            ("postgres://alice@host/db".to_string(), None)
        );
        assert_eq!(
            strip_password("postgres://host/db"),
            ("postgres://host/db".to_string(), None)
        );
        // An empty password is dropped from the URL and not reported.
        assert_eq!(
            strip_password("postgres://alice:@host/db"),
            ("postgres://alice@host/db".to_string(), None)
        );
        // `@` in the path or query is not the userinfo separator.
        assert_eq!(
            strip_password("postgres://host/db?application_name=a@b"),
            ("postgres://host/db?application_name=a@b".to_string(), None)
        );
        assert_eq!(strip_password("nonsense"), ("nonsense".to_string(), None));
    }

    #[test]
    fn connection_errors_are_explained() {
        let s = summarize_url("postgres://alice@localhost:5432/fec");
        let (what, fix) = explain_connect_error(
            s.as_ref(),
            "error communicating with database: Connection refused (os error 61)",
        );
        assert!(what.contains("not running at localhost:5432"), "{what}");
        assert!(fix.contains("brew services start"), "{fix}");
        let (what, fix) = explain_connect_error(
            s.as_ref(),
            "error returned from database: database \"fec\" does not exist",
        );
        assert!(what.contains("\"fec\" does not exist"), "{what}");
        assert_eq!(fix, "create it: createdb fec");
        let (what, fix) = explain_connect_error(
            s.as_ref(),
            "error returned from database: role \"alice\" does not exist",
        );
        assert!(what.contains("no Postgres user named \"alice\""), "{what}");
        assert!(fix.contains("createuser"), "{fix}");
        let (what, fix) = explain_connect_error(
            s.as_ref(),
            "error returned from database: password authentication failed for user \"alice\"",
        );
        assert!(what.contains("did not accept the login"), "{what}");
        assert!(
            fix.contains("postgres://alice:PASSWORD@localhost:5432/fec"),
            "{fix}"
        );
        let (what, _) = explain_connect_error(s.as_ref(), "pool timed out while waiting");
        assert!(what.contains("timed out"), "{what}");
        let (what, _) = explain_connect_error(None, "something odd");
        assert!(what.contains("could not connect"), "{what}");
    }

    #[test]
    fn pg_restore_check_matches_the_machine() {
        // Whatever this machine has, the check must describe it without
        // panicking and give a fix when it is not a pass.
        let c = check_pg_restore();
        assert_eq!(c.name, "pg_restore");
        match pg_restore_version() {
            Ok(v) if v.can_read_dumps() => {
                assert_eq!(c.status, Status::Pass, "{c:?}");
                assert!(c.detail.contains(&v.to_string()));
            }
            Ok(_) => assert_eq!(c.status, Status::Fail, "{c:?}"),
            Err(ToolVersionError::NotRunnable { .. }) => {
                assert_eq!(c.status, Status::Fail, "{c:?}");
                assert!(c.fix.as_deref().is_some_and(|f| f.contains("install")));
            }
            Err(ToolVersionError::Unparsable { .. }) => assert_eq!(c.status, Status::Warn),
        }
        assert!(!pg_restore_install_hint().is_empty());
        assert!(suggest_database_setup().contains("createdb fec"));
        assert!(!local_username().is_empty());
    }
}
