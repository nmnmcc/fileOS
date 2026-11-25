// SPDX-License-Identifier: AGPL-3.0-only

//! Everything a node answers over HTTP: the REST API, federation between
//! nodes, `WebDAV` with `CardDAV` and `CalDAV`, a Model Context Protocol endpoint,
//! a minimal Bitwarden-compatible vault, passkeys, `OpenID` Connect, forward
//! auth and the web app.
//!
//! Every request runs with an [`Access`]: a bearer token (header, cookie or
//! HTTP Basic password), another node (an HTTP Message Signature), or
//! nobody. What each can do is decided by grants in the private space.

use std::sync::Arc;

use hs_net::client::Client;
use hs_net::server::{Handler, Peer, Request, Response};
use hs_node::{Access, Node, now_ms};
use http::{Method, StatusCode, header};
use serde_json::json;

mod access_log;
mod auth;
mod bitwarden;
mod condition;
mod dav;
pub mod export;
mod fed;
pub mod formats;
mod mcp;
mod preview;
mod push;
pub mod replica;
mod rest;
mod s3;
pub mod sync;
pub mod util;
mod web;

use util::{ApiError, Result, json_ok};

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The API of one node.
pub struct Api {
    /// The node.
    pub node: Arc<Node>,
    /// For requests to other nodes. Nodes are authenticated by signatures,
    /// so self-signed certificates are accepted.
    pub nodes: Client,
    /// For requests to the rest of the Web (push services, ACME).
    pub web: Client,
    /// Answers for ACME HTTP-01 challenges.
    pub acme: hs_net::acme::Challenges,
    auth: auth::State,
    preview: Option<preview::Preview>,
    access_log: access_log::AccessLog,
}

impl std::fmt::Debug for Api {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Api").finish_non_exhaustive()
    }
}

impl Api {
    /// Wrap a node.
    pub fn new(node: Arc<Node>) -> Arc<Api> {
        Self::build(node, None)
    }

    /// Wrap a node and configure an executable that supports `thumbnail-worker`.
    pub fn with_thumbnail_worker(node: Arc<Node>, executable: std::path::PathBuf) -> Arc<Api> {
        Self::build(
            node,
            Some(preview::Preview::new(executable.into_boxed_path())),
        )
    }

    fn build(node: Arc<Node>, preview: Option<preview::Preview>) -> Arc<Api> {
        let auth = auth::State::load(&node);
        Arc::new(Api {
            node,
            nodes: Client::accepting_self_signed(),
            web: Client::new(),
            acme: hs_net::acme::Challenges::new(),
            auth,
            preview,
            access_log: access_log::AccessLog::default(),
        })
    }

    /// Start the background work: pushing entries to members, pulling from
    /// primaries, sending Web Push notifications and syncing replicas.
    pub fn start(self: &Arc<Api>) {
        sync::start(Arc::clone(self));
        push::start(Arc::clone(self));
        replica::start(Arc::clone(self));
    }

    /// Flush queued access events before shutting down the server.
    pub async fn flush_access_log(&self) {
        self.access_log.flush().await;
    }

    /// Create a space owned by this node.
    ///
    /// # Errors
    ///
    /// Returns an error if the space metadata is invalid or its log or index cannot be written.
    pub async fn create_space(
        &self,
        name: &str,
        info: serde_json::Value,
    ) -> std::result::Result<hs_core::Hash, String> {
        rest::create_space(self, name, info)
            .await
            .map_err(|e| e.message)
    }

    /// A space as `GET /api/spaces/{id}` shows it.
    ///
    /// # Errors
    ///
    /// Returns an error if the space is not on this node.
    pub async fn space_json(
        &self,
        id: &hs_core::Hash,
    ) -> std::result::Result<serde_json::Value, String> {
        rest::space_json(self, id).await.map_err(|e| e.message)
    }

    /// Create a grant as `POST /api/grants` does, from `{role, label,
    /// spaces, types, expires}`. Returns `{token, id}`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request is invalid or the grant cannot be written.
    pub async fn create_grant(
        &self,
        body: &serde_json::Value,
    ) -> std::result::Result<serde_json::Value, String> {
        rest::create_grant(self, body).await.map_err(|e| e.message)
    }

    /// Who is asking.
    pub async fn access(&self, req: &Request) -> Access {
        if let Some(token) = bearer(req) {
            return self
                .node
                .authenticate(&token)
                .await
                .unwrap_or(Access::Public);
        }
        if req.headers().contains_key("signature-input") {
            let now = now_ms() / 1000;
            let (parts, ()) = clone_parts(req);
            if let Ok(key) = hs_net::sig::verify(&parts, req.body(), now, 300) {
                return self.node.node_access(key).await;
            }
        }
        Access::Public
    }

    async fn route(&self, req: &Request, peer: Peer) -> Result {
        let path = req.uri().path().to_string();
        let segs: Vec<String> = path
            .split('/')
            .filter(|s| !s.is_empty())
            .map(|s| util::percent_decode(s, false))
            .collect();
        let segs: Vec<&str> = segs.iter().map(String::as_str).collect();
        match segs.as_slice() {
            ["api", rest @ ..] => {
                let access = self.access(req).await;
                rest::handle(self, req, peer, &access, rest).await
            }
            ["fed", rest @ ..] => {
                let access = self.access(req).await;
                fed::handle(self, req, &access, rest).await
            }
            ["dav", rest @ ..] => {
                let access = self.access(req).await;
                dav::handle(self, req, &access, rest).await
            }
            [".well-known", "carddav" | "caldav"] => {
                let mut r = util::empty(StatusCode::MOVED_PERMANENTLY);
                util::set_header(&mut r, header::LOCATION, "/dav/");
                Ok(r)
            }
            ["mcp"] => {
                let access = self.access(req).await;
                mcp::handle(self, req, &access).await
            }
            ["s3", ..] => s3::handle(self, req),
            ["bitwarden", rest @ ..] => bitwarden::handle(self, req, rest).await,
            [".well-known", "homeserver"] => Ok(json_ok(&self.info())),
            [".well-known", "acme-challenge", token] => match self.acme.get(token) {
                Some(answer) => Ok(util::respond(StatusCode::OK, "text/plain", answer)),
                None => Err(ApiError::not_found()),
            },
            ["auth" | "oauth", ..] | [".well-known", "openid-configuration"] => {
                let access = self.access(req).await;
                auth::handle(self, req, peer, &access, &segs).await
            }
            _ if req.method() == Method::GET || req.method() == Method::HEAD => {
                web::handle(req, &segs)
            }
            _ => Err(ApiError::new(
                StatusCode::METHOD_NOT_ALLOWED,
                "method not allowed",
            )),
        }
    }

    /// What this node says about itself, publicly.
    pub fn info(&self) -> serde_json::Value {
        let c = self.node.config();
        let id = self.node.identity();
        json!({
            "name": c.name,
            "identity": c.identity.to_id(),
            "did": hs_identity::did::did(&id),
            "node": self.node.key.public().to_id(),
            "url": c.url,
            "version": env!("CARGO_PKG_VERSION"),
        })
    }
}

/// A bearer token from the Authorization header (Bearer, or the password of
/// Basic), or the session cookie.
fn bearer(req: &Request) -> Option<String> {
    if let Some(h) = util::header(req, header::AUTHORIZATION) {
        if let Some(t) = h.strip_prefix("Bearer ") {
            return Some(t.trim().to_string());
        }
        if let Some(b) = h.strip_prefix("Basic ") {
            let raw = util::unb64(b.trim())?;
            let text = String::from_utf8(raw).ok()?;
            return text.split_once(':').map(|(_, p)| p.to_string());
        }
    }
    util::cookie(req, auth::COOKIE).map(String::from)
}

/// The request's parts without its body (for signature checks).
fn clone_parts(req: &Request) -> (http::request::Parts, ()) {
    let mut copy = http::Request::new(());
    *copy.method_mut() = req.method().clone();
    *copy.uri_mut() = req.uri().clone();
    *copy.version_mut() = req.version();
    copy.headers_mut().clone_from(req.headers());
    copy.into_parts()
}

impl Handler for Api {
    async fn handle(&self, req: Request, peer: Peer) -> Response {
        let time = now_ms();
        let start = std::time::Instant::now();
        let method = known_method(req.method());
        let path = req.uri().path();
        let resp = match self.route(&req, peer).await {
            Ok(r) => r,
            Err(e) => {
                let mut r = e.response();
                if e.status == StatusCode::UNAUTHORIZED && path.starts_with("/dav") {
                    util::set_header(
                        &mut r,
                        header::WWW_AUTHENTICATE,
                        "Basic realm=\"homeserver\"",
                    );
                }
                r
            }
        };
        self.access_log.record(|| self.node.dir().join("access.log"), access_log::RequestEvent {
            name: "http.request.completed",
            template: "{{http.request.method}} {{http.route}} returned {{http.response.status_code}} in {{ms}} ms",
            time,
            method,
            route: access_route(path),
            status: resp.status().as_u16(),
            ms: start.elapsed().as_millis(),
        }).await;
        let mut resp = resp;
        let h = resp.headers_mut();
        h.entry(header::X_CONTENT_TYPE_OPTIONS)
            .or_insert(http::HeaderValue::from_static("nosniff"));
        h.entry(header::REFERRER_POLICY)
            .or_insert(http::HeaderValue::from_static("same-origin"));
        resp
    }
}

fn access_route(path: &str) -> &'static str {
    match path.trim_start_matches('/').split('/').next() {
        Some("api") => "/api",
        Some("auth") => "/auth",
        Some("bitwarden") => "/bitwarden",
        Some("dav") => "/dav",
        Some("fed") => "/fed",
        Some("mcp") => "/mcp",
        Some("oauth") => "/oauth",
        Some("s3") => "/s3",
        Some(".well-known") => "/.well-known",
        _ => "/",
    }
}

fn known_method(method: &Method) -> &'static str {
    match method.as_str() {
        "GET" => "GET",
        "HEAD" => "HEAD",
        "POST" => "POST",
        "PUT" => "PUT",
        "PATCH" => "PATCH",
        "DELETE" => "DELETE",
        "OPTIONS" => "OPTIONS",
        "PROPFIND" => "PROPFIND",
        "REPORT" => "REPORT",
        _ => "OTHER",
    }
}
