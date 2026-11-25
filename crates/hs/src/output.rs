// SPDX-License-Identifier: AGPL-3.0-only

//! What commands print: aligned tables for people, JSON for programs
//! (`--json`).

use unicode_width::UnicodeWidthStr;

pub fn print_json<T: serde::Serialize + ?Sized>(v: &T) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

/// Print columns, each as wide as its widest cell (a wide character counts
/// as two), with a header row. Line breaks in cells become spaces.
pub fn table(headers: &[&str], rows: &[Vec<String>]) {
    print!("{}", render(headers, rows));
}

fn render(headers: &[&str], rows: &[Vec<String>]) -> String {
    let clean = |s: &str| s.replace(['\n', '\r', '\t'], " ");
    let rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| r.iter().map(|c| clean(c)).collect())
        .collect();
    let mut widths: Vec<usize> = headers.iter().map(|h| h.width()).collect();
    for r in &rows {
        for (w, c) in widths.iter_mut().zip(r) {
            *w = (*w).max(c.width());
        }
    }
    let mut out = String::new();
    let mut line = |cells: &mut dyn Iterator<Item = &str>| {
        let mut l = String::new();
        for (i, c) in cells.enumerate() {
            if i > 0 {
                l.push_str("  ");
            }
            l.push_str(c);
            if i + 1 < widths.len() {
                l.push_str(&" ".repeat(widths[i].saturating_sub(c.width())));
            }
        }
        out.push_str(l.trim_end());
        out.push('\n');
    };
    line(&mut headers.iter().copied());
    for r in &rows {
        line(&mut r.iter().map(String::as_str));
    }
    out
}

/// A time in milliseconds since 1970, as `2026-11-25 14:03` (UTC).
pub fn time(ms: u64) -> String {
    if ms == 0 {
        return String::new();
    }
    let format = time::macros::format_description!("[year]-[month]-[day] [hour]:[minute]");
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .ok()
        .and_then(|t| t.format(&format).ok())
        .unwrap_or_default()
}

/// A size such as `1.5 MB`.
pub fn size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
    let mut n = bytes as f64;
    let mut unit = 0;
    while n >= 1000.0 && unit + 1 < UNITS.len() {
        n /= 1000.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{n:.1} {}", UNITS[unit])
    }
}

/// The first `n` characters of a long ID, so tables stay narrow. Commands
/// take any start of an ID that fits only one.
pub fn short(id: &str, n: usize) -> String {
    id.chars().take(n).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_align_wide_characters() {
        let out = render(
            &["NAME", "N"],
            &[
                vec!["家".into(), "1".into()],
                vec!["home".into(), "22".into()],
            ],
        );
        assert_eq!(out, "NAME  N\n家    1\nhome  22\n");
    }

    #[test]
    fn times_and_sizes() {
        assert_eq!(time(0), "");
        assert_eq!(time(1_764_079_380_000), "2025-11-25 14:03");
        assert_eq!(size(999), "999 B");
        assert_eq!(size(1_500_000), "1.5 MB");
    }
}
