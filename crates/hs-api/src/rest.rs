// SPDX-License-Identifier: AGPL-3.0-only

//! The REST API under `/api/`, used by the web app and other clients.
//!
//! Resources are plain HTTP: `GET`, `PUT`, `PATCH` (JSON Merge Patch) and
//! `DELETE` on `/api/spaces/{space}/resources/{type}/{id}`, with `ETags`
//! (the payload hash), `If-Match`, `If-None-Match` and `Idempotency-Key`.
//!
//! Timeline and resource lists accept `page=1` to return `items` with a
//! `next_offset`. Requests without that flag retain the array response.

use hs_core::Hash;
use hs_core::event::Kind;
use hs_node::{Access, now_ms};
use hs_space::Role;
use http::{Method, StatusCode, header};
use serde_json::{Value as Json, json};

use crate::util::{
    ApiError, Result, body_json, empty, header as hdr, json_ok, json_status, param, query, respond,
    set_header,
};
use crate::{Api, auth, formats, sync};
use hs_net::server::{Body, Peer, Request, Response};

/// The spaces a caller may read.
pub fn readable(api: &Api, access: &Access) -> Vec<Hash> {
    api.node
        .space_ids()
        .into_iter()
        .filter(|s| access.can_read(s))
        .collect()
}

fn hexes(ids: &[Hash]) -> Vec<String> {
    ids.iter().map(Hash::to_hex).collect()
}

/// Whether a grant limited to some types covers this one.
pub fn type_ok(access: &Access, kind: &str) -> bool {
    allowed_types(access).is_none_or(|types| types.iter().any(|value| value == kind))
}

fn allowed_types(access: &Access) -> Option<&[String]> {
    match access {
        Access::Grant { types, .. } => types.as_deref(),
        _ => None,
    }
}

fn rows(access: &Access, rows: Vec<hs_store::Row>) -> Json {
    Json::Array(
        rows.into_iter()
            .filter(|r| type_ok(access, &r.kind))
            .filter_map(|r| serde_json::to_value(r).ok())
            .collect(),
    )
}

fn require_owner(access: &Access) -> Result<()> {
    match access {
        Access::Public => Err(ApiError::unauthorized()),
        a if a.is_owner() => Ok(()),
        _ => Err(ApiError::forbidden("only the owner can do this")),
    }
}

fn signed_in(access: &Access) -> Result<()> {
    match access {
        Access::Public => Err(ApiError::unauthorized()),
        _ => Ok(()),
    }
}

/// Find a space the caller can read.
pub async fn space(api: &Api, access: &Access, name: &str) -> Result<Hash> {
    signed_in(access)?;
    let id = api.node.find_space(name).await?;
    if !access.can_read(&id) {
        return Err(ApiError::not_found());
    }
    Ok(id)
}

/// A space as JSON.
pub async fn space_json(api: &Api, id: &Hash) -> Result<Json> {
    let me = api.node.key.public();
    Ok(api
        .node
        .with_space(id, |l| {
            let s = &l.space;
            json!({
                "id": id.to_hex(),
                "name": s.name(),
                "info": s.info,
                "seq": s.seq,
                "primary": s.primary.to_id(),
                "is_primary": s.primary == me,
                "home": l.home,
                "members": s.members.iter().map(|(k, m)| json!({
                    "node": k.to_id(),
                    "identity": m.identity.to_id(),
                    "role": m.role.as_str(),
                    "home": m.home,
                })).collect::<Vec<_>>(),
            })
        })
        .await?)
}

/// The `ETag` of a payload.
pub fn etag(hash: &Hash) -> String {
    format!("\"{}\"", &hash.to_hex()[..32])
}

fn matches_etag(list: &str, tag: &str) -> bool {
    list.split(',')
        .map(str::trim)
        .any(|t| t == "*" || t.trim_start_matches("W/") == tag)
}

/// A nonce from an `Idempotency-Key`, so retries are ordered once.
pub fn nonce(req: &Request, access: &Access) -> Option<Vec<u8>> {
    let key = hdr(req, "idempotency-key")?;
    let who = access.grant_id().unwrap_or_default();
    Some(blake3::hash(format!("{who}\n{key}").as_bytes()).as_bytes()[..16].to_vec())
}

/// The current payload hash of a resource (`None` if absent or deleted).
pub async fn current(api: &Api, id: &Hash, target: &str) -> Result<Option<Hash>> {
    Ok(api
        .node
        .with_space(id, |l| {
            l.space.resources.get(target).and_then(|r| r.payload)
        })
        .await?)
}

/// Add what a file's bytes say about it (size, dimensions, time, place).
pub fn enrich_file(api: &Api, body: &mut Json) {
    let Some(hash) = body
        .get("hash")
        .and_then(Json::as_str)
        .and_then(Hash::from_hex)
    else {
        return;
    };
    let Ok(Some(bytes)) = api.node.blob(&hash) else {
        return;
    };
    let Some(o) = body.as_object_mut() else {
        return;
    };
    o.entry("size").or_insert(Json::from(bytes.len()));
    if let Some(media) = hs_media::probe(&bytes) {
        let m = media.to_json();
        if let Some(mo) = m.as_object() {
            for k in ["media_type", "time", "place"] {
                if let Some(v) = mo.get(k) {
                    o.entry(k).or_insert(v.clone());
                }
            }
        }
        o.insert("media".into(), m);
    }
}

/// Store a resource. Returns the response for PUT.
pub async fn put(
    api: &Api,
    req: &Request,
    access: &Access,
    id: &Hash,
    target: &str,
    bytes: Vec<u8>,
    media_type: &str,
) -> Result {
    put_version(api, req, access, id, target, (bytes, media_type), None).await
}

async fn put_version(
    api: &Api,
    req: &Request,
    access: &Access,
    id: &Hash,
    target: &str,
    content: (Vec<u8>, &str),
    expected: Option<Hash>,
) -> Result {
    let before = current(api, id, target).await?;
    let (mut bytes, media_type) = content;
    let is_json = media_type == "application/json" || media_type.ends_with("+json");
    if is_json {
        let mut v: Json =
            serde_json::from_slice(&bytes).map_err(|e| ApiError::bad(format!("bad JSON: {e}")))?;
        if target.starts_with("file/") {
            enrich_file(api, &mut v);
        }
        bytes = serde_json::to_vec(&v).unwrap_or_default();
    }
    let mt = if is_json {
        "application/json"
    } else {
        media_type
    };
    let signed = api.node.propose(
        access,
        id,
        Kind::Put,
        target,
        Some(&bytes),
        mt,
        nonce(req, access),
    )?;
    let conditions = crate::condition::Conditions::from_request(req, expected, false);
    let seq = sync::write_checked(api, id, &signed, Some(&bytes), &conditions).await?;
    let tag = etag(&Hash::of(&bytes));
    let status = if before.is_some() {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    let mut r = json_status(status, &json!({ "seq": seq, "etag": tag }));
    set_header(&mut r, header::ETAG, &tag);
    Ok(r)
}

async fn get_resource(
    api: &Api,
    req: &Request,
    access: &Access,
    id: &Hash,
    target: &str,
) -> Result {
    let (kind, _) = target.split_once('/').unwrap_or((target, ""));
    if !type_ok(access, kind) {
        return Err(ApiError::not_found());
    }
    let Some((res, bytes)) = api.node.get(id, target).await? else {
        return Err(ApiError::not_found());
    };
    let (Some(hash), Some(bytes)) = (res.payload, bytes) else {
        return Err(ApiError::new(StatusCode::GONE, "deleted"));
    };
    let tag = etag(&hash);
    if hdr(req, header::IF_NONE_MATCH).is_some_and(|m| matches_etag(m, &tag)) {
        let mut r = empty(StatusCode::NOT_MODIFIED);
        set_header(&mut r, header::ETAG, &tag);
        return Ok(r);
    }
    let q = query(req);
    let accept = hdr(req, header::ACCEPT).unwrap_or("");
    let wanted = param(&q, "format").unwrap_or(if accept.contains("text/vcard") {
        "vcard"
    } else if accept.contains("text/calendar") {
        "ical"
    } else {
        ""
    });
    let (body, mt) = match (wanted, res.media_type.as_str()) {
        ("vcard", "application/json") => {
            let v: Json = serde_json::from_slice(&bytes).unwrap_or_default();
            (
                formats::to_vcard(&v, target).into_bytes(),
                "text/vcard; charset=utf-8".to_string(),
            )
        }
        ("ical", "application/json") => {
            let v: Json = serde_json::from_slice(&bytes).unwrap_or_default();
            (
                formats::to_ical(&v, target).into_bytes(),
                "text/calendar; charset=utf-8".to_string(),
            )
        }
        _ => (bytes, res.media_type.clone()),
    };
    let mut r = respond(
        StatusCode::OK,
        if mt.is_empty() {
            "application/octet-stream"
        } else {
            &mt
        },
        body,
    );
    set_header(&mut r, header::ETAG, &tag);
    set_header(
        &mut r,
        header::LAST_MODIFIED,
        &crate::util::http_date(res.time),
    );
    set_header(&mut r, header::CACHE_CONTROL, "private, no-cache");
    if req.method() == Method::HEAD {
        *r.body_mut() = Body::empty();
    }
    Ok(r)
}

async fn resource(
    api: &Api,
    req: &Request,
    access: &Access,
    s: &str,
    kind: &str,
    rid: &str,
) -> Result {
    let id = space(api, access, s).await?;
    let target = format!("{kind}/{rid}");
    if !hs_core::valid_target(&target) {
        return Err(ApiError::bad("bad type or id"));
    }
    match *req.method() {
        Method::GET | Method::HEAD => get_resource(api, req, access, &id, &target).await,
        Method::PUT => {
            let ct = hdr(req, header::CONTENT_TYPE).unwrap_or("application/json");
            let ct = ct
                .split(';')
                .next()
                .unwrap_or(ct)
                .trim()
                .to_ascii_lowercase();
            let (bytes, mt) = match ct.as_str() {
                "text/vcard" | "text/x-vcard" => {
                    let text = String::from_utf8_lossy(req.body());
                    (
                        serde_json::to_vec(&formats::from_vcard(&text)).unwrap_or_default(),
                        "application/json".to_string(),
                    )
                }
                "text/calendar" => {
                    let text = String::from_utf8_lossy(req.body());
                    (
                        serde_json::to_vec(&formats::from_ical(&text)).unwrap_or_default(),
                        "application/json".to_string(),
                    )
                }
                _ => (req.body().to_vec(), ct),
            };
            put(api, req, access, &id, &target, bytes, &mt).await
        }
        Method::PATCH => {
            let patch = body_json(req)?;
            let Some((res, Some(bytes))) = api.node.get(&id, &target).await? else {
                return Err(ApiError::not_found());
            };
            if res.media_type != "application/json" {
                return Err(ApiError::new(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    "only JSON resources can be patched",
                ));
            }
            let mut doc: Json = serde_json::from_slice(&bytes).unwrap_or_default();
            crate::util::merge_patch(&mut doc, &patch);
            let new = serde_json::to_vec(&doc).unwrap_or_default();
            put_version(
                api,
                req,
                access,
                &id,
                &target,
                (new, "application/json"),
                res.payload,
            )
            .await
        }
        Method::DELETE => delete_resource(api, req, access, &id, &target).await,
        _ => Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method not allowed",
        )),
    }
}

pub(crate) async fn delete_resource(
    api: &Api,
    req: &Request,
    access: &Access,
    id: &Hash,
    target: &str,
) -> Result {
    let signed = api.node.propose(
        access,
        id,
        Kind::Delete,
        target,
        None,
        "",
        nonce(req, access),
    )?;
    let conditions = crate::condition::Conditions::from_request(req, None, true);
    sync::write_checked(api, id, &signed, None, &conditions).await?;
    Ok(empty(StatusCode::NO_CONTENT))
}

struct PageQuery {
    limit: u32,
    offset: u32,
    envelope: bool,
}

impl PageQuery {
    fn parse(query: &[(String, String)], default: u32) -> Self {
        Self {
            limit: limit(query, default),
            offset: param(query, "offset")
                .and_then(|value| value.parse().ok())
                .unwrap_or(0),
            envelope: param(query, "page") == Some("1"),
        }
    }

    fn types<'a>(&self, access: &'a Access) -> Option<&'a [String]> {
        if self.envelope {
            allowed_types(access)
        } else {
            None
        }
    }

    fn query_limit(&self) -> u32 {
        self.limit + u32::from(self.envelope && self.limit > 0)
    }

    fn response(&self, access: &Access, mut found: Vec<hs_store::Row>) -> Response {
        if !self.envelope {
            return json_ok(&rows(access, found));
        }
        let more = found.len() > self.limit as usize;
        found.truncate(self.limit as usize);
        let next = more.then(|| self.offset.checked_add(self.limit)).flatten();
        json_ok(&json!({ "items": rows(access, found), "next_offset": next }))
    }
}

fn limit(q: &[(String, String)], default: u32) -> u32 {
    param(q, "limit")
        .and_then(|l| l.parse().ok())
        .unwrap_or(default)
        .min(1000)
}

async fn blob_get(api: &Api, req: &Request, access: &Access, h: &str) -> Result {
    signed_in(access)?;
    let hash = Hash::from_hex(h).ok_or_else(ApiError::not_found)?;
    let tag = etag(&hash);
    if hdr(req, header::IF_NONE_MATCH).is_some_and(|m| matches_etag(m, &tag)) {
        return Ok(empty(StatusCode::NOT_MODIFIED));
    }
    let bytes = api.node.blob(&hash)?.ok_or_else(ApiError::not_found)?;
    let q = query(req);
    let mut mt = param(&q, "type")
        .unwrap_or("application/octet-stream")
        .to_string();
    let mut bytes = bytes;
    if let Some(side) = param(&q, "thumb").and_then(|t| t.parse::<u32>().ok()) {
        let worker = api.preview.as_ref().ok_or_else(|| {
            ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "thumbnail worker is not configured",
            )
        })?;
        let side = side.clamp(16, 1024);
        let thumb = worker.run(&bytes, side).await.map_err(|error| {
            let status = if error.kind() == std::io::ErrorKind::WouldBlock {
                StatusCode::SERVICE_UNAVAILABLE
            } else {
                tracing::warn!(name: "media.thumbnail.failed", %error, "Thumbnail worker failed");
                StatusCode::BAD_GATEWAY
            };
            ApiError::new(status, "thumbnail worker unavailable")
        })?;
        let (t, t_mt) = thumb.ok_or_else(|| {
            ApiError::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "no thumbnail for this")
        })?;
        bytes = t;
        mt = t_mt.to_string();
    }
    if mt.contains("html") || mt.contains("svg") || mt.contains("xml") || mt.contains("javascript")
    {
        // Never let a stored file run script in this origin.
        mt = "application/octet-stream".into();
    }
    let total = bytes.len();
    let mut status = StatusCode::OK;
    let mut range_header = None;
    if let Some(range) = hdr(req, header::RANGE).and_then(|r| r.strip_prefix("bytes="))
        && !range.contains(',')
        && let Some((a, b)) = range.split_once('-')
    {
        let (start, end) = match (a.parse::<usize>().ok(), b.parse::<usize>().ok()) {
            (Some(s), Some(e)) => (s, e.min(total.saturating_sub(1))),
            (Some(s), None) => (s, total.saturating_sub(1)),
            (None, Some(n)) => (total.saturating_sub(n), total.saturating_sub(1)),
            (None, None) => (0, total.saturating_sub(1)),
        };
        if start >= total || start > end {
            let mut r = empty(StatusCode::RANGE_NOT_SATISFIABLE);
            set_header(&mut r, header::CONTENT_RANGE, &format!("bytes */{total}"));
            return Ok(r);
        }
        bytes = bytes[start..=end].to_vec();
        status = StatusCode::PARTIAL_CONTENT;
        range_header = Some(format!("bytes {start}-{end}/{total}"));
    }
    let mut r = respond(status, &mt, bytes);
    set_header(&mut r, header::ETAG, &tag);
    set_header(
        &mut r,
        header::CACHE_CONTROL,
        "private, max-age=31536000, immutable",
    );
    set_header(&mut r, header::ACCEPT_RANGES, "bytes");
    set_header(&mut r, "content-security-policy", "sandbox");
    if let Some(cr) = range_header {
        set_header(&mut r, header::CONTENT_RANGE, &cr);
    }
    if req.method() == Method::HEAD {
        *r.body_mut() = Body::empty();
    }
    Ok(r)
}

fn can_write_somewhere(access: &Access) -> bool {
    match access {
        Access::Public => false,
        Access::Grant { role, .. } => *role >= Role::Writer,
        Access::Node { roles, .. } => roles.iter().any(|(_, r)| *r >= Role::Writer),
    }
}

fn events(api: &Api, access: &Access) -> Result {
    signed_in(access)?;
    let (tx, rx) = tokio::sync::mpsc::channel::<bytes::Bytes>(32);
    let mut sub = api.node.subscribe();
    let access = access.clone();
    tokio::spawn(async move {
        if tx
            .send(bytes::Bytes::from_static(b"retry: 3000\n\n"))
            .await
            .is_err()
        {
            return;
        }
        loop {
            let ev = tokio::select! {
                ev = sub.recv() => ev,
                () = tokio::time::sleep(std::time::Duration::from_secs(25)) => {
                    if tx.send(bytes::Bytes::from_static(b": keep-alive\n\n")).await.is_err() { return }
                    continue;
                }
            };
            let ev = match ev {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            };
            let kind = ev.target.split('/').next().unwrap_or("");
            if !access.can_read(&ev.space) || !type_ok(&access, kind) {
                continue;
            }
            let data = json!({
                "space": ev.space.to_hex(),
                "seq": ev.seq,
                "kind": ev.kind.as_str(),
                "target": ev.target,
            });
            let line = format!("id: {}\ndata: {data}\n\n", ev.seq);
            if tx.send(bytes::Bytes::from(line)).await.is_err() {
                return;
            }
        }
    });
    let mut r = http::Response::new(Body::Stream(rx));
    set_header(&mut r, header::CONTENT_TYPE, "text/event-stream");
    set_header(&mut r, header::CACHE_CONTROL, "no-cache");
    Ok(r)
}

fn access_json(access: &Access) -> Json {
    match access {
        Access::Public => json!({ "role": null }),
        Access::Grant {
            id,
            role,
            spaces,
            types,
        } => json!({
            "grant": id,
            "role": role.as_str(),
            "owner": access.is_owner(),
            "spaces": spaces.as_ref().map(|s| hexes(s)),
            "types": types,
        }),
        Access::Node { key, roles } => json!({
            "node": key.to_id(),
            "roles": roles.iter().map(|(s, r)| json!({"space": s.to_hex(), "role": r.as_str()})).collect::<Vec<_>>(),
        }),
    }
}

async fn resolve_spaces(api: &Api, v: Option<&Json>) -> Result<Option<Vec<Hash>>> {
    let Some(Json::Array(a)) = v else {
        return Ok(None);
    };
    let mut out = Vec::new();
    for s in a.iter().filter_map(Json::as_str) {
        out.push(api.node.find_space(s).await?);
    }
    Ok(Some(out))
}

/// Create a token grant from a JSON request: `{role, label, spaces, types,
/// expires}`. Returns `{token, id}`.
pub async fn create_grant(api: &Api, body: &Json) -> Result<Json> {
    let role = body
        .get("role")
        .and_then(Json::as_str)
        .and_then(Role::parse)
        .ok_or_else(|| ApiError::bad("role must be reader, writer or owner"))?;
    let label = body.get("label").and_then(Json::as_str).unwrap_or("");
    let spaces = resolve_spaces(api, body.get("spaces")).await?;
    let mut extra = serde_json::Map::new();
    if let Some(t) = body.get("types").filter(|t| t.is_array()) {
        extra.insert("types".into(), t.clone());
    }
    if let Some(e) = body.get("expires").and_then(Json::as_u64) {
        extra.insert("expires".into(), e.into());
    }
    for k in ["session", "client"] {
        if let Some(v) = body.get(k) {
            extra.insert(k.into(), v.clone());
        }
    }
    let (token, id) = api
        .node
        .create_token(role, spaces, label, Some(Json::Object(extra)))
        .await?;
    Ok(json!({ "token": token, "id": id }))
}

async fn session(api: &Api, req: &Request, peer: Peer, access: &Access) -> Result {
    match *req.method() {
        Method::GET => {
            signed_in(access)?;
            Ok(json_ok(&access_json(access)))
        }
        Method::POST => {
            let body = body_json(req)?;
            let token = body
                .get("token")
                .and_then(Json::as_str)
                .ok_or_else(|| ApiError::bad("token missing"))?;
            let a = api
                .node
                .authenticate(token)
                .await
                .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unknown token"))?;
            let mut r = json_ok(&access_json(&a));
            auth::set_session_cookie(&mut r, token, peer.tls, None);
            Ok(r)
        }
        Method::DELETE => {
            if let Access::Grant { id, .. } = access {
                let is_session = api.node.grants().await.iter().any(|g| {
                    g.get("id").and_then(Json::as_str) == Some(id)
                        && g.get("session").and_then(Json::as_bool) == Some(true)
                });
                if is_session {
                    let _ = api.node.revoke(id).await;
                }
            }
            let mut r = empty(StatusCode::NO_CONTENT);
            auth::clear_session_cookie(&mut r, peer.tls);
            Ok(r)
        }
        _ => Err(ApiError::new(
            StatusCode::METHOD_NOT_ALLOWED,
            "method not allowed",
        )),
    }
}

async fn add_member(api: &Api, access: &Access, id: &Hash, body: &Json) -> Result {
    let mut m = json!({
        "node": body.get("node"),
        "identity": body.get("identity"),
        "role": body.get("role").and_then(Json::as_str).unwrap_or("writer"),
        "home": body.get("home").or(body.get("url")),
    });
    if let Some(url) = body.get("url").and_then(Json::as_str)
        && body.get("node").is_none()
    {
        let u = format!("{}/.well-known/homeserver", url.trim_end_matches('/'));
        let req = http::Request::get(&u)
            .body(Vec::new())
            .map_err(|e| ApiError::bad(e.to_string()))?;
        let resp = api
            .nodes
            .send(req)
            .await
            .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
        let info: Json = serde_json::from_slice(resp.body())
            .map_err(|_| ApiError::new(StatusCode::BAD_GATEWAY, "not a homeserver"))?;
        m["node"] = info.get("node").cloned().unwrap_or_default();
        m["identity"] = info.get("identity").cloned().unwrap_or_default();
    }
    let node = m
        .get("node")
        .and_then(Json::as_str)
        .ok_or_else(|| ApiError::bad("node or url needed"))?
        .to_string();
    if m.get("identity").and_then(Json::as_str).is_none() {
        return Err(ApiError::bad("identity needed"));
    }
    let bytes = serde_json::to_vec(&m).unwrap_or_default();
    let seq = sync::write(
        api,
        access,
        id,
        Kind::Member,
        &format!("member/{node}"),
        Some(&bytes),
        "application/json",
        None,
    )
    .await?;
    Ok(json_status(
        StatusCode::CREATED,
        &json!({ "seq": seq, "member": m }),
    ))
}

async fn remove_member(api: &Api, access: &Access, id: &Hash, node: &str) -> Result {
    let key = hs_core::PublicKey::from_id(node).ok_or_else(|| ApiError::bad("bad node id"))?;
    let m = api
        .node
        .with_space(id, |l| l.space.members.get(&key).cloned())
        .await?
        .ok_or_else(ApiError::not_found)?;
    let body = json!({ "node": node, "identity": m.identity.to_id(), "role": m.role.as_str(), "remove": true });
    let bytes = serde_json::to_vec(&body).unwrap_or_default();
    sync::write(
        api,
        access,
        id,
        Kind::Member,
        &format!("member/{node}"),
        Some(&bytes),
        "application/json",
        None,
    )
    .await?;
    Ok(empty(StatusCode::NO_CONTENT))
}

/// Create a space, recording this node's home so other members can reach it.
pub async fn create_space(api: &Api, name: &str, info: Json) -> Result<Hash> {
    let id = api.node.create_space(name, info).await?;
    let c = api.node.config();
    if let Some(url) = c.url {
        let me = api.node.key.public().to_id();
        let m = json!({ "node": me, "identity": c.identity.to_id(), "role": "owner", "home": url });
        let bytes = serde_json::to_vec(&m).unwrap_or_default();
        api.node
            .write(
                &api.node.owner(),
                &id,
                Kind::Member,
                &format!("member/{me}"),
                Some(&bytes),
                "application/json",
                None,
            )
            .await?;
    }
    Ok(id)
}

pub async fn handle(
    api: &Api,
    req: &Request,
    peer: Peer,
    access: &Access,
    segs: &[&str],
) -> Result {
    let m = req.method().clone();
    let q = query(req);
    match (&m, segs) {
        (_, ["session"]) => session(api, req, peer, access).await,
        (&Method::GET, ["node"]) => Ok(json_ok(&api.info())),
        (&Method::GET, ["spaces"]) => {
            signed_in(access)?;
            let mut out = Vec::new();
            for id in readable(api, access) {
                out.push(space_json(api, &id).await?);
            }
            Ok(json_ok(&Json::Array(out)))
        }
        (&Method::POST, ["spaces"]) => {
            require_owner(access)?;
            let body = body_json(req)?;
            let name = body
                .get("name")
                .and_then(Json::as_str)
                .filter(|n| !n.is_empty())
                .ok_or_else(|| ApiError::bad("name missing"))?;
            let info = body.get("info").cloned().unwrap_or(json!({}));
            let id = create_space(api, name, info).await?;
            Ok(json_status(
                StatusCode::CREATED,
                &space_json(api, &id).await?,
            ))
        }
        (&Method::POST, ["spaces", "join"]) => {
            require_owner(access)?;
            let body = body_json(req)?;
            let home = body
                .get("home")
                .and_then(Json::as_str)
                .ok_or_else(|| ApiError::bad("home missing"))?;
            let id = body
                .get("space")
                .and_then(Json::as_str)
                .and_then(Hash::from_hex)
                .ok_or_else(|| ApiError::bad("space missing"))?;
            sync::join(api, home, id).await?;
            Ok(json_status(
                StatusCode::CREATED,
                &space_json(api, &id).await?,
            ))
        }
        (&Method::GET, ["spaces", s]) => {
            let id = space(api, access, s).await?;
            Ok(json_ok(&space_json(api, &id).await?))
        }
        (&Method::POST, ["spaces", s, "members"]) => {
            let id = space(api, access, s).await?;
            add_member(api, access, &id, &body_json(req)?).await
        }
        (&Method::DELETE, ["spaces", s, "members", node]) => {
            let id = space(api, access, s).await?;
            remove_member(api, access, &id, node).await
        }
        (&Method::POST, ["spaces", s, "handover"]) => {
            let id = space(api, access, s).await?;
            let body = body_json(req)?;
            let node = body
                .get("node")
                .and_then(Json::as_str)
                .ok_or_else(|| ApiError::bad("node missing"))?;
            let p = serde_json::to_vec(&json!({ "primary": node })).unwrap_or_default();
            let seq = sync::write(
                api,
                access,
                &id,
                Kind::Handover,
                "space/primary",
                Some(&p),
                "application/json",
                None,
            )
            .await?;
            Ok(json_ok(&json!({ "seq": seq })))
        }
        (&Method::POST, ["spaces", s, "sync"]) => {
            let id = space(api, access, s).await?;
            let n = sync::pull(api, &id).await?;
            Ok(json_ok(&json!({ "pulled": n })))
        }
        (&Method::GET, ["spaces", s, "resources"]) => {
            let id = space(api, access, s).await?;
            let kind = param(&q, "type");
            let page = PageQuery::parse(&q, 100);
            let found = api
                .node
                .index
                .list_page(
                    &id.to_hex(),
                    kind,
                    page.query_limit(),
                    page.offset,
                    page.types(access),
                )
                .await?;
            Ok(page.response(access, found))
        }
        (_, ["spaces", s, "resources", kind, rid]) => {
            resource(api, req, access, s, kind, rid).await
        }
        (&Method::POST, ["spaces", s, "erase", kind, rid]) => {
            let id = space(api, access, s).await?;
            let target = format!("{kind}/{rid}");
            sync::write(api, access, &id, Kind::Erase, &target, None, "", None).await?;
            Ok(empty(StatusCode::NO_CONTENT))
        }
        (&Method::GET, ["search"]) => {
            signed_in(access)?;
            let text = param(&q, "q").unwrap_or("");
            let found = api
                .node
                .index
                .search(text, &hexes(&readable(api, access)), limit(&q, 50))
                .await?;
            Ok(json_ok(&rows(access, found)))
        }
        (&Method::GET, ["timeline"]) => {
            signed_in(access)?;
            let before = param(&q, "before").and_then(|b| b.parse().ok());
            let page = PageQuery::parse(&q, 50);
            let found = api
                .node
                .index
                .timeline_page(
                    &hexes(&readable(api, access)),
                    before,
                    page.query_limit(),
                    page.offset,
                    page.types(access),
                )
                .await?;
            Ok(page.response(access, found))
        }
        (&Method::GET, ["near"]) => {
            signed_in(access)?;
            let f = |k| param(&q, k).and_then(|v| v.parse::<f64>().ok());
            let (lat, lon) = f("lat")
                .zip(f("lon"))
                .ok_or_else(|| ApiError::bad("lat and lon needed"))?;
            let km = f("km").unwrap_or(1.0);
            let found = api
                .node
                .index
                .near(lat, lon, km, &hexes(&readable(api, access)), limit(&q, 50))
                .await?;
            Ok(json_ok(&rows(access, found)))
        }
        (&Method::GET, ["related", s, kind, rid]) => {
            let id = space(api, access, s).await?;
            let found = api
                .node
                .index
                .related(&id.to_hex(), &format!("{kind}/{rid}"), limit(&q, 100))
                .await?;
            let allowed = hexes(&readable(api, access));
            let out: Vec<Json> = found
                .into_iter()
                .filter(|(_, r)| allowed.contains(&r.space) && type_ok(access, &r.kind))
                .map(|(rel, r)| json!({ "rel": rel, "row": r }))
                .collect();
            Ok(json_ok(&Json::Array(out)))
        }
        (&Method::POST, ["blobs"]) => {
            if !can_write_somewhere(access) {
                return Err(if matches!(access, Access::Public) {
                    ApiError::unauthorized()
                } else {
                    ApiError::forbidden("needs writer")
                });
            }
            let hash = api.node.put_blob(req.body())?;
            let mut r = json_status(
                StatusCode::CREATED,
                &json!({ "hash": hash.to_hex(), "size": req.body().len() }),
            );
            set_header(
                &mut r,
                header::LOCATION,
                &format!("/api/blobs/{}", hash.to_hex()),
            );
            Ok(r)
        }
        (&Method::GET | &Method::HEAD, ["blobs", h]) => blob_get(api, req, access, h).await,
        (&Method::GET, ["events"]) => events(api, access),
        (&Method::GET, ["grants"]) => {
            require_owner(access)?;
            let list: Vec<Json> = api
                .node
                .grants()
                .await
                .into_iter()
                .map(|mut g| {
                    if let Some(o) = g.as_object_mut() {
                        o.remove("token");
                    }
                    g
                })
                .collect();
            Ok(json_ok(&Json::Array(list)))
        }
        (&Method::POST, ["grants"]) => {
            require_owner(access)?;
            Ok(json_status(
                StatusCode::CREATED,
                &create_grant(api, &body_json(req)?).await?,
            ))
        }
        (&Method::DELETE, ["grants", gid]) => {
            require_owner(access)?;
            api.node.revoke(gid).await?;
            Ok(empty(StatusCode::NO_CONTENT))
        }
        (&Method::GET, ["identity"]) => {
            let doc = hs_identity::did::document(&api.node.identity());
            Ok(respond(
                StatusCode::OK,
                "application/did+json",
                serde_json::to_vec(&doc).unwrap_or_default(),
            ))
        }
        (&Method::GET, ["health"]) => {
            signed_in(access)?;
            let index = api.node.index.healthy().await;
            let counts: serde_json::Map<String, Json> = api
                .node
                .index
                .counts()
                .await?
                .into_iter()
                .map(|(k, v)| (k, Json::from(v)))
                .collect();
            Ok(json_ok(&json!({
                "ok": index,
                "index": index,
                "counts": counts,
                "spaces": api.node.space_ids().len(),
                "time": now_ms(),
            })))
        }
        (&Method::GET, ["export"]) => {
            require_owner(access)?;
            crate::export::response(api)
        }
        (&Method::GET, ["replicas"]) => {
            require_owner(access)?;
            let list: Vec<Json> = crate::replica::configured(api)
                .iter()
                .map(|s| Json::String(crate::replica::redact(s)))
                .collect();
            Ok(json_ok(&Json::Array(list)))
        }
        (&Method::POST, ["replicas", "sync"]) => {
            require_owner(access)?;
            Ok(json_ok(&Json::Array(crate::replica::sync_all(api).await)))
        }
        (_, ["push", rest @ ..]) => crate::push::handle(api, req, access, rest).await,
        _ => Err(ApiError::not_found()),
    }
}
