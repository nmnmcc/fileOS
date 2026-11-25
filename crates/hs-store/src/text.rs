// SPDX-License-Identifier: AGPL-3.0-only

//! Extracting what the index needs from a resource's JSON, and making
//! Chinese, Japanese and Korean text searchable with FTS5.
//!
//! FTS5's `unicode61` tokenizer treats a run of CJK characters as one token,
//! and `trigram` cannot find two-character words. So before indexing and
//! querying, runs of CJK characters are split into overlapping two-character
//! pieces (bigrams) separated by spaces; other text is left to `unicode61`.
//! The index is contentless, so the original text is stored once, in the
//! resource's body.

use serde_json::Value as Json;

fn is_cjk(c: char) -> bool {
    matches!(c as u32,
        0x3040..=0x30ff     // Hiragana, Katakana
        | 0x3400..=0x4dbf   // CJK Extension A
        | 0x4e00..=0x9fff   // CJK Unified Ideographs
        | 0xac00..=0xd7af   // Hangul syllables
        | 0xf900..=0xfaff   // CJK Compatibility Ideographs
        | 0x20000..=0x3134f // Extensions B to G
    )
}

/// Split runs of CJK characters into space-separated bigrams.
pub fn bigrams(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 2);
    let mut run: Vec<char> = Vec::new();
    let flush = |run: &mut Vec<char>, out: &mut String| {
        match run.len() {
            0 => {}
            1 => {
                out.push(' ');
                out.push(run[0]);
                out.push(' ');
            }
            _ => {
                for w in run.windows(2) {
                    out.push(' ');
                    out.push(w[0]);
                    out.push(w[1]);
                }
                out.push(' ');
            }
        }
        run.clear();
    };
    for c in text.chars() {
        if is_cjk(c) {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// Turn what a person typed into a safe FTS5 query: every term must match.
/// CJK terms become bigram phrases; other words match as prefixes.
pub fn fts_query(input: &str) -> Option<String> {
    let mut terms = Vec::new();
    for word in bigrams(input).split(|c: char| !c.is_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        let quoted = format!("\"{}\"", word.replace('"', ""));
        if word.chars().any(is_cjk) {
            terms.push(quoted);
        } else {
            terms.push(format!("{quoted}*"));
        }
    }
    (!terms.is_empty()).then(|| terms.join(" "))
}

/// A title for lists.
pub fn title(json: &Json) -> String {
    for key in ["title", "name", "fn", "summary", "subject"] {
        if let Some(s) = json.get(key).and_then(Json::as_str) {
            return s.chars().take(200).collect();
        }
    }
    json.get("text")
        .and_then(Json::as_str)
        .map(|s| s.chars().take(80).collect())
        .unwrap_or_default()
}

/// A title for text resources: a vCard's `FN`, an iCalendar `SUMMARY`, a
/// Markdown heading or the first line.
pub fn text_title(text: &str) -> String {
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        for prefix in ["FN:", "SUMMARY:"] {
            if let Some(v) = line.strip_prefix(prefix) {
                return v.chars().take(200).collect();
            }
        }
    }
    let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    first
        .trim_start_matches('#')
        .trim()
        .chars()
        .take(80)
        .collect()
}

/// All text worth searching, from string fields at any depth (not links,
/// hashes or IDs).
pub fn searchable(json: &Json) -> String {
    fn walk(v: &Json, key: &str, out: &mut String) {
        if out.len() > 64 * 1024 {
            return;
        }
        match v {
            Json::String(s)
                if !matches!(
                    key,
                    "href" | "hash" | "id" | "type" | "rel" | "media_type" | "time"
                ) =>
            {
                out.push_str(s);
                out.push('\n');
            }
            Json::Array(a) => a.iter().for_each(|x| walk(x, key, out)),
            Json::Object(o) => o
                .iter()
                .filter(|(k, _)| k.as_str() != "links")
                .for_each(|(k, x)| walk(x, k, out)),
            _ => {}
        }
    }
    let mut out = String::new();
    walk(json, "", &mut out);
    out
}

/// The time a resource is about, from its `time` field: milliseconds, or an
/// ISO 8601 date or date-time in UTC.
pub fn time_of(json: &Json) -> Option<u64> {
    match json.get("time")? {
        Json::Number(n) => n.as_u64(),
        Json::String(s) => parse_iso(s),
        _ => None,
    }
}

/// Parse `YYYY-MM-DD[THH:MM[:SS][.fff]][Z]` (UTC) to milliseconds.
pub fn parse_iso(s: &str) -> Option<u64> {
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, m, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if s.get(4..5) != Some("-")
        || s.get(7..8) != Some("-")
        || !(1..=12).contains(&m)
        || !(1..=31).contains(&d)
    {
        return None;
    }
    let (mut hh, mut mm, mut ss) = (0, 0, 0);
    if s.len() >= 16 && matches!(s.get(10..11), Some("T" | " ")) {
        hh = num(11, 13)?;
        mm = num(14, 16)?;
        if s.get(16..17) == Some(":") {
            ss = num(17, 19)?;
        }
    }
    // Days from civil (Howard Hinnant).
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let ms = ((days * 86400 + hh * 3600 + mm * 60 + ss) * 1000).max(0);
    Some(ms as u64)
}

/// A place: `place: {lat, lon}`.
pub fn place(json: &Json) -> Option<(f64, f64)> {
    let p = json.get("place")?;
    let lat = p.get("lat")?.as_f64()?;
    let lon = p.get("lon")?.as_f64()?;
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

/// Links: `links: [{rel, href}]`, where `href` is `type/id` in the same space
/// or `space:<hex>/type/id`.
pub fn links(json: &Json, space: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    for l in json
        .get("links")
        .and_then(Json::as_array)
        .into_iter()
        .flatten()
    {
        let rel = l.get("rel").and_then(Json::as_str).unwrap_or("related");
        let Some(href) = l.get("href").and_then(Json::as_str) else {
            continue;
        };
        let (dst_space, dst) = match href.strip_prefix("space:").and_then(|r| r.split_once('/')) {
            Some((s, t)) => (s.to_string(), t.to_string()),
            None => (space.to_string(), href.to_string()),
        };
        if hs_core::valid_target(&dst) {
            out.push((rel.to_string(), dst_space, dst));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bigram_split() {
        assert_eq!(
            bigrams("人民公园").split_whitespace().collect::<Vec<_>>(),
            ["人民", "民公", "公园"]
        );
        assert_eq!(
            bigrams("去a").split_whitespace().collect::<Vec<_>>(),
            ["去", "a"]
        );
        assert_eq!(fts_query("公园 walk").unwrap(), "\"公园\" \"walk\"*");
        assert_eq!(fts_query("   "), None);
    }

    #[test]
    fn iso_times() {
        assert_eq!(parse_iso("1970-01-01"), Some(0));
        assert_eq!(parse_iso("2000-03-01T00:00:00Z"), Some(951_868_800_000));
        assert_eq!(parse_iso("2024-13-01"), None);
    }
}
