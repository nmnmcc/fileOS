// SPDX-License-Identifier: AGPL-3.0-only

//! The fileOS gateway: the HTTP front of a node, on this machine only.
//!
//! It listens on `127.0.0.1` and `::1`, on [`DEFAULT_PORT`] unless told
//! otherwise. Each request goes by its host name (see "HTTP and the browser"
//! in the docs):
//!
//! - `fileos.localhost`, `localhost`, `127.0.0.1` and `::1`: the kernel API below.
//! - `<app-id>.localhost`: the app with that ID.
//! - Any other name: `421 Misdirected Request`, so that DNS rebinding does
//!   not work.
//!
//! The kernel API takes no requests that a browser says are cross-site
//! (`Sec-Fetch-Site`). Every answer has safe default headers, and the
//! answers of apps can set no cookies.
//!
//! Every request to the kernel API needs `Authorization: Bearer <token>`,
//! with the token from the data directory. Requests act with the node key.
//!
//! | Request                                      | What it does                         |
//! | -------------------------------------------- | ------------------------------------ |
//! | `GET /v0/status`                             | The status, as text                  |
//! | `GET /v0/fs/<path>`                          | Read a file                          |
//! | `GET /v0/fs/<path>?list`                     | The names under a path, one a line   |
//! | `PUT /v0/fs/<path>`                          | Put a file                           |
//! | `POST /v0/fs/<path>?append`                  | Add to the end of a file             |
//! | `DELETE /v0/fs/<path>`                       | Delete a file, or take a mount away  |
//! | `POST /v0/space?at=<path>`                   | Make a space and mount it            |
//! | `POST /v0/mount?id=<id>&at=<path>`           | Mount a space from other nodes       |
//! | `GET /v0/grants/<path>`                      | The grants in the space of a path    |
//! | `POST /v0/grants/<path>?to=<key>&permissions=` | Give permissions; prints the grant ID |
//! | `DELETE /v0/grants/<path>?id=<grant>`        | Take back a grant                    |
//! | `POST /v0/checkpoint/<path>`                 | Checkpoint and compact a space       |
//! | `POST /v0/install?dir=<dir>`                 | Install an app from a directory      |
//! | `GET /v0/apps/<app>/view`                    | The mounts in an app's view          |
//! | `POST /v0/apps/<app>/view?path=&permissions=&as=` | Give an app a path, in its view  |
//! | `DELETE /v0/apps/<app>/view?name=<name>`     | Take a mount away from an app's view |
//! | `GET /v0/requests`                           | Requests of apps for access          |
//! | `POST /v0/requests/<id>?path=<path>`         | Answer a request with a path         |
//! | `DELETE /v0/requests/<id>`                   | Say no to a request                  |
//! | `GET /v0/kv/<app>`                           | An app's cache on this node, as text |
//! | `POST /v0/dial`                              | Connect to a multiaddr (the body)    |
//!

use std::collections::BTreeMap;
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::PathBuf;

use bytes::Bytes;
use fileos_core::Error;
use fileos_node::{Node, parse_grant, parse_permissions, parse_principal};
use http_body_util::combinators::UnsyncBoxBody;
use http_body_util::{BodyExt as _, Full};
use hyper::body::Incoming;
use hyper::header::{self, HeaderMap, HeaderValue};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use tokio::net::TcpListener;

/// An error in the body of an answer.
type BodyError = Box<dyn std::error::Error + Send + Sync>;

/// The body of an answer: bytes that the gateway made, or the stream that an
/// app gives.
type Body = UnsyncBoxBody<Bytes, BodyError>;

type Reply = Response<Body>;

/// The port of the gateway when no other is given.
pub const DEFAULT_PORT: u16 = 7341;

/// Who a request is for, by its host name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// The kernel API.
    Kernel,
    /// The app with this ID.
    App(String),
}

/// Says who a request is for, from its `Host`. `None` for a host name that
/// is not one of ours.
pub fn target(host: &str) -> Option<Target> {
    let host = host.trim().to_ascii_lowercase();
    // Take the port away: "[::1]:7341", "fileos.localhost:7341".
    let name = if let Some(rest) = host.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        host.rsplit_once(':')
            .map_or(host.as_str(), |(name, _)| name)
    };
    let name = name.trim_end_matches('.');
    match name {
        "fileos.localhost" | "localhost" | "127.0.0.1" | "::1" => Some(Target::Kernel),
        _ => {
            let label = name.strip_suffix(".localhost")?;
            let good = !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
            good.then(|| Target::App(label.to_string()))
        }
    }
}

/// Puts in the default headers that every answer has.
fn safe_headers(headers: &mut HeaderMap, target: &Target) {
    let mut set = |name: &'static str, value: &'static str| {
        if !headers.contains_key(name) {
            headers.insert(name, HeaderValue::from_static(value));
        }
    };
    set("x-content-type-options", "nosniff");
    set("referrer-policy", "same-origin");
    set("cross-origin-opener-policy", "same-origin");
    set("cross-origin-resource-policy", "same-origin");
    match target {
        Target::Kernel => {
            set(
                "content-security-policy",
                "default-src 'none'; frame-ancestors 'none'",
            );
            set("cache-control", "no-store");
        }
        Target::App(_) => {
            set(
                "content-security-policy",
                "default-src 'self'; frame-ancestors 'self'",
            );
        }
    }
}

/// Makes the answer of an app safe: it may set no cookies, because the
/// kernel signs people in, with no cookies at all.
pub fn clean_app_headers(headers: &mut HeaderMap) {
    headers.remove(header::SET_COOKIE);
    safe_headers(headers, &Target::App(String::new()));
}

/// Says if a browser sent the request from another site.
fn cross_site(headers: &HeaderMap) -> bool {
    headers
        .get("sec-fetch-site")
        .is_some_and(|v| v.as_bytes() == b"cross-site")
}

fn reply(status: StatusCode, body: impl Into<Bytes>) -> Reply {
    let body = Full::new(body.into())
        .map_err(|never| match never {})
        .boxed_unsync();
    let mut response = Response::new(body);
    *response.status_mut() = status;
    response
}

fn decode(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

/// Starts the gateway on `port` (0 for any free port), on `127.0.0.1` and,
/// when this machine has it, `::1`. Gives the port it listens on.
///
/// # Errors
///
/// When it cannot listen on 127.0.0.1.
pub async fn start(node: Node, port: u16, token: String) -> Result<u16, Error> {
    let v4 = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = TcpListener::bind(v4)
        .await
        .map_err(|e| Error(format!("gateway: {v4}: {e}")))?;
    let port = listener
        .local_addr()
        .map_err(|e| Error(format!("gateway: {e}")))?
        .port();
    let mut listeners = vec![listener];
    let v6 = SocketAddr::from((Ipv6Addr::LOCALHOST, port));
    match TcpListener::bind(v6).await {
        Ok(listener) => listeners.push(listener),
        Err(e) => tracing::debug!("gateway: not on {v6}: {e}"),
    }
    for listener in listeners {
        let (node, token) = (node.clone(), token.clone());
        tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };
                let (node, token) = (node.clone(), token.clone());
                tokio::spawn(async move {
                    let service = hyper::service::service_fn(move |req| {
                        serve(node.clone(), token.clone(), req)
                    });
                    if let Err(e) = hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(stream), service)
                        .await
                    {
                        tracing::debug!("gateway connection: {e}");
                    }
                });
            }
        });
    }
    Ok(port)
}

async fn serve(
    node: Node,
    token: String,
    req: Request<Incoming>,
) -> Result<Reply, std::convert::Infallible> {
    let host = req
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
        .or_else(|| req.uri().authority().map(ToString::to_string))
        .unwrap_or_default();
    let Some(target) = target(&host) else {
        let mut answer = reply(
            StatusCode::MISDIRECTED_REQUEST,
            format!("this node does not answer for the host {host:?}\n"),
        );
        safe_headers(answer.headers_mut(), &Target::Kernel);
        return Ok(answer);
    };
    let mut answer = match &target {
        Target::Kernel => kernel(node, token, req).await,
        Target::App(id) => app(&node, id, req).await,
    };
    safe_headers(answer.headers_mut(), &target);
    Ok(answer)
}

/// The headers that only the kernel may give an app. The kernel signs
/// people in, so an app gets no cookies or credentials, and no person that
/// the kernel did not say.
const KERNEL_HEADERS: [&str; 4] = ["cookie", "authorization", "fileos-user", "fileos-event"];

/// Says if a path of an app is for the kernel only, such as the requests
/// for schedules. Requests from outside never reach these paths.
pub fn kernel_only(path: &str) -> bool {
    path.starts_with("/.well-known/fileos/")
}

/// A request for an app.
async fn app(node: &Node, id: &str, req: Request<Incoming>) -> Reply {
    let Some(app) = node.http_app(id) else {
        let mut answer = reply(
            StatusCode::NOT_FOUND,
            format!("no app {id:?} on this node\n"),
        );
        clean_app_headers(answer.headers_mut());
        return answer;
    };
    if kernel_only(req.uri().path()) {
        let mut answer = reply(StatusCode::FORBIDDEN, "this path is for the kernel only\n");
        clean_app_headers(answer.headers_mut());
        return answer;
    }
    let (mut parts, body) = req.into_parts();
    for name in KERNEL_HEADERS {
        parts.headers.remove(name);
    }
    let mut answer = match app.handle(Request::from_parts(parts, body)).await {
        Ok(answer) => answer.map(|body| body.map_err(|e| Box::new(e) as BodyError).boxed_unsync()),
        Err(e) => {
            tracing::warn!("{e}");
            reply(StatusCode::BAD_GATEWAY, format!("{e}\n"))
        }
    };
    clean_app_headers(answer.headers_mut());
    answer
}

/// A request for the kernel API.
async fn kernel(node: Node, token: String, req: Request<Incoming>) -> Reply {
    if cross_site(req.headers()) {
        return reply(StatusCode::FORBIDDEN, "no requests from other sites\n");
    }
    let authorized = req
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.strip_prefix("Bearer ") == Some(token.as_str()));
    if !authorized {
        return reply(StatusCode::UNAUTHORIZED, "a token is needed\n");
    }
    let method = req.method().clone();
    let path = decode(req.uri().path());
    let query: BTreeMap<String, String> = req
        .uri()
        .query()
        .unwrap_or_default()
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect();
    let body = match req.into_body().collect().await {
        Ok(body) => body.to_bytes().to_vec(),
        Err(e) => return reply(StatusCode::BAD_REQUEST, format!("{e}\n")),
    };
    // Node calls may run WebAssembly and touch the disk, so they run on a
    // thread for blocking work.
    let result =
        tokio::task::spawn_blocking(move || route(&node, &method, &path, &query, body)).await;
    match result {
        Ok(Ok(reply)) => reply,
        Ok(Err(e)) => reply(StatusCode::BAD_REQUEST, format!("{e}\n")),
        Err(e) => reply(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}\n")),
    }
}

fn route(
    node: &Node,
    method: &Method,
    path: &str,
    query: &BTreeMap<String, String>,
    body: Vec<u8>,
) -> Result<Reply, Error> {
    let arg = |name: &str| {
        query
            .get(name)
            .cloned()
            .ok_or_else(|| Error(format!("`{name}` is needed")))
    };
    let ok = |text: String| Ok(reply(StatusCode::OK, text));
    let text = |body: Vec<u8>| {
        String::from_utf8(body).map_err(|e| Error(format!("the body is not text: {e}")))
    };
    if let Some(fs_path) = path.strip_prefix("/v0/fs") {
        let fs_path = if fs_path.is_empty() { "/" } else { fs_path };
        return match *method {
            Method::GET if query.contains_key("list") => ok(node.list(fs_path)?.join("\n") + "\n"),
            Method::GET => Ok(reply(StatusCode::OK, node.read(fs_path)?)),
            Method::PUT => node.put(fs_path, &body).and_then(|()| ok(String::new())),
            Method::POST if query.contains_key("append") => {
                node.append(fs_path, &body).and_then(|()| ok(String::new()))
            }
            Method::DELETE => node.delete(fs_path).and_then(|()| ok(String::new())),
            _ => Ok(reply(StatusCode::METHOD_NOT_ALLOWED, "")),
        };
    }
    if let Some(at) = path.strip_prefix("/v0/grants") {
        let at = if at.is_empty() { "/" } else { at };
        return match *method {
            Method::GET => ok(node.grants(at).into_iter().map(|l| l + "\n").collect()),
            Method::POST => {
                let to = parse_principal(&arg("to")?)?;
                let id = node.grant_to(at, to, &parse_permissions(&arg("permissions")?)?)?;
                ok(format!("{}:{}\n", id.entry, id.index))
            }
            Method::DELETE => node
                .take(at, parse_grant(&arg("id")?)?)
                .and_then(|()| ok(String::new())),
            _ => Ok(reply(StatusCode::METHOD_NOT_ALLOWED, "")),
        };
    }
    if let Some(at) = path.strip_prefix("/v0/checkpoint") {
        let at = if at.is_empty() { "/" } else { at };
        return node.checkpoint(at).and_then(|()| ok(String::new()));
    }
    if let Some(app) = path
        .strip_prefix("/v0/apps/")
        .and_then(|p| p.strip_suffix("/view"))
    {
        return match *method {
            Method::GET => ok(node
                .app_view(app)
                .into_iter()
                .map(|v| format!("/{}\t{} {:?}\n", v.name, v.space, v.prefix))
                .collect()),
            Method::POST => {
                let at = node.app_grant(
                    app,
                    &arg("path")?,
                    &parse_permissions(&arg("permissions")?)?,
                    query.get("as").map(String::as_str),
                )?;
                ok(format!("{at}\n"))
            }
            Method::DELETE => node
                .app_take(app, &arg("name")?)
                .and_then(|()| ok(String::new())),
            _ => Ok(reply(StatusCode::METHOD_NOT_ALLOWED, "")),
        };
    }
    if path == "/v0/requests" {
        return ok(node
            .requests()
            .into_iter()
            .map(|r| format!("{}\t{}\t{}\t{}\n", r.id, r.app, r.permissions, r.purpose))
            .collect());
    }
    if let Some(id) = path.strip_prefix("/v0/requests/") {
        return match *method {
            Method::POST => ok(format!("{}\n", node.answer(id, &arg("path")?)?)),
            Method::DELETE => node.refuse(id).and_then(|()| ok(String::new())),
            _ => Ok(reply(StatusCode::METHOD_NOT_ALLOWED, "")),
        };
    }
    if let Some(app) = path.strip_prefix("/v0/kv/") {
        let lines: String = node
            .kv(app)
            .iter()
            .map(|(k, v)| format!("{k}\t{}\n", String::from_utf8_lossy(v)))
            .collect();
        return ok(lines);
    }
    match (method, path) {
        (&Method::GET, "/v0/status") => ok(node.status().to_string()),
        (&Method::GET, "/v0/id") => ok(node.identity().map_or_else(
            || "no identity: make one with `fileos identity new`, or join one with `fileos identity join`\n".into(),
            |id| id.to_string(),
        )),
        (&Method::POST, "/v0/id/new") => ok(format!("{}\n", node.identity_new()?)),
        (&Method::POST, "/v0/id/node") => ok(format!(
            "{}\n",
            node.identity_add_node(arg("key")?.parse()?)?
        )),
        (&Method::DELETE, "/v0/id/node") => {
            node.identity_remove_node(arg("key")?.parse()?)?;
            ok(String::new())
        }
        (&Method::POST, "/v0/id/join") => {
            node.identity_join(arg("space")?.parse()?)?;
            ok(String::new())
        }
        (&Method::POST, "/v0/space") => ok(format!("{}\n", node.create(&arg("at")?)?)),
        (&Method::POST, "/v0/mount") => {
            node.mount(arg("id")?.parse()?, &arg("at")?)?;
            ok(String::new())
        }
        (&Method::POST, "/v0/install") => {
            let app = node.install(&PathBuf::from(arg("dir")?))?;
            ok(format!("{}\n{}\n", app.key, app.id))
        }
        (&Method::POST, "/v0/dial") => {
            node.dial(text(body)?.trim())?;
            ok(String::new())
        }
        _ => Ok(reply(StatusCode::NOT_FOUND, "no such request\n")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_paths_are_for_the_kernel_only() {
        assert!(kernel_only("/.well-known/fileos/schedule/reindex"));
        assert!(!kernel_only("/.well-known/other"));
        assert!(!kernel_only("/photos"));
    }

    #[test]
    fn requests_go_by_host_name() {
        for host in [
            "fileos.localhost",
            "fileos.localhost:7341",
            "localhost:7341",
            "127.0.0.1:7341",
            "[::1]:7341",
            "FILEOS.LOCALHOST.",
        ] {
            assert_eq!(target(host), Some(Target::Kernel), "{host}");
        }
        assert_eq!(
            target("photos-k3x9q2m7ab.localhost:7341"),
            Some(Target::App("photos-k3x9q2m7ab".into()))
        );
        // Names that are not ours: DNS rebinding, other sites, bad labels.
        for host in [
            "evil.example",
            "evil.example:7341",
            "a.b.localhost",
            "-x.localhost",
            ".localhost",
            "x_y.localhost",
            "",
        ] {
            assert_eq!(target(host), None, "{host}");
        }
    }

    #[test]
    fn apps_set_no_cookies() {
        let mut headers = HeaderMap::new();
        headers.insert(header::SET_COOKIE, HeaderValue::from_static("session=1"));
        headers.append(
            header::SET_COOKIE,
            HeaderValue::from_static("other=2; Domain=localhost"),
        );
        headers.insert(
            "content-security-policy",
            HeaderValue::from_static("default-src 'none'"),
        );
        clean_app_headers(&mut headers);
        assert!(headers.get(header::SET_COOKIE).is_none());
        // An app may make the policy stricter: its own one stays.
        assert_eq!(headers["content-security-policy"], "default-src 'none'");
        assert_eq!(headers["x-content-type-options"], "nosniff");
    }

    #[test]
    fn requests_from_other_sites_are_seen() {
        let mut headers = HeaderMap::new();
        assert!(!cross_site(&headers));
        headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
        assert!(!cross_site(&headers));
        headers.insert("sec-fetch-site", HeaderValue::from_static("cross-site"));
        assert!(cross_site(&headers));
    }
}
