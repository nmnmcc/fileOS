//! The app's flows, with the steps a user takes, against a stub server.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use nsh_core::i18n::Translations;
use nsh_engine::{EngineConfig, Root};
use nsh_testing::{DialogAnswer, Headless, StubNetwork, StubResponse, StubRoute};

const ORIGIN: &str = "https://hs.test";
const GONE: Duration = Duration::from_secs(5);

/// What the stub server holds: whether a session exists, and the notes.
#[derive(Default)]
struct Server {
    signed_in: bool,
    logout_unavailable: bool,
    timeline_offsets: Vec<usize>,
    notes: Vec<(String, String, String)>, // id, title, text
    deleted: Vec<String>,
}

fn json(status: u16, body: &serde_json::Value) -> StubResponse {
    let mut r = StubResponse::default();
    r.status = status;
    r.headers = vec![("content-type".into(), "application/json".into())];
    r.body = body.to_string().into_bytes();
    r
}

fn status(status: u16) -> StubResponse {
    let mut r = StubResponse::default();
    r.status = status;
    r
}

fn row(id: &str, title: &str, text: &str) -> serde_json::Value {
    serde_json::json!({
        "space": "s1", "target": format!("note/{id}"), "type": "note",
        "payload": "aa", "media_type": "application/json", "seq": 1,
        "updated": 1_767_258_000_000_u64, "time": 1_767_258_000_000_u64,
        "title": title, "body": { "title": title, "text": text },
    })
}

/// Answers the API from `server`.
fn handle(server: &Mutex<Server>, method: &str, url: &str, body: Option<&str>) -> StubResponse {
    let mut s = server.lock().unwrap();
    let path = url.strip_prefix(ORIGIN).unwrap_or(url);
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let param = |name: &str| {
        query
            .split('&')
            .filter_map(|part| part.split_once('='))
            .find_map(|(key, value)| (key == name).then_some(value))
    };
    let segs: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match (method, segs.as_slice()) {
        ("POST", ["api", "session"]) => {
            let token = body
                .and_then(|b| serde_json::from_str::<serde_json::Value>(b).ok())
                .and_then(|b| b["token"].as_str().map(str::to_owned));
            if token.as_deref() == Some("good") {
                s.signed_in = true;
                json(200, &serde_json::json!({ "owner": true }))
            } else {
                json(401, &serde_json::json!({ "error": "unknown token" }))
            }
        }
        _ if !s.signed_in => json(401, &serde_json::json!({ "error": "sign in" })),
        ("GET", ["api", "session"]) => json(200, &serde_json::json!({ "owner": true })),
        ("DELETE", ["api", "session"]) => {
            if s.logout_unavailable {
                return status(503);
            }
            s.signed_in = false;
            status(204)
        }
        ("GET", ["api", "timeline"] | ["api", "spaces", "s1", "resources"]) => {
            assert_eq!(param("page"), Some("1"));
            let offset = param("offset").unwrap().parse::<usize>().unwrap();
            let limit = param("limit").unwrap().parse::<usize>().unwrap();
            if path == "/api/timeline" {
                s.timeline_offsets.push(offset);
            }
            let items: Vec<_> = s
                .notes
                .iter()
                .skip(offset)
                .take(limit)
                .map(|(i, t, x)| row(i, t, x))
                .collect();
            let next_offset =
                (offset + items.len() < s.notes.len()).then_some(offset + items.len());
            json(
                200,
                &serde_json::json!({ "items": items, "next_offset": next_offset }),
            )
        }
        ("GET", ["api", "search"]) => {
            let rows: Vec<_> = s.notes.iter().map(|(i, t, x)| row(i, t, x)).collect();
            json(200, &serde_json::Value::Array(rows))
        }
        ("GET", ["api", "spaces"]) => json(
            200,
            &serde_json::json!([{ "id": "s1", "name": "Home", "seq": 3, "members": [{ "identity": "x", "role": "owner" }] }]),
        ),
        ("GET", ["api", "spaces", "s1"]) => json(
            200,
            &serde_json::json!({ "id": "s1", "name": "Home", "seq": 3, "members": [] }),
        ),
        ("GET", ["api", "node"]) => json(
            200,
            &serde_json::json!({ "name": "test node", "identity": "id1", "version": "0.1.0" }),
        ),
        ("GET", ["api", "spaces", "s1", "resources", "note", id]) => {
            match s.notes.iter().find(|n| n.0 == *id) {
                Some((_, title, text)) => {
                    let mut r = json(200, &serde_json::json!({ "title": title, "text": text }));
                    r.headers.push(("etag".into(), "\"v1\"".into()));
                    r
                }
                None => status(404),
            }
        }
        ("PUT", ["api", "spaces", "s1", "resources", "note", id]) => {
            let b: serde_json::Value = serde_json::from_str(body.unwrap_or("{}")).unwrap();
            let title = b["title"].as_str().unwrap_or_default().to_owned();
            let text = b["text"].as_str().unwrap_or_default().to_owned();
            s.notes.retain(|n| n.0 != *id);
            s.notes.push(((*id).to_owned(), title, text));
            let mut r = status(201);
            r.headers.push(("etag".into(), "\"v2\"".into()));
            r
        }
        ("DELETE", ["api", "spaces", "s1", "resources", "note", id]) => {
            s.notes.retain(|n| n.0 != *id);
            s.deleted.push((*id).to_owned());
            status(204)
        }
        _ => json(
            404,
            &serde_json::json!({ "error": format!("no route {method} {path}") }),
        ),
    }
}

fn start(server: Server) -> (Headless, Arc<Mutex<Server>>) {
    let server = Arc::new(Mutex::new(server));
    let routes = ["GET", "POST", "PUT", "DELETE"].map(|method| {
        let server = Arc::clone(&server);
        let handler: nsh_testing::StubHandler = Arc::new(move |req, reply| {
            let body = match &req.body {
                nsh_engine::HttpBody::Bytes(b) => Some(String::from_utf8_lossy(b).into_owned()),
                _ => None,
            };
            reply.send(handle(&server, &req.method, &req.url, body.as_deref()));
        });
        (format!("{method} {ORIGIN}/*"), StubRoute::Handler(handler))
    });
    let mut config = EngineConfig::new(
        Root::of::<hs_ui::App>()
            .translations(Translations::from_json(hs_ui::TRANSLATIONS).unwrap()),
    );
    config.network = Some(Arc::new(StubNetwork::new(routes)));
    let app = Headless::start_at_page(config, &format!("{ORIGIN}/")).unwrap();
    (app, server)
}

fn signed_in_with(notes: &[(&str, &str, &str)]) -> (Headless, Arc<Mutex<Server>>) {
    let notes = notes
        .iter()
        .map(|(a, b, c)| ((*a).to_owned(), (*b).to_owned(), (*c).to_owned()))
        .collect();
    start(Server {
        signed_in: true,
        notes,
        ..Server::default()
    })
}

#[test]
fn a_bad_token_is_refused_and_a_good_one_signs_in() {
    let (mut app, _) = start(Server::default());
    app.wait_for("role:button[Sign in]").unwrap();
    assert!(!app.screen().enabled("role:button[Sign in]").unwrap());

    app.type_text("label:Access token", "bad").unwrap();
    app.press("role:button[Sign in]").unwrap();
    app.wait_for("That token was not accepted.").unwrap();

    app.clear("label:Access token").unwrap();
    app.type_text("label:Access token", "good").unwrap();
    app.press("role:button[Sign in]").unwrap();
    app.wait_for("Nothing here yet.").unwrap();
    assert!(app.screen().shows("Timeline"));
}

#[test]
fn the_timeline_lists_notes_and_opens_one() {
    let (mut app, _) = signed_in_with(&[("n1", "Groceries", "milk, eggs")]);
    app.wait_for("Groceries").unwrap();
    assert!(app.screen().shows("milk, eggs"));
    app.press("Groceries").unwrap();
    app.wait_for("role:button[Save]").unwrap();
    assert_eq!(
        app.screen().input_text("label:Title").unwrap(),
        Some("Groceries")
    );
}

#[test]
fn a_new_note_is_saved_and_listed() {
    let (mut app, server) = signed_in_with(&[]);
    app.wait_for("Nothing here yet.").unwrap();
    app.press("role:tab[Spaces]").unwrap();
    app.wait_for("Home").unwrap();
    app.press("Home").unwrap();
    app.wait_for("role:button[New note]").unwrap();
    app.press("role:button[New note]").unwrap();
    app.wait_for("label:Title").unwrap();
    app.type_text("label:Title", "Ideas").unwrap();
    app.type_text("label:Text", "Plant a tree").unwrap();
    app.press("role:button[Save]").unwrap();
    // The note's own page replaces the new one.
    app.wait_for("role:button[Delete]").unwrap();
    {
        let s = server.lock().unwrap();
        assert_eq!(s.notes.len(), 1);
        assert_eq!(s.notes[0].1, "Ideas");
        assert_eq!(s.notes[0].2, "Plant a tree");
    }
    app.back().unwrap();
    app.wait_for("Ideas").unwrap();
}

#[test]
fn deleting_asks_first() {
    let (mut app, server) = signed_in_with(&[("n1", "Old", "")]);
    app.wait_for("Old").unwrap();
    app.press("Old").unwrap();
    app.wait_for("role:button[Delete]").unwrap();

    app.on_dialog(|_| DialogAnswer::Press("Cancel".to_owned()));
    app.press("role:button[Delete]").unwrap();
    app.wait_for("role:button[Delete]").unwrap();
    assert_eq!(server.lock().unwrap().deleted, Vec::<String>::new());

    app.on_dialog(|_| DialogAnswer::Press("Delete".to_owned()));
    app.press("role:button[Delete]").unwrap();
    app.wait_for("Nothing here yet.").unwrap();
    assert_eq!(server.lock().unwrap().deleted, ["n1"]);
}

#[test]
fn search_shows_what_matches() {
    let (mut app, _) = signed_in_with(&[("n1", "Groceries", "milk")]);
    app.wait_for("Groceries").unwrap();
    app.press("role:tab[Search]").unwrap();
    app.type_text("label:Words, names, places…", "milk")
        .unwrap();
    app.press("role:button[Search]").unwrap();
    app.wait_for("Groceries").unwrap();
}

#[test]
fn signing_out_returns_to_the_sign_in_page() {
    let (mut app, _) = signed_in_with(&[]);
    app.wait_for("Nothing here yet.").unwrap();
    app.press("role:tab[Settings]").unwrap();
    app.wait_for("test node").unwrap();
    app.press("role:button[Sign out]").unwrap();
    app.wait_for("role:button[Sign in]").unwrap();
    app.wait_for_gone("test node", GONE).unwrap();
}

#[test]
fn failed_sign_out_keeps_the_session_visible_and_can_be_retried() {
    let (mut app, server) = start(Server {
        signed_in: true,
        logout_unavailable: true,
        ..Server::default()
    });
    app.wait_for("Nothing here yet.").unwrap();
    app.press("role:tab[Settings]").unwrap();
    app.wait_for("test node").unwrap();
    app.press("role:button[Sign out]").unwrap();
    app.wait_for("Something went wrong: HTTP 503").unwrap();
    assert!(app.screen().shows("test node"));
    assert!(server.lock().unwrap().signed_in);

    server.lock().unwrap().logout_unavailable = false;
    app.press("role:button[Sign out]").unwrap();
    app.wait_for("role:button[Sign in]").unwrap();
    assert!(!server.lock().unwrap().signed_in);
}

#[test]
fn loading_more_appends_the_next_page_without_requesting_the_first_again() {
    let notes = (0..51)
        .map(|i| (format!("n{i}"), format!("Note {i:03}"), String::new()))
        .collect();
    let (mut app, server) = start(Server {
        signed_in: true,
        notes,
        ..Server::default()
    });
    app.wait_for("Note 000").unwrap();
    assert!(!app.screen().shows("Note 050"));
    app.press("role:button[Load more]").unwrap();
    app.wait_for("Note 050").unwrap();
    assert!(app.screen().shows("Note 000"));
    app.wait_for_gone("role:button[Load more]", GONE).unwrap();
    assert_eq!(server.lock().unwrap().timeline_offsets, [0, 50]);
}

#[test]
fn every_language_has_every_key() {
    let keys = |json: &str| {
        let map: serde_json::Map<String, serde_json::Value> = serde_json::from_str(json).unwrap();
        map.keys().cloned().collect::<Vec<_>>()
    };
    assert_eq!(
        keys(include_str!("../locales/en.json")),
        keys(include_str!("../locales/zh-CN.json"))
    );
}
