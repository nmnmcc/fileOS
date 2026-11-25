// SPDX-License-Identifier: AGPL-3.0-only

//! Talking to a running node over its HTTP API, as the web app and other
//! clients do, and remembering the nodes one has signed in to.
//!
//! Sign-ins are kept in `$XDG_CONFIG_HOME/homeserver/cli.json` (only the
//! owner can read it): for each server URL a token, and which one is the
//! default. `--server` (or `$HS_SERVER`) picks another, and `$HS_TOKEN`
//! supplies a token without saving it.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use http::{Method, StatusCode, header};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde_json::{Value as Json, json};

use crate::Res;

/// The address `hs serve` listens on unless told otherwise.
pub const LOCAL_URL: &str = "http://127.0.0.1:8080";

/// Bodies read whole: everything except exports, blobs and event streams.
const MAX_JSON: usize = 64 * 1024 * 1024;

/// What stays as it is in a path segment (RFC 3986 unreserved).
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// One path segment, percent-encoded.
pub fn seg(s: &str) -> String {
    utf8_percent_encode(s, SEGMENT).to_string()
}

/// A query string from pairs, skipping the `None`s.
pub fn query(pairs: &[(&str, Option<String>)]) -> String {
    let mut q = form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        if let Some(v) = v {
            q.append_pair(k, v);
        }
    }
    let q = q.finish();
    if q.is_empty() { q } else { format!("?{q}") }
}

// Saved sign-ins

#[derive(Default)]
struct Saved {
    default: Option<String>,
    servers: BTreeMap<String, Login>,
}

#[derive(Clone)]
struct Login {
    token: String,
    /// Accept any TLS certificate (a node's own self-signed one).
    insecure: bool,
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".config")
        });
    base.join("homeserver/cli.json")
}

impl Saved {
    fn load() -> Res<Saved> {
        let path = config_path();
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Saved::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let v: Json =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let servers = v["servers"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(url, s)| {
                Some((
                    url.clone(),
                    Login {
                        token: s["token"].as_str()?.to_string(),
                        insecure: s["insecure"].as_bool().unwrap_or(false),
                    },
                ))
            })
            .collect();
        Ok(Saved {
            default: v["default"].as_str().map(str::to_string),
            servers,
        })
    }

    fn save(&self) -> Res {
        let path = config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let servers: serde_json::Map<String, Json> = self
            .servers
            .iter()
            .map(|(url, l)| {
                (
                    url.clone(),
                    json!({ "token": l.token, "insecure": l.insecure }),
                )
            })
            .collect();
        let text = serde_json::to_string_pretty(&json!({
            "default": self.default,
            "servers": servers,
        }))
        .unwrap_or_default();
        // Written beside, then moved over, so a crash leaves the old file.
        let tmp = path.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut f = options
            .open(&tmp)
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.write_all(text.as_bytes())
            .and_then(|()| f.write_all(b"\n"))
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// `http://host:port` without a trailing slash; `host:port` means `http`
/// for loopback addresses and `https` for anything else.
pub fn normalize_url(url: &str) -> Res<String> {
    let url = url.trim().trim_end_matches('/');
    let url = if url.contains("://") {
        url.to_string()
    } else if url.starts_with("localhost") || url.starts_with("127.") || url.starts_with("[::1]") {
        format!("http://{url}")
    } else {
        format!("https://{url}")
    };
    let uri: http::Uri = url.parse().map_err(|_| format!("not a URL: {url}"))?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.host().is_none() {
        return Err(format!("not an http(s) URL: {url}"));
    }
    if uri.path() != "/" && !uri.path().is_empty() {
        return Err(format!("give the server's address without a path: {url}"));
    }
    Ok(url)
}

/// The server picked by `--server`, `$HS_SERVER` or the saved default, if
/// any.
pub fn configured(server: Option<&str>) -> Res<Option<Server>> {
    let saved = Saved::load()?;
    let env_token = std::env::var("HS_TOKEN").ok().filter(|t| !t.is_empty());
    let url = match server {
        Some(s) => normalize_url(s)?,
        None => match &saved.default {
            Some(d) => d.clone(),
            None => return Ok(None),
        },
    };
    let login = saved.servers.get(&url).cloned();
    let token = match (env_token, &login) {
        (Some(t), _) => t,
        (None, Some(l)) => l.token.clone(),
        (None, None) => {
            return Err(format!(
                "no token for {url}; sign in with `hs login {url}` or set HS_TOKEN"
            ));
        }
    };
    Ok(Some(Server::new(
        url,
        token,
        login.is_some_and(|l| l.insecure),
    )))
}

/// The server, or a message saying how to pick one.
pub fn required(server: Option<&str>) -> Res<Server> {
    configured(server)?.ok_or_else(|| {
        "this needs a running node: sign in to one with `hs login URL` (or pass --server)".into()
    })
}

/// Remember a sign-in and make it the default.
pub fn save_login(url: &str, token: &str, insecure: bool) -> Res {
    let mut saved = Saved::load()?;
    saved.servers.insert(
        url.to_string(),
        Login {
            token: token.to_string(),
            insecure,
        },
    );
    saved.default = Some(url.to_string());
    saved.save()
}

/// Whether any sign-in is saved.
pub fn has_logins() -> Res<bool> {
    Ok(!Saved::load()?.servers.is_empty())
}

/// Forget a sign-in (the default one without `url`). Returns the URL.
pub fn forget_login(url: Option<&str>) -> Res<Option<String>> {
    let mut saved = Saved::load()?;
    let url = match url {
        Some(u) => normalize_url(u)?,
        None => match saved.default.clone() {
            Some(d) => d,
            None => return Ok(None),
        },
    };
    if saved.servers.remove(&url).is_none() {
        return Ok(None);
    }
    if saved.default.as_deref() == Some(&url) {
        saved.default = saved.servers.keys().next().cloned();
    }
    saved.save()?;
    Ok(Some(url))
}

/// The saved sign-ins: URL and whether it is the default.
pub fn logins() -> Res<Vec<(String, bool)>> {
    let saved = Saved::load()?;
    Ok(saved
        .servers
        .keys()
        .map(|u| (u.clone(), saved.default.as_deref() == Some(u)))
        .collect())
}

/// Read a token: from a prompt without echo on a terminal, else one line
/// of stdin.
pub fn read_token() -> Res<String> {
    let stdin = std::io::stdin();
    let line = if stdin.is_terminal() {
        eprint!("Token: ");
        let _ = std::io::stderr().flush();
        let line = without_echo(|| {
            let mut s = String::new();
            stdin.read_line(&mut s).map(|_| s)
        });
        eprintln!();
        line
    } else {
        let mut s = String::new();
        stdin.read_line(&mut s).map(|_| s)
    }
    .map_err(|e| e.to_string())?;
    let token = line.trim().to_string();
    if token.is_empty() {
        return Err("no token given".into());
    }
    Ok(token)
}

#[cfg(unix)]
fn without_echo<T>(read: impl FnOnce() -> T) -> T {
    use rustix::termios::{LocalModes, OptionalActions, tcgetattr, tcsetattr};
    let fd = std::io::stdin();
    let Ok(before) = tcgetattr(&fd) else {
        return read();
    };
    let mut quiet = before.clone();
    quiet.local_modes.remove(LocalModes::ECHO);
    let _ = tcsetattr(&fd, OptionalActions::Now, &quiet);
    let out = read();
    let _ = tcsetattr(&fd, OptionalActions::Now, &before);
    out
}

#[cfg(not(unix))]
fn without_echo<T>(read: impl FnOnce() -> T) -> T {
    read()
}

// Requests

/// A node reached over HTTP, with a token.
pub struct Server {
    pub url: String,
    token: String,
    client: hs_net::client::Client,
}

/// A response, read whole.
pub struct Reply {
    pub status: StatusCode,
    pub headers: http::HeaderMap,
    pub body: Vec<u8>,
}

impl Reply {
    pub fn json(&self) -> Res<Json> {
        if self.body.is_empty() {
            return Ok(Json::Null);
        }
        serde_json::from_slice(&self.body).map_err(|e| format!("the server sent bad JSON: {e}"))
    }

    pub fn header(&self, name: header::HeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

/// A request body and its media type.
pub type Body<'a> = Option<(&'a str, Vec<u8>)>;

impl Server {
    pub fn new(url: String, token: String, insecure: bool) -> Server {
        let client = if insecure {
            hs_net::client::Client::accepting_self_signed()
        } else {
            hs_net::client::Client::new()
        };
        Server { url, token, client }
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        headers: &[(header::HeaderName, String)],
        body: Body<'_>,
    ) -> Res<http::Request<Vec<u8>>> {
        let mut b = http::Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.url))
            .header(header::AUTHORIZATION, format!("Bearer {}", self.token))
            .header(
                header::USER_AGENT,
                concat!("hs/", env!("CARGO_PKG_VERSION")),
            );
        for (k, v) in headers {
            b = b.header(k, v);
        }
        let bytes = match body {
            Some((media_type, bytes)) => {
                b = b.header(header::CONTENT_TYPE, media_type);
                bytes
            }
            None => Vec::new(),
        };
        b.body(bytes).map_err(|e| e.to_string())
    }

    /// Send a request; any answer other than 2xx (or 304) is an error.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        headers: &[(header::HeaderName, String)],
        body: Body<'_>,
    ) -> Res<Reply> {
        let reply = self.fetch(method, path, headers, body).await?;
        check(&self.url, &reply)?;
        Ok(reply)
    }

    /// Send a request and take whatever comes back.
    pub async fn fetch(
        &self,
        method: Method,
        path: &str,
        headers: &[(header::HeaderName, String)],
        body: Body<'_>,
    ) -> Res<Reply> {
        let req = self.request(method, path, headers, body)?;
        let r = self.open(req).await?;
        let (parts, mut body) = r.into_parts();
        Ok(Reply {
            status: parts.status,
            headers: parts.headers,
            body: body.read_to_end(MAX_JSON).await?,
        })
    }

    async fn open(
        &self,
        req: http::Request<Vec<u8>>,
    ) -> Res<http::Response<hs_net::client::Streaming>> {
        self.client.send_streaming(req).await.map_err(|e| {
            format!(
                "cannot reach {}: {e}\n(is `hs serve` running there? Commands for a stopped node take --dir.)",
                self.url
            )
        })
    }

    /// The error for a reply that is not a success.
    pub fn check(&self, reply: &Reply) -> Res {
        check(&self.url, reply)
    }

    pub async fn get(&self, path: &str) -> Res<Json> {
        self.send(Method::GET, path, &[], None).await?.json()
    }

    pub async fn post(&self, path: &str, body: &Json) -> Res<Json> {
        self.send(Method::POST, path, &[], Some(json_body(body)))
            .await?
            .json()
    }

    pub async fn delete(&self, path: &str) -> Res<Reply> {
        self.send(Method::DELETE, path, &[], None).await
    }

    /// Send a request and hand the body, as it arrives, to `out`. Returns
    /// the response headers.
    pub async fn download(&self, path: &str, out: &mut dyn Write) -> Res<http::response::Parts> {
        let req = self.request(Method::GET, path, &[], None)?;
        let r = self.open(req).await?;
        let (parts, mut body) = r.into_parts();
        if !parts.status.is_success() {
            let reply = Reply {
                status: parts.status,
                headers: parts.headers,
                body: body.read_to_end(MAX_JSON).await.unwrap_or_default(),
            };
            check(&self.url, &reply)?;
            unreachable!("check fails for a status other than 2xx");
        }
        while let Some(data) = body.chunk().await? {
            out.write_all(&data).map_err(|e| e.to_string())?;
        }
        out.flush().map_err(|e| e.to_string())?;
        Ok(parts)
    }

    /// Lines of a `text/event-stream`, as they arrive, until `each` says
    /// stop or the server closes the stream.
    pub async fn events(&self, path: &str, mut each: impl FnMut(&str) -> bool) -> Res {
        let headers = [(header::ACCEPT, "text/event-stream".to_string())];
        let req = self.request(Method::GET, path, &headers, None)?;
        let r = self.open(req).await?;
        let (parts, mut body) = r.into_parts();
        if !parts.status.is_success() {
            let reply = Reply {
                status: parts.status,
                headers: parts.headers,
                body: body.read_to_end(MAX_JSON).await.unwrap_or_default(),
            };
            return check(&self.url, &reply);
        }
        let mut pending = Vec::new();
        while let Some(data) = body.chunk().await? {
            pending.extend_from_slice(&data);
            while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                let line = String::from_utf8_lossy(&line);
                if !each(line.trim_end_matches(['\n', '\r'])) {
                    return Ok(());
                }
            }
        }
        Ok(())
    }
}

pub fn json_body(v: &Json) -> (&'static str, Vec<u8>) {
    (
        "application/json",
        serde_json::to_vec(v).unwrap_or_default(),
    )
}

/// The error for a reply other than 2xx or 304, from its `{"error": …}`.
fn check(url: &str, r: &Reply) -> Res {
    if r.status.is_success() || r.status == StatusCode::NOT_MODIFIED {
        return Ok(());
    }
    let message = serde_json::from_slice::<Json>(&r.body)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_string))
        .unwrap_or_else(|| {
            r.status
                .canonical_reason()
                .unwrap_or("error")
                .to_lowercase()
        });
    Err(match r.status {
        StatusCode::UNAUTHORIZED => format!(
            "{url} did not accept the token ({message}); sign in again with `hs login {url}`"
        ),
        StatusCode::PRECONDITION_FAILED => {
            format!("it changed on the server in the meantime ({message})")
        }
        s => format!("{message} ({})", s.as_u16()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_get_a_scheme_and_lose_the_slash() {
        assert_eq!(
            normalize_url("localhost:8080/").unwrap(),
            "http://localhost:8080"
        );
        assert_eq!(
            normalize_url("127.0.0.1:8080").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_url("hs.example.org").unwrap(),
            "https://hs.example.org"
        );
        assert_eq!(
            normalize_url("http://hs.example.org").unwrap(),
            "http://hs.example.org"
        );
        assert!(normalize_url("ftp://x").is_err());
        assert!(normalize_url("https://x/api").is_err());
    }

    #[test]
    fn segments_and_queries_are_encoded() {
        assert_eq!(seg("家 home/x"), "%E5%AE%B6%20home%2Fx");
        assert_eq!(
            query(&[("q", Some("a b".into())), ("limit", None)]),
            "?q=a+b"
        );
        assert_eq!(query(&[("limit", None)]), "");
    }
}
