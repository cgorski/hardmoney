//! The on-disk download cache shared by everything in [`crate::fec`] that
//! fetches from the FEC.
//!
//! Layout under the cache root (`HARDMONEY_CACHE_DIR`, else
//! `$XDG_CACHE_HOME/hardmoney`, else `~/.cache/hardmoney`, else the system
//! temp dir -- the same resolution `bulk-restore-dump` uses):
//!
//! ```text
//! <root>/
//!   filings/<id>.fec        raw filings, exactly as downloaded
//!   efile/YYYYMMDD.zip      daily e-filing archives (backfill)
//!   efile-seen.txt          filing ids `hardmoney efile watch` has processed
//! ```
//!
//! Writes are atomic (temp file + rename), so an interrupted download
//! never leaves a truncated file that a later run would trust. Nothing
//! here deletes anything it did not create: [`Cache::clear`] removes only
//! the three entries above and leaves `bulk`'s `dumps/` alone.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::NaiveDate;

/// A cache root and the paths under it. Cheap to clone; creates
/// directories lazily on first write.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    root: PathBuf,
}

/// What [`Cache::info`] and [`Cache::clear`] report.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
#[non_exhaustive]
pub struct CacheInfo {
    /// The cache root.
    pub root: PathBuf,
    /// Number of cached raw filings.
    pub filings: u64,
    /// Their total size in bytes.
    pub filing_bytes: u64,
    /// Number of cached daily e-filing archives.
    pub daily_zips: u64,
    /// Their total size in bytes.
    pub daily_zip_bytes: u64,
    /// Number of filing ids in the e-file seen list.
    pub seen_ids: u64,
}

impl CacheInfo {
    /// Bytes across everything the cache holds.
    #[must_use]
    pub fn total_bytes(&self) -> u64 {
        self.filing_bytes.saturating_add(self.daily_zip_bytes)
    }
}

impl Cache {
    /// The environment variable that overrides the cache root.
    pub const ENV_VAR: &'static str = "HARDMONEY_CACHE_DIR";

    /// The cache at `root`.
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Cache { root: root.into() }
    }

    /// The cache root the environment selects: `HARDMONEY_CACHE_DIR` if
    /// set and non-empty, else `$XDG_CACHE_HOME/hardmoney`, else
    /// `$HOME/.cache/hardmoney`, else `<temp dir>/hardmoney`.
    ///
    /// Never fails: with no usable variable at all it falls back to the
    /// temp dir rather than refusing to work.
    #[must_use]
    pub fn from_env() -> Self {
        if let Some(dir) = std::env::var_os(Self::ENV_VAR).filter(|v| !v.is_empty()) {
            return Cache::at(dir);
        }
        let base = std::env::var_os("XDG_CACHE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME")
                    .filter(|v| !v.is_empty())
                    .map(|h| PathBuf::from(h).join(".cache"))
            })
            .unwrap_or_else(std::env::temp_dir);
        Cache::at(base.join("hardmoney"))
    }

    /// The cache root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// `<root>/filings`.
    #[must_use]
    pub fn filings_dir(&self) -> PathBuf {
        self.root.join("filings")
    }

    /// `<root>/efile`, where daily archives live.
    #[must_use]
    pub fn daily_zips_dir(&self) -> PathBuf {
        self.root.join("efile")
    }

    /// `<root>/filings/<id>.fec`.
    #[must_use]
    pub fn filing_path(&self, filing_id: u64) -> PathBuf {
        self.filings_dir().join(format!("{filing_id}.fec"))
    }

    /// `<root>/efile/YYYYMMDD.zip`.
    #[must_use]
    pub fn daily_zip_path(&self, date: NaiveDate) -> PathBuf {
        self.daily_zips_dir()
            .join(format!("{}.zip", date.format("%Y%m%d")))
    }

    /// `<root>/efile-seen.txt`.
    #[must_use]
    pub fn seen_path(&self) -> PathBuf {
        self.root.join("efile-seen.txt")
    }

    /// The cached bytes of a filing, or `None` if it is not cached.
    pub fn read_filing(&self, filing_id: u64) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.filing_path(filing_id)) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Stores a filing's bytes atomically and returns the path written.
    pub fn write_filing(&self, filing_id: u64, bytes: &[u8]) -> io::Result<PathBuf> {
        let path = self.filing_path(filing_id);
        write_atomically(&path, |f| f.write_all(bytes))?;
        Ok(path)
    }

    /// The ids recorded as seen by the e-file watcher. A missing file is an
    /// empty set; lines that are not a number are ignored.
    pub fn read_seen(&self) -> io::Result<BTreeSet<u64>> {
        let file = match fs::File::open(self.seen_path()) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(BTreeSet::new()),
            Err(e) => return Err(e),
        };
        let mut seen = BTreeSet::new();
        for line in io::BufReader::new(file).lines() {
            if let Ok(id) = line?.trim().parse::<u64>() {
                seen.insert(id);
            }
        }
        Ok(seen)
    }

    /// Appends ids to the seen list (one per line), creating it if needed.
    pub fn append_seen(&self, ids: impl IntoIterator<Item = u64>) -> io::Result<()> {
        fs::create_dir_all(&self.root)?;
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.seen_path())?;
        let mut out = String::new();
        for id in ids {
            out.push_str(&id.to_string());
            out.push('\n');
        }
        file.write_all(out.as_bytes())?;
        file.sync_all()
    }

    /// Forgets every seen id. Not an error if there were none.
    pub fn clear_seen(&self) -> io::Result<()> {
        remove_file_if_exists(&self.seen_path())
    }

    /// Counts and sizes of what is cached. A cache root that does not
    /// exist yet reports zeros.
    pub fn info(&self) -> io::Result<CacheInfo> {
        let (filings, filing_bytes) = dir_stats(&self.filings_dir(), "fec")?;
        let (daily_zips, daily_zip_bytes) = dir_stats(&self.daily_zips_dir(), "zip")?;
        let seen_ids = u64::try_from(self.read_seen()?.len()).unwrap_or(u64::MAX);
        Ok(CacheInfo {
            root: self.root.clone(),
            filings,
            filing_bytes,
            daily_zips,
            daily_zip_bytes,
            seen_ids,
        })
    }

    /// Deletes cached filings and daily archives (and, with
    /// `including_seen`, the seen list), returning what was there before.
    /// Other files under the root are untouched.
    pub fn clear(&self, including_seen: bool) -> io::Result<CacheInfo> {
        let before = self.info()?;
        remove_dir_if_exists(&self.filings_dir())?;
        remove_dir_if_exists(&self.daily_zips_dir())?;
        if including_seen {
            self.clear_seen()?;
        }
        Ok(before)
    }
}

/// Writes `dest` via a sibling `.partial` file and a rename, creating
/// parent directories as needed.
pub(crate) fn write_atomically(
    dest: &Path,
    write: impl FnOnce(&mut fs::File) -> io::Result<()>,
) -> io::Result<()> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut name = dest
        .file_name()
        .map(std::ffi::OsString::from)
        .unwrap_or_default();
    name.push(".partial");
    let tmp = dest.with_file_name(name);
    let result = (|| {
        let mut file = fs::File::create(&tmp)?;
        write(&mut file)?;
        file.sync_all()?;
        fs::rename(&tmp, dest)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn dir_stats(dir: &Path, extension: &str) -> io::Result<(u64, u64)> {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok((0, 0)),
        Err(e) => return Err(e),
    };
    let mut count = 0u64;
    let mut bytes = 0u64;
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some(extension) {
            continue;
        }
        let meta = entry.metadata()?;
        if meta.is_file() {
            count = count.saturating_add(1);
            bytes = bytes.saturating_add(meta.len());
        }
    }
    Ok((count, bytes))
}

fn remove_dir_if_exists(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

fn remove_file_if_exists(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache(tag: &str) -> Cache {
        let dir =
            std::env::temp_dir().join(format!("hardmoney-cache-test-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Cache::at(dir)
    }

    #[test]
    fn paths_follow_the_layout() {
        let c = Cache::at("/tmp/hm");
        assert_eq!(
            c.filing_path(2011915),
            Path::new("/tmp/hm/filings/2011915.fec")
        );
        assert_eq!(
            c.daily_zip_path(NaiveDate::from_ymd_opt(2026, 9, 6).unwrap()),
            Path::new("/tmp/hm/efile/20260906.zip")
        );
        assert_eq!(c.seen_path(), Path::new("/tmp/hm/efile-seen.txt"));
    }

    #[test]
    fn write_read_info_clear_round_trip() {
        let c = temp_cache("rt");
        assert_eq!(c.read_filing(1).unwrap(), None);
        assert_eq!(c.info().unwrap().filings, 0);

        c.write_filing(1, b"HDR").unwrap();
        c.write_filing(2, b"HDR\x1cFEC").unwrap();
        assert_eq!(c.read_filing(1).unwrap().as_deref(), Some(&b"HDR"[..]));
        c.append_seen([5, 7]).unwrap();
        c.append_seen([9]).unwrap();
        assert_eq!(
            c.read_seen().unwrap().into_iter().collect::<Vec<_>>(),
            [5, 7, 9]
        );

        let info = c.info().unwrap();
        assert_eq!((info.filings, info.filing_bytes, info.seen_ids), (2, 10, 3));
        assert!(!c.filings_dir().join("1.fec.partial").exists());

        let before = c.clear(false).unwrap();
        assert_eq!(before.filings, 2);
        assert_eq!(c.info().unwrap().filings, 0);
        assert_eq!(c.read_seen().unwrap().len(), 3);
        c.clear(true).unwrap();
        assert!(c.read_seen().unwrap().is_empty());
        let _ = fs::remove_dir_all(c.root());
    }

    #[test]
    fn failed_write_leaves_no_partial() {
        let c = temp_cache("fail");
        let path = c.filing_path(3);
        let err = write_atomically(&path, |_| Err(io::Error::other("boom"))).unwrap_err();
        assert_eq!(err.to_string(), "boom");
        assert!(!path.exists());
        assert!(!c.filings_dir().join("3.fec.partial").exists());
        let _ = fs::remove_dir_all(c.root());
    }
}
