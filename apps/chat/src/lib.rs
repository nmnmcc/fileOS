// SPDX-License-Identifier: AGPL-3.0-only

//! The fileOS chat app.
//!
//! A room is a directory in the space at `/chat`: `/chat/<room>/`. Each
//! message is one small file in it, `<time>-<hash>`, with one line:
//! `<from>\t<text>\n`, with `\`, tab and new line escaped. The names sort by
//! time and never meet, so different files never conflict, and people on
//! different nodes can write at the same time. Share `/chat` with other
//! nodes to talk with them.
//!
//! At its own origin it gives the rooms, their messages, and a form to send
//! one. When a room changes, it keeps the number of messages in `kv`.

use std::fmt::Write as _;

use fileos_app::app::exports::fileos::app::events::Guest;
use fileos_app::app::exports::wasi::http::incoming_handler::Guest as Http;
use fileos_app::app::fileos::app::{fs, kv, log};
use fileos_app::app::wasi::http::types::{IncomingRequest, ResponseOutparam};
use fileos_app::http::{Request, Response};

/// Where the rooms are.
const ROOT: &str = "/chat";

/// The most bytes of one message.
const MAX_MESSAGE: usize = 4096;

struct Chat;

impl Guest for Chat {
    fn on_change(path: String) {
        let Some(room) = path
            .strip_prefix("/chat/")
            .and_then(|r| r.split('/').next())
            .filter(|r| valid_room(r))
        else {
            return;
        };
        let count = messages(room).len();
        kv::set(&format!("count:{room}"), count.to_string().as_bytes());
        log::info(&format!("room {room} has {count} messages"));
    }
}

impl Http for Chat {
    fn handle(request: IncomingRequest, out: ResponseOutparam) {
        fileos_app::http::serve(request, out, route);
    }
}

fileos_app::app::export_app!(Chat);

fn route(req: &Request) -> Response {
    let (path, query) = req.path.split_once('?').unwrap_or((&req.path, ""));
    match (req.method.as_str(), path) {
        ("GET", "/") => index(req),
        ("GET", "/style.css") => Response::new(200, "text/css; charset=utf-8", STYLE),
        ("GET", "/go") => {
            // The form on the first page: go to a room by its name.
            let room = form_value(query, "room").unwrap_or_default();
            if valid_room(&room) {
                redirect(&format!("/room/{room}"))
            } else {
                Response::text(400, "a room name has 1 to 64 letters, digits, - or _\n")
            }
        }
        (method, path) => match path.strip_prefix("/room/") {
            Some(room) if valid_room(room) => match method {
                "GET" => room_page(room, req),
                "POST" => send(room, req),
                _ => Response::text(405, "only GET and POST\n"),
            },
            _ => Response::text(404, "not found\n"),
        },
    }
}

/// The first page: the rooms, and a form to go to one.
fn index(req: &Request) -> Response {
    let mut rows = String::new();
    for room in fs::list_dir(ROOT).unwrap_or_default() {
        if !valid_room(&room) {
            continue;
        }
        let count = kv::get(&format!("count:{room}"))
            .map_or_else(|| "0".into(), |v| String::from_utf8_lossy(&v).into_owned());
        let _ = write!(
            rows,
            "<li><a href=\"/room/{room}\">{room}</a> <small>{count} messages</small></li>"
        );
    }
    page(
        "Chat",
        req,
        &format!(
            "<h1>Chat</h1><ul>{rows}</ul>\
             <form method=get action=/go><input name=room placeholder=room required>\
             <button>Open</button></form>"
        ),
    )
}

/// A room: its messages, and a form to send one.
fn room_page(room: &str, req: &Request) -> Response {
    let mut rows = String::new();
    for (from, text) in messages(room) {
        let _ = write!(
            rows,
            "<li><b>{}</b> {}</li>",
            escape_html(&from),
            escape_html(&text)
        );
    }
    page(
        room,
        req,
        &format!(
            "<p><a href=/>All rooms</a></p><h1>{room}</h1><ol>{rows}</ol>\
             <form method=post action=\"/room/{room}\">\
             <input name=from placeholder=\"your name\" required>\
             <input name=text placeholder=message required autofocus>\
             <button>Send</button></form>"
        ),
    )
}

/// Adds a message to a room, and goes back to the room.
fn send(room: &str, req: &Request) -> Response {
    let body = String::from_utf8_lossy(&req.body);
    // A person who is signed in is named by the kernel; otherwise by the form.
    let from = req
        .user()
        .map(str::to_string)
        .or_else(|| form_value(&body, "from"))
        .unwrap_or_default();
    let text = form_value(&body, "text").unwrap_or_default();
    if from.trim().is_empty() || text.trim().is_empty() || text.len() > MAX_MESSAGE {
        return Response::text(400, "a message needs a name and a text\n");
    }
    let record = format!("{}\t{}\n", escape_field(&from), escape_field(&text));
    let now = fileos_app::app::wasi::clocks::wall_clock::now();
    let name = format!(
        "{:012}.{:09}-{:016x}",
        now.seconds,
        now.nanoseconds,
        fnv(record.as_bytes())
    );
    match fs::write(&format!("{ROOT}/{room}/{name}"), record.as_bytes()) {
        Ok(()) => redirect(&format!("/room/{room}")),
        Err(e) => Response::text(500, format!("the message was not sent: {e:?}\n")),
    }
}

/// The messages of a room, oldest first.
fn messages(room: &str) -> Vec<(String, String)> {
    let mut names = fs::list_dir(&format!("{ROOT}/{room}")).unwrap_or_default();
    names.sort();
    names
        .iter()
        .filter_map(|name| fs::read(&format!("{ROOT}/{room}/{name}")).ok())
        .filter_map(|bytes| {
            let line = String::from_utf8_lossy(&bytes).into_owned();
            let (from, text) = line.trim_end_matches('\n').split_once('\t')?;
            Some((unescape_field(from), unescape_field(text)))
        })
        .collect()
}

/// A 64-bit FNV-1a hash, so that two messages sent at the same time have
/// different names.
fn fnv(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3)
    })
}

fn page(title: &str, req: &Request, body: &str) -> Response {
    let who = req.user().map_or_else(
        || "Not signed in".to_string(),
        |u| format!("Signed in: {}", escape_html(u)),
    );
    Response::html(
        200,
        format!(
            "<!doctype html><meta charset=utf-8>\
             <meta name=viewport content=\"width=device-width\">\
             <link rel=stylesheet href=/style.css><title>{}</title>\
             <header>{who}</header><main>{body}</main>",
            escape_html(title)
        ),
    )
}

fn redirect(to: &str) -> Response {
    let mut response = Response::text(303, "");
    response
        .headers
        .push(("location".into(), to.as_bytes().to_vec()));
    response
}

/// A room name: 1 to 64 letters, digits, `-` or `_`.
fn valid_room(room: &str) -> bool {
    !room.is_empty()
        && room.len() <= 64
        && room
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// The value of a field in a form (`application/x-www-form-urlencoded`).
fn form_value(form: &str, name: &str) -> Option<String> {
    form.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (percent_decode(key) == name).then(|| percent_decode(value))
    })
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        let hex = (byte == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (byte, hex) {
            (_, Some(decoded)) => {
                out.push(decoded);
                i += 3;
            }
            (b'+', None) => {
                out.push(b' ');
                i += 1;
            }
            (other, None) => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A field of a record, with `\`, tab and new line escaped.
fn escape_field(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('\t', "\\t")
        .replace('\r', "")
        .replace('\n', "\\n")
}

fn unescape_field(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// Text that is safe to put into HTML.
fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The style of the pages. It is a file of its own, because the kernel's
/// default content security policy lets in no style inside the page.
const STYLE: &str = "body{font:16px/1.5 system-ui,sans-serif;max-width:40rem;margin:0 auto;padding:1rem}\
header{color:#666;font-size:.9em}ol,ul{padding-left:1.2rem}input{font:inherit;margin:.2rem}\
@media(prefers-color-scheme:dark){body{background:#111;color:#eee}a{color:#8ab4f8}}";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fields_go_through_escaping() {
        let text = "a\tb\nc\\d";
        assert_eq!(unescape_field(&escape_field(text)), text);
        assert!(!escape_field(text).contains('\t'));
        assert!(!escape_field(text).contains('\n'));
    }

    #[test]
    fn forms_are_read() {
        let form = "from=Ann+B&text=hi%20there%21&x=%zz";
        assert_eq!(form_value(form, "from").as_deref(), Some("Ann B"));
        assert_eq!(form_value(form, "text").as_deref(), Some("hi there!"));
        assert_eq!(form_value(form, "x").as_deref(), Some("%zz"));
        assert_eq!(form_value(form, "y"), None);
    }

    #[test]
    fn room_names_are_checked() {
        assert!(valid_room("family"));
        assert!(valid_room("a-b_c1"));
        assert!(!valid_room(""));
        assert!(!valid_room("../x"));
        assert!(!valid_room("a b"));
    }
}
