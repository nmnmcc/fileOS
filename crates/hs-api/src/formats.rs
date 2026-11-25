// SPDX-License-Identifier: AGPL-3.0-only

//! vCard (RFC 6350) and iCalendar (RFC 5545) to and from the JSON that
//! contacts and events are stored as.
//!
//! A contact: `{name, given, family, emails, phones, org, title, note,
//! birthday, address, url, uid, extra}`. An event: `{title, start, end,
//! all_day, location, description, rrule, uid, time, extra}`. Properties
//! not understood are kept as lines in `extra` and written back. So when
//! data goes to a `CardDAV` or `CalDAV` client and comes back, nothing the
//! client cares about is lost.

#[cfg(test)]
use serde_json::json;
use serde_json::{Map, Value as Json};
use time::format_description::BorrowedFormatItem;
use time::format_description::well_known::Rfc3339;
use time::macros::format_description;
use time::{Date, OffsetDateTime, PrimitiveDateTime, UtcOffset};

/// One content line: name, parameters, value (unescaped later as needed).
#[derive(Debug, Clone)]
struct Line {
    name: String,
    value: String,
    raw: String,
}

fn unfold(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for l in text.split('\n') {
        let l = l.strip_suffix('\r').unwrap_or(l);
        if let Some(rest) = l.strip_prefix([' ', '\t'])
            && let Some(last) = out.last_mut()
        {
            last.push_str(rest);
            continue;
        }
        if !l.is_empty() {
            out.push(l.to_string());
        }
    }
    out
}

fn parse(text: &str) -> Vec<Line> {
    unfold(text)
        .into_iter()
        .filter_map(|raw| {
            // The value starts at the first colon outside quotes.
            let mut quoted = false;
            let mut split = None;
            for (i, c) in raw.char_indices() {
                match c {
                    '"' => quoted = !quoted,
                    ':' if !quoted => {
                        split = Some(i);
                        break;
                    }
                    _ => {}
                }
            }
            let i = split?;
            let (head, value) = (&raw[..i], &raw[i + 1..]);
            let name = head.split_once(';').map_or(head, |(n, _)| n);
            let name = name.rsplit('.').next().unwrap_or(name).to_ascii_uppercase();
            Some(Line {
                name,
                value: value.to_string(),
                raw: raw.clone(),
            })
        })
        .collect()
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(o) => out.push(o),
                None => {}
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(',', "\\,")
        .replace(';', "\\;")
}

/// Fold a content line at 75 octets.
fn fold(line: &str, out: &mut String) {
    let mut width = 0;
    for c in line.chars() {
        let n = c.len_utf8();
        if width + n > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(c);
        width += n;
    }
    out.push_str("\r\n");
}

fn texts(v: &Json, key: &str) -> Vec<String> {
    match v.get(key) {
        Some(Json::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        Some(Json::String(s)) => vec![s.clone()],
        _ => Vec::new(),
    }
}

fn text<'a>(v: &'a Json, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Json::as_str).filter(|s| !s.is_empty())
}

fn uid_of(v: &Json, target: &str) -> String {
    text(v, "uid")
        .map(String::from)
        .unwrap_or_else(|| target.rsplit('/').next().unwrap_or(target).to_string())
}

/// A vCard 4.0 from a contact.
pub fn to_vcard(v: &Json, target: &str) -> String {
    let mut out = String::new();
    let mut add = |l: String| fold(&l, &mut out);
    add("BEGIN:VCARD".into());
    add("VERSION:4.0".into());
    add(format!("UID:{}", escape(&uid_of(v, target))));
    let name = text(v, "name").unwrap_or(target);
    add(format!("FN:{}", escape(name)));
    if text(v, "given").is_some() || text(v, "family").is_some() {
        add(format!(
            "N:{};{};;;",
            escape(text(v, "family").unwrap_or("")),
            escape(text(v, "given").unwrap_or(""))
        ));
    }
    for e in texts(v, "emails") {
        add(format!("EMAIL:{}", escape(&e)));
    }
    for p in texts(v, "phones") {
        add(format!("TEL:{}", escape(&p)));
    }
    for (k, prop) in [
        ("org", "ORG"),
        ("title", "TITLE"),
        ("note", "NOTE"),
        ("birthday", "BDAY"),
        ("url", "URL"),
    ] {
        if let Some(t) = text(v, k) {
            add(format!("{prop}:{}", escape(t)));
        }
    }
    if let Some(a) = text(v, "address") {
        add(format!("ADR:;;{};;;;", escape(a)));
    }
    for x in texts(v, "extra") {
        add(x);
    }
    add("END:VCARD".into());
    out
}

/// A contact from a vCard (the first one in the text).
pub fn from_vcard(text: &str) -> Json {
    let mut o = Map::new();
    let mut emails = Vec::new();
    let mut phones = Vec::new();
    let mut extra = Vec::new();
    for l in parse(text) {
        let val = unescape(&l.value);
        match l.name.as_str() {
            "BEGIN" | "END" | "VERSION" | "PRODID" | "REV" => {}
            "FN" => {
                o.insert("name".into(), val.into());
            }
            "N" => {
                let parts: Vec<&str> = l.value.split(';').collect();
                if let Some(f) = parts.first().filter(|s| !s.is_empty()) {
                    o.insert("family".into(), unescape(f).into());
                }
                if let Some(g) = parts.get(1).filter(|s| !s.is_empty()) {
                    o.insert("given".into(), unescape(g).into());
                }
            }
            "EMAIL" => emails.push(Json::from(val)),
            "TEL" => phones.push(Json::from(val.trim_start_matches("tel:").to_string())),
            "ORG" => {
                o.insert("org".into(), unescape(&l.value.replace(';', ", ")).into());
            }
            "TITLE" => {
                o.insert("title".into(), val.into());
            }
            "NOTE" => {
                o.insert("note".into(), val.into());
            }
            "BDAY" => {
                o.insert("birthday".into(), val.into());
            }
            "URL" => {
                o.insert("url".into(), val.into());
            }
            "UID" => {
                o.insert("uid".into(), val.into());
            }
            "ADR" => {
                let parts: Vec<String> = l
                    .value
                    .split(';')
                    .map(unescape)
                    .filter(|s| !s.is_empty())
                    .collect();
                o.insert("address".into(), parts.join(", ").into());
            }
            _ => extra.push(Json::from(l.raw.clone())),
        }
    }
    if !o.contains_key("name") {
        let n = [o.get("given"), o.get("family")]
            .into_iter()
            .flatten()
            .filter_map(Json::as_str)
            .collect::<Vec<_>>()
            .join(" ");
        o.insert("name".into(), n.into());
    }
    if !emails.is_empty() {
        o.insert("emails".into(), emails.into());
    }
    if !phones.is_empty() {
        o.insert("phones".into(), phones.into());
    }
    if !extra.is_empty() {
        o.insert("extra".into(), extra.into());
    }
    Json::Object(o)
}

const ICAL_DATE: &[BorrowedFormatItem<'_>] = format_description!("[year][month][day]");
const ICAL_TIME: &[BorrowedFormatItem<'_>] =
    format_description!("[year][month][day]T[hour][minute][second]");
const ISO_DATE: &[BorrowedFormatItem<'_>] = format_description!("[year]-[month]-[day]");
const ISO_TIME: &[BorrowedFormatItem<'_>] =
    format_description!("[year]-[month]-[day]T[hour]:[minute]:[second]");
const ISO_LOCAL: &[BorrowedFormatItem<'_>] = format_description!(
    "[year]-[month]-[day]T[hour]:[minute][optional [:[second][optional [.[subsecond]]]]]"
);

/// `20240102T030405Z`, `20240102T030405` or `20240102` to ISO 8601, and
/// whether it was a date. Anything else is passed through unchanged.
fn ical_to_iso(v: &str) -> (String, bool) {
    let v = v.trim();
    let (local, z) = match v.strip_suffix('Z') {
        Some(l) => (l, "Z"),
        None => (v, ""),
    };
    if let Ok(d) = Date::parse(v, ICAL_DATE) {
        (d.format(ISO_DATE).unwrap_or_default(), true)
    } else if let Ok(t) = PrimitiveDateTime::parse(local, ICAL_TIME) {
        (
            format!("{}{z}", t.format(ISO_TIME).unwrap_or_default()),
            false,
        )
    } else {
        (v.to_string(), false)
    }
}

/// ISO 8601 to iCalendar form: a date, a UTC time (any offset is converted)
/// or a floating local time. Anything else is passed through unchanged.
fn iso_to_ical(v: &str) -> String {
    let v = v.trim();
    if let Ok(t) = OffsetDateTime::parse(v, &Rfc3339) {
        let t = t.to_offset(UtcOffset::UTC);
        format!("{}Z", t.format(ICAL_TIME).unwrap_or_default())
    } else if let Ok(t) = PrimitiveDateTime::parse(v, ISO_LOCAL) {
        t.format(ICAL_TIME).unwrap_or_default()
    } else if let Ok(d) = Date::parse(v, ISO_DATE) {
        d.format(ICAL_DATE).unwrap_or_default()
    } else {
        v.to_string()
    }
}

/// The date part of an ISO 8601 date or time, in iCalendar form.
fn iso_date_to_ical(v: &str) -> String {
    let v = v.trim();
    match v.get(..10).map(|d| Date::parse(d, ISO_DATE)) {
        Some(Ok(d)) => d.format(ICAL_DATE).unwrap_or_default(),
        _ => iso_to_ical(v),
    }
}

/// An iCalendar object with one event.
pub fn to_ical(v: &Json, target: &str) -> String {
    let mut out = String::new();
    let mut add = |l: String| fold(&l, &mut out);
    add("BEGIN:VCALENDAR".into());
    add("VERSION:2.0".into());
    add("PRODID:-//homeserver//homeserver//EN".into());
    add("BEGIN:VEVENT".into());
    add(format!("UID:{}", escape(&uid_of(v, target))));
    add("DTSTAMP:19700101T000000Z".into());
    let all_day = v.get("all_day").and_then(Json::as_bool).unwrap_or(false);
    for (k, prop) in [("start", "DTSTART"), ("end", "DTEND")] {
        if let Some(t) = text(v, k) {
            if all_day {
                add(format!("{prop};VALUE=DATE:{}", iso_date_to_ical(t)));
            } else {
                add(format!("{prop}:{}", iso_to_ical(t)));
            }
        }
    }
    add(format!(
        "SUMMARY:{}",
        escape(text(v, "title").unwrap_or(""))
    ));
    for (k, prop) in [("location", "LOCATION"), ("description", "DESCRIPTION")] {
        if let Some(t) = text(v, k) {
            add(format!("{prop}:{}", escape(t)));
        }
    }
    if let Some(r) = text(v, "rrule") {
        add(format!("RRULE:{r}"));
    }
    for x in texts(v, "extra") {
        add(x);
    }
    add("END:VEVENT".into());
    add("END:VCALENDAR".into());
    out
}

/// An event from iCalendar text (its first VEVENT).
pub fn from_ical(text: &str) -> Json {
    let mut o = Map::new();
    let mut extra = Vec::new();
    let mut depth_event = false;
    let mut done = false;
    for l in parse(text) {
        match (l.name.as_str(), l.value.to_ascii_uppercase().as_str()) {
            ("BEGIN", "VEVENT") if !done => {
                depth_event = true;
                continue;
            }
            ("END", "VEVENT") if depth_event => {
                depth_event = false;
                done = true;
                continue;
            }
            _ => {}
        }
        if !depth_event {
            continue;
        }
        let val = unescape(&l.value);
        match l.name.as_str() {
            "DTSTART" | "DTEND" => {
                let (iso, date) = ical_to_iso(&l.value);
                let key = if l.name == "DTSTART" { "start" } else { "end" };
                if key == "start" {
                    o.insert("all_day".into(), date.into());
                    o.insert("time".into(), iso.clone().into());
                }
                o.insert(key.into(), iso.into());
            }
            "SUMMARY" => {
                o.insert("title".into(), val.into());
            }
            "LOCATION" => {
                o.insert("location".into(), val.into());
            }
            "DESCRIPTION" => {
                o.insert("description".into(), val.into());
            }
            "RRULE" => {
                o.insert("rrule".into(), l.value.clone().into());
            }
            "UID" => {
                o.insert("uid".into(), val.into());
            }
            "DTSTAMP" | "CREATED" | "LAST-MODIFIED" | "SEQUENCE" => {}
            _ => extra.push(Json::from(l.raw.clone())),
        }
    }
    if !extra.is_empty() {
        o.insert("extra".into(), extra.into());
    }
    Json::Object(o)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vcard_round_trip() {
        let card = "BEGIN:VCARD\r\nVERSION:4.0\r\nUID:abc\r\nFN:Ada Lovelace\r\nN:Lovelace;Ada;;;\r\nEMAIL;TYPE=work:ada@example.org\r\nTEL:+44 1\r\nNOTE:line one\\nline two\\, ok\r\nX-CUSTOM;X-P=\"a:b\":kept\r\nEND:VCARD\r\n";
        let v = from_vcard(card);
        assert_eq!(v["name"], "Ada Lovelace");
        assert_eq!(v["given"], "Ada");
        assert_eq!(v["emails"][0], "ada@example.org");
        assert_eq!(v["note"], "line one\nline two, ok");
        let back = to_vcard(&v, "contact/x");
        assert!(back.contains("UID:abc\r\n"));
        assert!(back.contains("X-CUSTOM;X-P=\"a:b\":kept"));
        assert_eq!(from_vcard(&back), v);
    }

    #[test]
    fn ical_round_trip() {
        let cal = "BEGIN:VCALENDAR\r\nBEGIN:VTIMEZONE\r\nTZID:X\r\nEND:VTIMEZONE\r\nBEGIN:VEVENT\r\nUID:e1\r\nDTSTART:20240102T030405Z\r\nDTEND:20240102T040405Z\r\nSUMMARY:Tea\r\nLOCATION:Kitchen\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let v = from_ical(cal);
        assert_eq!(v["start"], "2024-01-02T03:04:05Z");
        assert_eq!(v["time"], "2024-01-02T03:04:05Z");
        assert_eq!(v["title"], "Tea");
        let back = to_ical(&v, "event/e1");
        assert!(back.contains("DTSTART:20240102T030405Z\r\n"));
        assert_eq!(from_ical(&back), v);
        let day = from_ical(
            "BEGIN:VEVENT\r\nDTSTART;VALUE=DATE:20240102\r\nSUMMARY:Holiday\r\nEND:VEVENT\r\n",
        );
        assert_eq!(day["all_day"], true);
        assert!(to_ical(&day, "event/h").contains("DTSTART;VALUE=DATE:20240102"));
        let long = to_vcard(&json!({"name": "x".repeat(200)}), "contact/a");
        assert!(long.lines().all(|l| l.len() <= 75));
    }

    #[test]
    fn ical_times() {
        assert_eq!(ical_to_iso("20240102"), ("2024-01-02".into(), true));
        assert_eq!(
            ical_to_iso("20240102T030405Z"),
            ("2024-01-02T03:04:05Z".into(), false)
        );
        assert_eq!(
            ical_to_iso("20240102T030405"),
            ("2024-01-02T03:04:05".into(), false)
        );
        assert_eq!(ical_to_iso("nonsense"), ("nonsense".into(), false));
        assert_eq!(iso_to_ical("2024-01-02T05:04:05+02:00"), "20240102T030405Z");
        assert_eq!(iso_to_ical("2024-01-02T03:04"), "20240102T030400");
        assert_eq!(iso_to_ical("2024-01-02"), "20240102");
        assert_eq!(iso_date_to_ical("2024-01-02T03:04:05Z"), "20240102");
        assert_eq!(iso_date_to_ical("2024-01-0é"), "2024-01-0é");
    }
}
