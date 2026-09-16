//! The fixture corpus as a whole: which real-world regimes it covers, and
//! that its bytes are exactly the ones committed.
//!
//! The FEC's own files arrive both CRLF- and LF-terminated, FS- (0x1C) and
//! comma-delimited, ASCII and Windows-1252 or UTF-8, with and without
//! `[BEGINTEXT]` blocks. Those regimes have to stay represented here, and
//! nothing between the repository and the parser -- a checkout with
//! `core.autocrlf=true`, an editor, a copy tool -- may rewrite a fixture.
//! `.gitattributes` marks `tests/fixtures/**` as `-text` so git never
//! converts them, and `tests/fixtures/MANIFEST.sha256` records the
//! committed bytes so a rewrite fails here instead of showing up as a
//! parser "regression" on one contributor's machine.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use hardmoney::{Filing, ParseOptions};

// The golden pack's SHA-256 (checked against the standard vectors in
// `tests/golden_fixtures.rs`), reused so the manifest check needs no
// extra dependency. Declared here because `#[path]` inside an inline
// module resolves through a directory that does not exist.
#[cfg(feature = "serde")]
#[allow(dead_code)]
#[path = "../examples/golden_fixtures.rs"]
mod golden;

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

/// Every file under `tests/fixtures/`, sorted, except the manifest itself.
fn fixture_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    walk(&fixtures_dir(), &mut files);
    files.retain(|p| p.file_name().is_some_and(|n| n != "MANIFEST.sha256"));
    files.sort();
    files
}

/// Every `.fec` under `tests/fixtures/`, including `chain/`, `rad/`, and
/// `golden/`.
fn fec_fixtures() -> Vec<PathBuf> {
    let mut files = fixture_files();
    files.retain(|p| p.extension().is_some_and(|x| x == "fec"));
    files
}

/// `path` relative to the crate root with `/` separators, as the manifest
/// and `git ls-files` spell it on every platform.
fn repo_path(path: &Path) -> String {
    path.strip_prefix(env!("CARGO_MANIFEST_DIR"))
        .unwrap()
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/")
}

fn has_crlf(bytes: &[u8]) -> bool {
    bytes.windows(2).any(|w| w == b"\r\n")
}

/// `bytes` with every CRLF collapsed to LF.
fn to_lf(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
            i += 1;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// `bytes` (already LF-only) with every LF expanded to CRLF.
fn to_crlf(lf: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(lf.len() + lf.len() / 40);
    for &b in lf {
        if b == b'\n' {
            out.push(b'\r');
        }
        out.push(b);
    }
    out
}

/// The corpus must keep covering the line-ending and delimiter regimes
/// real filings come in, so a change that only handles one of them cannot
/// pass the suite. The floors are well under the current counts (9 CRLF+FS,
/// 47 LF+FS, 5 LF+comma, 3 non-ASCII, 4 with a `[BEGINTEXT]` block) so
/// that pruning a few fixtures stays possible.
#[test]
fn corpus_covers_the_line_ending_and_delimiter_regimes_the_fec_produces() {
    let mut crlf_fs = 0;
    let mut lf_fs = 0;
    let mut lf_comma = 0;
    let mut crlf_comma = 0;
    let mut non_ascii = 0;
    let mut begintext = 0;
    for path in fec_fixtures() {
        let bytes = std::fs::read(&path).unwrap();
        let name = repo_path(&path);
        // The signature of a CRLF file checked out through a second
        // CRLF conversion (core.autocrlf=true on a file git thought was
        // text): unambiguous corruption, never something the FEC sends.
        assert!(
            !bytes.windows(3).any(|w| w == b"\r\r\n"),
            "{name}: contains CR CR LF; the fixture was rewritten on checkout (see .gitattributes)"
        );
        // The header line decides the delimiter for the whole file.
        let header_end = bytes
            .iter()
            .position(|&b| b == b'\n')
            .unwrap_or(bytes.len());
        let fs = bytes[..header_end].contains(&0x1c);
        match (has_crlf(&bytes), fs) {
            (true, true) => crlf_fs += 1,
            (false, true) => lf_fs += 1,
            (false, false) => lf_comma += 1,
            (true, false) => crlf_comma += 1,
        }
        if bytes.iter().any(|&b| b > 0x7f) {
            non_ascii += 1;
        }
        if bytes
            .windows(11)
            .any(|w| w.eq_ignore_ascii_case(b"[BEGINTEXT]"))
        {
            begintext += 1;
        }
    }
    eprintln!(
        "corpus: {crlf_fs} CRLF+FS, {lf_fs} LF+FS, {lf_comma} LF+comma, {crlf_comma} CRLF+comma; \
         {non_ascii} non-ASCII; {begintext} with [BEGINTEXT]"
    );
    assert!(
        crlf_fs >= 5,
        "only {crlf_fs} CRLF-terminated FS-delimited fixtures"
    );
    assert!(
        lf_fs >= 20,
        "only {lf_fs} LF-terminated FS-delimited fixtures"
    );
    assert!(
        lf_comma >= 3,
        "only {lf_comma} comma-delimited (pre-6.x) fixtures"
    );
    assert!(non_ascii >= 1, "no fixture with non-ASCII bytes");
    assert!(
        begintext >= 2,
        "only {begintext} fixtures with a [BEGINTEXT] block"
    );
}

/// A filing means the same thing however its lines are terminated: the
/// FEC accepts both, filers' software differs, and a file that crossed a
/// Windows machine may have been converted in transit. Every fixture,
/// re-terminated both ways, must parse to the same header, cover, lines,
/// and skipped-line count -- strictly and leniently.
#[test]
fn every_fixture_parses_identically_with_crlf_and_lf_line_endings() {
    let mut checked = 0;
    for path in fec_fixtures() {
        let name = repo_path(&path);
        let original = std::fs::read(&path).unwrap();
        let lf = to_lf(&original);
        let crlf = to_crlf(&lf);
        assert_eq!(to_lf(&crlf), lf, "{name}: re-termination is not reversible");

        for (label, options) in [
            ("strict", ParseOptions::STRICT),
            ("lenient", ParseOptions::LENIENT),
        ] {
            let a = Filing::parse_bytes_with(&lf, &options);
            let b = Filing::parse_bytes_with(&crlf, &options);
            match (a, b) {
                (Ok(a), Ok(b)) => {
                    let (fa, skipped_a) = a.into_parts();
                    let (fb, skipped_b) = b.into_parts();
                    assert_eq!(fa.header, fb.header, "{name} ({label}): header");
                    assert_eq!(fa.version, fb.version, "{name} ({label}): version");
                    assert_eq!(
                        fa.raw_form_type, fb.raw_form_type,
                        "{name} ({label}): form type"
                    );
                    assert_eq!(fa.summary, fb.summary, "{name} ({label}): cover");
                    assert_eq!(
                        fa.lines.len(),
                        fb.lines.len(),
                        "{name} ({label}): line count"
                    );
                    for (x, y) in fa.lines.iter().zip(&fb.lines) {
                        assert_eq!(x, y, "{name} ({label}): line {}", x.line_no);
                    }
                    assert_eq!(
                        skipped_a.len(),
                        skipped_b.len(),
                        "{name} ({label}): skipped lines"
                    );
                }
                (Err(a), Err(b)) => {
                    assert_eq!(a.line_no(), b.line_no(), "{name} ({label}): error line");
                }
                (a, b) => panic!(
                    "{name} ({label}): LF parse ok={} but CRLF parse ok={}",
                    a.is_ok(),
                    b.is_ok()
                ),
            }
        }
        checked += 1;
    }
    assert!(checked >= 40, "only {checked} fixtures checked");
}

/// The bytes on disk are the bytes that were committed. Uses the golden
/// pack's SHA-256 (checked against the standard vectors in
/// `tests/golden_fixtures.rs`), so no extra dependency.
#[cfg(feature = "serde")]
mod manifest {
    use super::*;

    use super::golden::pack;

    fn committed() -> BTreeMap<String, String> {
        let text = std::fs::read_to_string(fixtures_dir().join("MANIFEST.sha256")).unwrap();
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let (sha, path) = l
                    .split_once("  ")
                    .unwrap_or_else(|| panic!("malformed manifest line: {l:?}"));
                (path.to_string(), sha.to_string())
            })
            .collect()
    }

    #[test]
    fn fixture_bytes_match_the_committed_manifest() {
        let manifest = committed();
        let on_disk: BTreeMap<String, String> = fixture_files()
            .iter()
            .map(|p| (repo_path(p), pack::sha256_hex(&std::fs::read(p).unwrap())))
            .collect();

        let mut problems = Vec::new();
        for (path, sha) in &manifest {
            match on_disk.get(path) {
                None => problems.push(format!("{path}: in the manifest but not on disk")),
                Some(actual) if actual != sha => problems.push(format!(
                    "{path}: bytes differ from the committed file (was it rewritten on checkout or by an editor?)"
                )),
                Some(_) => {}
            }
        }
        for path in on_disk.keys() {
            if !manifest.contains_key(path) {
                problems.push(format!(
                    "{path}: not in tests/fixtures/MANIFEST.sha256 (new fixture? add its SHA-256 there)"
                ));
            }
        }
        assert!(
            problems.is_empty(),
            "{} fixture problem(s):\n  {}\n(regenerate the manifest on purpose with the command in its header)",
            problems.len(),
            problems.join("\n  ")
        );
        assert!(
            manifest.len() >= 60,
            "only {} manifest entries",
            manifest.len()
        );
    }
}
