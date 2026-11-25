//! The homeserver's HTTP API: what the pages read and change.
//!
//! The app is served by `hs` itself, so every request goes to the page's own
//! origin and carries the `hs_session` cookie the browser keeps. Native
//! Shell's `fetch` takes absolute URLs: the origin comes from the page's
//! address.

use nsh_core::prelude::*;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;

use crate::store;

/// A failed call: the status (0 when no response came) and what to show.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Error {
    pub(crate) status: u16,
    pub(crate) message: String,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

pub(crate) type Result<T> = std::result::Result<T, Error>;

/// One entry of the index: a timeline, search or space listing row.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct Row {
    pub(crate) space: String,
    /// `type/id`.
    pub(crate) target: String,
    #[serde(rename = "type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) updated: u64,
    #[serde(default)]
    pub(crate) time: u64,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) body: Option<Value>,
}

impl Row {
    /// The row's key in lists: unique across spaces.
    pub(crate) fn key(&self) -> String {
        format!("{}/{}", self.space, self.target)
    }

    /// What a list shows as its title.
    pub(crate) fn label(&self) -> String {
        let body = |k: &str| {
            self.body
                .as_ref()
                .and_then(|b| b.get(k))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        Some(self.title.clone())
            .filter(|s| !s.is_empty())
            .or_else(|| body("title"))
            .or_else(|| body("name"))
            .unwrap_or_else(|| self.target.clone())
    }

    /// The beginning of a note's text, if it has one.
    pub(crate) fn snippet(&self) -> Option<String> {
        let text = self.body.as_ref()?.get("text")?.as_str()?;
        let text = text.trim();
        (!text.is_empty()).then(|| text.chars().take(160).collect())
    }
}

/// A space, as `/api/spaces` lists it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct Space {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) seq: u64,
    #[serde(default)]
    pub(crate) members: Vec<Member>,
}

impl Space {
    pub(crate) fn label(&self) -> String {
        self.name
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.id.clone())
    }
}

/// A member of a space.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct Member {
    #[serde(default)]
    pub(crate) identity: String,
    #[serde(default)]
    pub(crate) role: String,
}

/// The node, as `/api/node` describes it.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub(crate) struct Node {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) identity: String,
    #[serde(default)]
    pub(crate) url: Option<String>,
    #[serde(default)]
    pub(crate) version: String,
}

/// A resource's representation and the `ETag` a change must match.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Resource {
    pub(crate) etag: Option<String>,
    pub(crate) media_type: String,
    pub(crate) text: String,
}

impl Resource {
    pub(crate) fn json(&self) -> Option<Value> {
        let json = self.media_type == "application/json" || self.media_type.ends_with("+json");
        json.then(|| serde_json::from_str(&self.text).ok())
            .flatten()
    }
}

/// The origin requests go to: the page's, read once per surface.
async fn origin() -> String {
    #[derive(Clone, Default)]
    struct Origin(Option<String>);
    let cached = global::<Origin>();
    if let Some(origin) = cached.get_untracked().0 {
        return origin;
    }
    let page = app_url().await.unwrap_or_default();
    let origin = origin_of(&page).unwrap_or_else(|| "http://localhost:8080".to_owned());
    cached.set(Origin(Some(origin.clone())));
    origin
}

/// `scheme://host[:port]` of an absolute URL.
fn origin_of(url: &str) -> Option<String> {
    let (scheme, rest) = url.split_once("://")?;
    let host = rest.split(['/', '?', '#']).next()?;
    (!host.is_empty()).then(|| format!("{scheme}://{host}"))
}

/// Percent-encodes one path segment or query value.
pub(crate) fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A path to a resource, its `type/id` target split and encoded.
pub(crate) fn resource_path(space: &str, target: &str) -> String {
    let (kind, id) = target.split_once('/').unwrap_or((target, ""));
    format!(
        "/api/spaces/{}/resources/{}/{}",
        enc(space),
        enc(kind),
        enc(id)
    )
}

/// Sends `request` (its URL a path on the server). A 401 means the session
/// is gone: the app shows the sign-in page.
async fn send(method: &str, path: &str, body: Option<(&str, String)>) -> Result<HttpResponse> {
    send_with(method, path, body, &[]).await
}

async fn send_with(
    method: &str,
    path: &str,
    body: Option<(&str, String)>,
    headers: &[(&str, &str)],
) -> Result<HttpResponse> {
    let url = format!("{}{path}", origin().await);
    let mut request = HttpRequest::new(method, url)
        .header("Accept", "application/json")
        .cache(nsh_core::platform::HttpCache::NoStore);
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    if let Some((media_type, body)) = body {
        request = request.header("Content-Type", media_type).body(body);
    }
    let response = fetch(request)
        .await
        .map_err(|message| Error { status: 0, message })?;
    if response.ok() {
        return Ok(response);
    }
    let status = response.status();
    if status == 401 && !path.starts_with("/api/session") {
        store::signed_out();
    }
    let text = response.text();
    let message = serde_json::from_str::<Value>(&text)
        .ok()
        .and_then(|v| v.get("error").and_then(Value::as_str).map(str::to_owned))
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| format!("HTTP {status}"));
    Err(Error { status, message })
}

fn parse<T: DeserializeOwned>(response: &HttpResponse) -> Result<T> {
    serde_json::from_slice(&response.body).map_err(|e| Error {
        status: response.status(),
        message: e.to_string(),
    })
}

async fn get<T: DeserializeOwned>(path: &str) -> Result<T> {
    parse(&send("GET", path, None).await?)
}

async fn post_json<T: DeserializeOwned>(path: &str, body: &Value) -> Result<T> {
    let body = Some(("application/json", body.to_string()));
    let response = send("POST", path, body).await?;
    if response.body.is_empty() {
        return serde_json::from_value(Value::Null).map_err(|e| Error {
            status: response.status(),
            message: e.to_string(),
        });
    }
    parse(&response)
}

// ---------------------------------------------------------------- calls

/// The current session, or an error (401 when signed out).
pub(crate) async fn session() -> Result<Value> {
    get("/api/session").await
}

/// Signs in with a token; the server sets the session cookie.
pub(crate) async fn sign_in(token: &str) -> Result<Value> {
    post_json("/api/session", &serde_json::json!({ "token": token })).await
}

pub(crate) async fn sign_out() -> Result<()> {
    send("DELETE", "/api/session", None).await.map(drop)
}

/// One page of visible rows and the index offset of the next server page.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub(crate) struct Page {
    pub(crate) items: Vec<Row>,
    pub(crate) next_offset: Option<u32>,
}

pub(crate) async fn timeline(limit: usize, offset: u32) -> Result<Page> {
    get(&format!(
        "/api/timeline?page=1&limit={limit}&offset={offset}"
    ))
    .await
}

pub(crate) async fn search(query: &str) -> Result<Vec<Row>> {
    get(&format!("/api/search?q={}&limit=50", enc(query))).await
}

pub(crate) async fn spaces() -> Result<Vec<Space>> {
    get("/api/spaces").await
}

pub(crate) async fn space(id: &str) -> Result<Space> {
    get(&format!("/api/spaces/{}", enc(id))).await
}

/// Creates a space; returns its id.
pub(crate) async fn create_space(name: &str) -> Result<String> {
    let created: Value = post_json("/api/spaces", &serde_json::json!({ "name": name })).await?;
    created
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| Error {
            status: 0,
            message: "no id in the answer".to_owned(),
        })
}

pub(crate) async fn resources(
    space: &str,
    kind: Option<&str>,
    limit: usize,
    offset: u32,
) -> Result<Page> {
    let kind = kind
        .map(|k| format!("&type={}", enc(k)))
        .unwrap_or_default();
    get(&format!(
        "/api/spaces/{}/resources?page=1&limit={limit}&offset={offset}{kind}",
        enc(space)
    ))
    .await
}

pub(crate) async fn resource(space: &str, target: &str) -> Result<Resource> {
    let response = send_with(
        "GET",
        &resource_path(space, target),
        None,
        &[("Accept", "application/json, text/*;q=0.9, */*;q=0.5")],
    )
    .await?;
    let media_type = response
        .header("content-type")
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_owned();
    Ok(Resource {
        etag: response.header("etag").map(str::to_owned),
        media_type,
        text: response.text(),
    })
}

/// Writes a JSON resource: over `etag` if given, else only if new.
pub(crate) async fn put_json(
    space: &str,
    target: &str,
    body: &Value,
    etag: Option<&str>,
) -> Result<Option<String>> {
    let key = new_id();
    let condition = match etag {
        Some(tag) => ("If-Match", tag),
        None => ("If-None-Match", "*"),
    };
    let response = send_with(
        "PUT",
        &resource_path(space, target),
        Some(("application/json", body.to_string())),
        &[condition, ("Idempotency-Key", &key)],
    )
    .await?;
    Ok(response.header("etag").map(str::to_owned))
}

pub(crate) async fn delete(space: &str, target: &str) -> Result<()> {
    send("DELETE", &resource_path(space, target), None)
        .await
        .map(drop)
}

pub(crate) async fn node() -> Result<Node> {
    get("/api/node").await
}

/// A new resource id: the wall clock in milliseconds and a count of the
/// ids this page made. Ids need not be secret (the server checks access),
/// and a new resource is written only if it does not exist yet
/// (`If-None-Match: *`), so a collision fails instead of overwriting.
pub(crate) fn new_id() -> String {
    #[derive(Clone, Default)]
    struct Made(u64);
    let made = global::<Made>();
    let n = made.get_untracked().0 + 1;
    made.set(Made(n));
    let ms = nsh_core::time::unix_ms() as u64;
    format!("{ms:011x}{n:04x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins() {
        assert_eq!(
            origin_of("https://hs.test/app/?x#y").as_deref(),
            Some("https://hs.test")
        );
        assert_eq!(
            origin_of("http://127.0.0.1:8080").as_deref(),
            Some("http://127.0.0.1:8080")
        );
        assert_eq!(origin_of("nope"), None);
    }

    #[test]
    fn encodes() {
        assert_eq!(enc("a b/ü"), "a%20b%2F%C3%BC");
        assert_eq!(
            resource_path("s1", "note/x y"),
            "/api/spaces/s1/resources/note/x%20y"
        );
    }
}
