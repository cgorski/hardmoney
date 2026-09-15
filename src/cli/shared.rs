//! Helpers more than one subcommand module needs: the default dump cache
//! directory and a fixed-width column renderer.

use std::path::PathBuf;

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
}
