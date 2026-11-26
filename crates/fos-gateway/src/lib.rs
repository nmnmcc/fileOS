// SPDX-License-Identifier: AGPL-3.0-only

//! The fileOS gateway: a small HTTP API for local clients, such as the `fos`
//! command.
//!
//! Every request needs `Authorization: Bearer <token>`, with the token from
//! the data directory. Requests act with the node key.
//!
//! | Request                                      | What it does                         |
//! | -------------------------------------------- | ------------------------------------ |
//! | `GET /v0/status`                             | The status, as text                  |
//! | `GET /v0/fs/<path>`                          | Read a file or stream                |
//! | `GET /v0/fs/<path>?list`                     | The names under a path, one a line   |
//! | `PUT /v0/fs/<path>`                          | Put a file                           |
//! | `POST /v0/fs/<path>?append`                  | Add a record to a stream             |
//! | `DELETE /v0/fs/<path>`                       | Take a file or mount away            |
//! | `POST /v0/folder?name=<n>&at=<path>`         | Make a folder and mount it           |
//! | `POST /v0/mount?id=<id>&at=<path>`           | Mount a folder from other nodes      |
//! | `POST /v0/policy/<path>`                     | Write a policy entry (the body)      |
//! | `POST /v0/grant/<path>?to=<key>&abilities=…` | Give abilities, as the engine says   |
//! | `POST /v0/install?dir=<dir>`                 | Install an app from a directory      |
//! | `GET /v0/kv/<app>`                           | An app's key-value store, as text    |
//! | `POST /v0/dial`                              | Connect to a multiaddr (the body)    |
//!
//! Later, requests for other paths will go to the `handle-http` of apps.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::path::PathBuf;

use bytes::Bytes;
use fos_core::Error;
use fos_node::{Node, parse_abilities};
use http_body_util::{BodyExt as _, Full};
use hyper::body::Incoming;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use percent_encoding::percent_decode_str;
use tokio::net::TcpListener;

type Reply = Response<Full<Bytes>>;

fn reply(status: StatusCode, body: impl Into<Bytes>) -> Reply {
    let mut response = Response::new(Full::new(body.into()));
    *response.status_mut() = status;
    response
}

fn decode(text: &str) -> String {
    percent_decode_str(text).decode_utf8_lossy().into_owned()
}

/// Starts the gateway on `addr`. Gives the address it listens on.
///
/// # Errors
///
/// When it cannot listen on the address.
pub async fn start(node: Node, addr: SocketAddr, token: String) -> Result<SocketAddr, Error> {
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| Error(format!("gateway: {addr}: {e}")))?;
    let local = listener
        .local_addr()
        .map_err(|e| Error(format!("gateway: {e}")))?;
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                continue;
            };
            let (node, token) = (node.clone(), token.clone());
            tokio::spawn(async move {
                let service =
                    hyper::service::service_fn(move |req| serve(node.clone(), token.clone(), req));
                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await
                {
                    tracing::debug!("gateway connection: {e}");
                }
            });
        }
    });
    Ok(local)
}

async fn serve(
    node: Node,
    token: String,
    req: Request<Incoming>,
) -> Result<Reply, std::convert::Infallible> {
    let authorized = req
        .headers()
        .get(hyper::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.strip_prefix("Bearer ") == Some(token.as_str()));
    if !authorized {
        return Ok(reply(StatusCode::UNAUTHORIZED, "a token is needed\n"));
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
        Err(e) => return Ok(reply(StatusCode::BAD_REQUEST, format!("{e}\n"))),
    };
    // Node calls may run WebAssembly and touch the disk, so they run on a
    // thread for blocking work.
    let result =
        tokio::task::spawn_blocking(move || route(&node, &method, &path, &query, body)).await;
    Ok(match result {
        Ok(Ok(reply)) => reply,
        Ok(Err(e)) => reply(StatusCode::BAD_REQUEST, format!("{e}\n")),
        Err(e) => reply(StatusCode::INTERNAL_SERVER_ERROR, format!("{e}\n")),
    })
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
    if let Some(at) = path.strip_prefix("/v0/policy") {
        return node.policy(at, &body).and_then(|()| ok(String::new()));
    }
    if let Some(at) = path.strip_prefix("/v0/grant") {
        let to = arg("to")?.parse()?;
        node.grant(at, to, &parse_abilities(&arg("abilities")?)?)?;
        return ok(String::new());
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
        (&Method::POST, "/v0/folder") => {
            ok(format!("{}\n", node.create(&arg("name")?, &arg("at")?)?))
        }
        (&Method::POST, "/v0/mount") => {
            node.mount(arg("id")?.parse()?, &arg("at")?)?;
            ok(String::new())
        }
        (&Method::POST, "/v0/install") => {
            ok(format!("{}\n", node.install(&PathBuf::from(arg("dir")?))?))
        }
        (&Method::POST, "/v0/dial") => {
            node.dial(text(body)?.trim())?;
            ok(String::new())
        }
        _ => Ok(reply(StatusCode::NOT_FOUND, "no such request\n")),
    }
}
