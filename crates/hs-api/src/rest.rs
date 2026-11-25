// SPDX-License-Identifier: AGPL-3.0-only

//! The REST API under `/api/`, used by the command line and other clients.
//!
//! Every endpoint is a function here with its request and response types
//! from [`crate::types`] and an `OpenAPI` description beside it, which
//! [`crate::openapi`] collects into one document.
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

use crate::types as t;
use crate::util::{
    ApiError, Result, body, empty, header as hdr, params, respond, set_header, typed,
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

fn rows(access: &Access, rows: Vec<hs_store::Row>) -> Vec<t::Row> {
    rows.into_iter()
        .filter(|r| type_ok(access, &r.kind))
        .map(t::Row::from)
        .collect()
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

/// A space as the API shows it.
pub async fn space_json(api: &Api, id: &Hash) -> Result<t::Space> {
    let me = api.node.key.public();
    Ok(api
        .node
        .with_space(id, |l| {
            let s = &l.space;
            t::Space {
                id: id.to_hex(),
                name: s.name().to_string(),
                info: s.info.clone(),
                seq: s.seq,
                primary: s.primary.to_id(),
                is_primary: s.primary == me,
                home: l.home.clone(),
                members: s
                    .members
                    .iter()
                    .map(|(k, m)| t::Member {
                        node: k.to_id(),
                        identity: m.identity.to_id(),
                        role: m.role.into(),
                        home: m.home.clone(),
                    })
                    .collect(),
            }
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
    let mut r = typed(
        status,
        &t::Written {
            seq,
            etag: tag.clone(),
        },
    );
    set_header(&mut r, header::ETAG, &tag);
    Ok(r)
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

/// Create a token grant. Returns the token, shown only this once.
pub async fn create_grant(api: &Api, new: &t::NewGrant) -> Result<t::NewToken> {
    let spaces = match &new.spaces {
        Some(names) => {
            let mut out = Vec::new();
            for s in names {
                out.push(api.node.find_space(s).await?);
            }
            Some(out)
        }
        None => None,
    };
    let mut extra = serde_json::Map::new();
    if let Some(types) = &new.types {
        extra.insert("types".into(), json!(types));
    }
    if let Some(e) = new.expires {
        extra.insert("expires".into(), e.into());
    }
    if let Some(s) = new.session {
        extra.insert("session".into(), s.into());
    }
    let (token, id) = api
        .node
        .create_token(
            new.role.into(),
            spaces,
            &new.label,
            Some(Json::Object(extra)),
        )
        .await?;
    Ok(t::NewToken { token, id })
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

fn not_allowed() -> ApiError {
    ApiError::new(StatusCode::METHOD_NOT_ALLOWED, "method not allowed")
}

/// Route a request under `/api/`.
pub async fn handle(
    api: &Api,
    req: &Request,
    peer: Peer,
    access: &Access,
    segs: &[&str],
) -> Result {
    let m = req.method().clone();
    match (&m, segs) {
        (&Method::GET, ["session"]) => get_session(access),
        (&Method::POST, ["session"]) => sign_in(api, req, peer).await,
        (&Method::DELETE, ["session"]) => sign_out(api, peer, access).await,
        (&Method::GET, ["node"]) => Ok(typed(StatusCode::OK, &api.info())),
        (&Method::GET, ["openapi.json"]) => Ok(respond(
            StatusCode::OK,
            "application/json",
            crate::openapi::json(),
        )),
        (&Method::GET, ["spaces"]) => list_spaces(api, access).await,
        (&Method::POST, ["spaces"]) => new_space(api, req, access).await,
        (&Method::POST, ["spaces", "join"]) => join_space(api, req, access).await,
        (&Method::GET, ["spaces", s]) => get_space(api, access, s).await,
        (&Method::POST, ["spaces", s, "members"]) => add_member(api, req, access, s).await,
        (&Method::DELETE, ["spaces", s, "members", node]) => {
            remove_member(api, access, s, node).await
        }
        (&Method::POST, ["spaces", s, "handover"]) => handover(api, req, access, s).await,
        (&Method::POST, ["spaces", s, "sync"]) => pull(api, access, s).await,
        (&Method::GET, ["spaces", s, "resources"]) => list_resources(api, req, access, s).await,
        (_, ["spaces", s, "resources", kind, rid]) => {
            let id = space(api, access, s).await?;
            let target = format!("{kind}/{rid}");
            if !hs_core::valid_target(&target) {
                return Err(ApiError::bad("bad type or id"));
            }
            match m {
                Method::GET | Method::HEAD => get_resource(api, req, access, &id, &target).await,
                Method::PUT => put_resource(api, req, access, &id, &target).await,
                Method::PATCH => patch_resource(api, req, access, &id, &target).await,
                Method::DELETE => delete_resource(api, req, access, &id, &target).await,
                _ => Err(not_allowed()),
            }
        }
        (&Method::POST, ["spaces", s, "erase", kind, rid]) => {
            erase(api, access, s, &format!("{kind}/{rid}")).await
        }
        (&Method::GET, ["search"]) => search(api, req, access).await,
        (&Method::GET, ["timeline"]) => timeline(api, req, access).await,
        (&Method::GET, ["near"]) => near(api, req, access).await,
        (&Method::GET, ["related", s, kind, rid]) => {
            related(api, req, access, s, &format!("{kind}/{rid}")).await
        }
        (&Method::POST, ["blobs"]) => put_blob(api, req, access),
        (&Method::GET | &Method::HEAD, ["blobs", h]) => get_blob(api, req, access, h).await,
        (&Method::GET, ["events"]) => events(api, access),
        (&Method::GET, ["grants"]) => list_grants(api, access).await,
        (&Method::POST, ["grants"]) => new_grant(api, req, access).await,
        (&Method::DELETE, ["grants", gid]) => revoke_grant(api, access, gid).await,
        (&Method::GET, ["identity"]) => Ok(identity(api)),
        (&Method::GET, ["health"]) => health(api, access).await,
        (&Method::GET, ["export"]) => {
            require_owner(access)?;
            crate::export::response(api)
        }
        (&Method::GET, ["replicas"]) => list_replicas(api, access),
        (&Method::POST, ["replicas", "sync"]) => sync_replicas(api, access).await,
        (_, ["push", rest @ ..]) => crate::push::handle(api, req, access, rest).await,
        (_, ["session" | "spaces" | "blobs" | "grants" | "replicas", ..]) => Err(not_allowed()),
        _ => Err(ApiError::not_found()),
    }
}

// Sessions

fn session_of(access: &Access) -> t::Session {
    match access {
        Access::Public => t::Session::default(),
        Access::Grant {
            id,
            role,
            spaces,
            types,
        } => t::Session {
            grant: Some(id.clone()),
            role: Some((*role).into()),
            owner: Some(access.is_owner()),
            spaces: spaces.as_ref().map(|s| hexes(s)),
            types: types.clone(),
            ..t::Session::default()
        },
        Access::Node { key, roles } => t::Session {
            node: Some(key.to_id()),
            roles: Some(
                roles
                    .iter()
                    .map(|(s, r)| t::SpaceRole {
                        space: s.to_hex(),
                        role: (*r).into(),
                    })
                    .collect(),
            ),
            ..t::Session::default()
        },
    }
}

/// Who the token belongs to.
#[utoipa::path(get, path = "/api/session", tag = "session",
    responses((status = 200, body = t::Session)))]
fn get_session(access: &Access) -> Result {
    signed_in(access)?;
    Ok(typed(StatusCode::OK, &session_of(access)))
}

/// Check a token and keep it in a session cookie, for browsers.
#[utoipa::path(post, path = "/api/session", tag = "session", security(()),
    request_body = t::SignIn,
    responses((status = 200, body = t::Session)))]
async fn sign_in(api: &Api, req: &Request, peer: Peer) -> Result {
    let t::SignIn { token } = body(req)?;
    let a = api
        .node
        .authenticate(&token)
        .await
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unknown token"))?;
    let mut r = typed(StatusCode::OK, &session_of(&a));
    auth::set_session_cookie(&mut r, &token, peer.tls, None);
    Ok(r)
}

/// Clear the session cookie, and revoke the grant if it was a session's.
#[utoipa::path(delete, path = "/api/session", tag = "session",
    responses((status = 204, description = "Signed out")))]
async fn sign_out(api: &Api, peer: Peer, access: &Access) -> Result {
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

/// What the node says about itself.
#[utoipa::path(get, path = "/api/node", tag = "node", security(()),
    responses((status = 200, body = t::NodeInfo)))]
#[expect(dead_code, reason = "only describes the route; `Api::info` answers it")]
fn node() {}

/// This document.
#[utoipa::path(get, path = "/api/openapi.json", tag = "node", security(()),
    responses((status = 200, description = "The OpenAPI document", content_type = "application/json")))]
#[expect(dead_code, reason = "only describes the route")]
fn openapi() {}

// Spaces

/// The spaces the caller can read.
#[utoipa::path(get, path = "/api/spaces", tag = "spaces",
    responses((status = 200, body = Vec<t::Space>)))]
async fn list_spaces(api: &Api, access: &Access) -> Result {
    signed_in(access)?;
    let mut out = Vec::new();
    for id in readable(api, access) {
        out.push(space_json(api, &id).await?);
    }
    Ok(typed(StatusCode::OK, &out))
}

/// Create a space kept by this node. Owner only.
#[utoipa::path(post, path = "/api/spaces", tag = "spaces",
    request_body = t::NewSpace,
    responses((status = 201, body = t::Space)))]
async fn new_space(api: &Api, req: &Request, access: &Access) -> Result {
    require_owner(access)?;
    let new: t::NewSpace = body(req)?;
    if new.name.is_empty() {
        return Err(ApiError::bad("name missing"));
    }
    let id = create_space(api, &new.name, new.info.unwrap_or_else(|| json!({}))).await?;
    Ok(typed(StatusCode::CREATED, &space_json(api, &id).await?))
}

/// Join a space kept by another node. Owner only.
#[utoipa::path(post, path = "/api/spaces/join", tag = "spaces",
    request_body = t::Join,
    responses((status = 201, body = t::Space)))]
async fn join_space(api: &Api, req: &Request, access: &Access) -> Result {
    require_owner(access)?;
    let join: t::Join = body(req)?;
    let id = Hash::from_hex(&join.space).ok_or_else(|| ApiError::bad("bad space ID"))?;
    sync::join(api, &join.home, id).await?;
    Ok(typed(StatusCode::CREATED, &space_json(api, &id).await?))
}

/// One space.
#[utoipa::path(get, path = "/api/spaces/{space}", tag = "spaces",
    params(("space" = String, Path, description = "A space's name, ID or unique ID prefix")),
    responses((status = 200, body = t::Space)))]
async fn get_space(api: &Api, access: &Access, s: &str) -> Result {
    let id = space(api, access, s).await?;
    Ok(typed(StatusCode::OK, &space_json(api, &id).await?))
}

/// Add a member, by node key and identity or by URL.
#[utoipa::path(post, path = "/api/spaces/{space}/members", tag = "spaces",
    params(("space" = String, Path, description = "A space's name, ID or unique ID prefix")),
    request_body = t::NewMember,
    responses((status = 201, body = t::MemberAdded)))]
async fn add_member(api: &Api, req: &Request, access: &Access, s: &str) -> Result {
    let id = space(api, access, s).await?;
    let new: t::NewMember = body(req)?;
    let (mut node, mut identity) = (new.node, new.identity);
    if let Some(url) = new.url.as_deref()
        && node.is_none()
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
        let info: t::NodeInfo = serde_json::from_slice(resp.body())
            .map_err(|_| ApiError::new(StatusCode::BAD_GATEWAY, "not a homeserver"))?;
        node = Some(info.node);
        identity = Some(info.identity);
    }
    let member = t::Member {
        node: node.ok_or_else(|| ApiError::bad("node or url needed"))?,
        identity: identity.ok_or_else(|| ApiError::bad("identity needed"))?,
        role: new.role.unwrap_or(t::Role::Writer),
        home: new.home.or(new.url),
    };
    let bytes = serde_json::to_vec(&member).unwrap_or_default();
    let seq = sync::write(
        api,
        access,
        &id,
        Kind::Member,
        &format!("member/{}", member.node),
        Some(&bytes),
        "application/json",
        None,
    )
    .await?;
    Ok(typed(StatusCode::CREATED, &t::MemberAdded { seq, member }))
}

/// Remove a member.
#[utoipa::path(delete, path = "/api/spaces/{space}/members/{node}", tag = "spaces",
    params(
        ("space" = String, Path, description = "A space's name, ID or unique ID prefix"),
        ("node" = String, Path, description = "The member node's key ID"),
    ),
    responses((status = 204, description = "Removed")))]
async fn remove_member(api: &Api, access: &Access, s: &str, node: &str) -> Result {
    let id = space(api, access, s).await?;
    let key = hs_core::PublicKey::from_id(node).ok_or_else(|| ApiError::bad("bad node id"))?;
    let m = api
        .node
        .with_space(&id, |l| l.space.members.get(&key).cloned())
        .await?
        .ok_or_else(ApiError::not_found)?;
    let body = json!({ "node": node, "identity": m.identity.to_id(), "role": m.role.as_str(), "remove": true });
    let bytes = serde_json::to_vec(&body).unwrap_or_default();
    sync::write(
        api,
        access,
        &id,
        Kind::Member,
        &format!("member/{node}"),
        Some(&bytes),
        "application/json",
        None,
    )
    .await?;
    Ok(empty(StatusCode::NO_CONTENT))
}

/// Hand the primary role to another member.
#[utoipa::path(post, path = "/api/spaces/{space}/handover", tag = "spaces",
    params(("space" = String, Path, description = "A space's name, ID or unique ID prefix")),
    request_body = t::Handover,
    responses((status = 200, body = t::Seq)))]
async fn handover(api: &Api, req: &Request, access: &Access, s: &str) -> Result {
    let id = space(api, access, s).await?;
    let to: t::Handover = body(req)?;
    let p = serde_json::to_vec(&json!({ "primary": to.node })).unwrap_or_default();
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
    Ok(typed(StatusCode::OK, &t::Seq { seq }))
}

/// Fetch new changes from the space's primary now.
#[utoipa::path(post, path = "/api/spaces/{space}/sync", tag = "spaces",
    params(("space" = String, Path, description = "A space's name, ID or unique ID prefix")),
    responses((status = 200, body = t::Pulled)))]
async fn pull(api: &Api, access: &Access, s: &str) -> Result {
    let id = space(api, access, s).await?;
    let pulled = sync::pull(api, &id).await?;
    Ok(typed(
        StatusCode::OK,
        &t::Pulled {
            pulled: u64::try_from(pulled).unwrap_or(u64::MAX),
        },
    ))
}

// Resources

/// Rows, as a bare array or (with `page=1`) a [`t::Page`].
struct Paged {
    limit: u32,
    offset: u32,
    envelope: bool,
}

impl Paged {
    fn new(limit: Option<u32>, offset: Option<u32>, page: Option<u8>, default: u32) -> Paged {
        Paged {
            limit: limit.unwrap_or(default).min(1000),
            offset: offset.unwrap_or(0),
            envelope: page == Some(1),
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
            return typed(StatusCode::OK, &rows(access, found));
        }
        let more = found.len() > self.limit as usize;
        found.truncate(self.limit as usize);
        let next_offset = more.then(|| self.offset.checked_add(self.limit)).flatten();
        typed(
            StatusCode::OK,
            &t::Page {
                items: rows(access, found),
                next_offset,
            },
        )
    }
}

fn limit(given: Option<u32>, default: u32) -> u32 {
    given.unwrap_or(default).min(1000)
}

/// The resources in a space, newest change first.
#[utoipa::path(get, path = "/api/spaces/{space}/resources", tag = "resources",
    params(("space" = String, Path, description = "A space's name, ID or unique ID prefix"), t::ListQuery),
    responses((status = 200, body = t::Listing)))]
async fn list_resources(api: &Api, req: &Request, access: &Access, s: &str) -> Result {
    let id = space(api, access, s).await?;
    let q: t::ListQuery = params(req)?;
    let page = Paged::new(q.limit, q.offset, q.page, 100);
    let found = api
        .node
        .index
        .list_page(
            &id.to_hex(),
            q.kind.as_deref(),
            page.query_limit(),
            page.offset,
            page.types(access),
        )
        .await?;
    Ok(page.response(access, found))
}

/// A resource's payload, with its `ETag`. Contacts and events stored as
/// JSON can be had as vCard or iCalendar.
#[utoipa::path(get, path = "/api/spaces/{space}/resources/{type}/{id}", tag = "resources",
    params(
        ("space" = String, Path, description = "A space's name, ID or unique ID prefix"),
        ("type" = String, Path, description = "The resource type"),
        ("id" = String, Path, description = "The resource ID"),
        ("If-None-Match" = Option<String>, Header, description = "Answer 304 if the ETag matches"),
        t::ResourceQuery,
    ),
    responses(
        (status = 200, description = "The payload, as stored", headers(("ETag" = String), ("Last-Modified" = String))),
        (status = 304, description = "Not modified"),
        (status = 410, description = "Deleted", body = t::Error),
    ))]
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
    let q: t::ResourceQuery = params(req)?;
    let accept = hdr(req, header::ACCEPT).unwrap_or("");
    let wanted = q
        .format
        .as_deref()
        .unwrap_or(if accept.contains("text/vcard") {
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

/// Store a resource: JSON, vCard or iCalendar (kept as JSON), or any other
/// bytes as they are.
#[utoipa::path(put, path = "/api/spaces/{space}/resources/{type}/{id}", tag = "resources",
    params(
        ("space" = String, Path, description = "A space's name, ID or unique ID prefix"),
        ("type" = String, Path, description = "The resource type"),
        ("id" = String, Path, description = "The resource ID"),
        ("If-Match" = Option<String>, Header, description = "Only if the current ETag matches"),
        ("If-None-Match" = Option<String>, Header, description = "`*`: only if it does not exist yet"),
        ("Idempotency-Key" = Option<String>, Header, description = "Retries with the same key are applied once"),
    ),
    request_body(content(
        (Object = "application/json"),
        (String = "text/vcard"),
        (String = "text/calendar"),
        ("application/octet-stream"),
    )),
    responses(
        (status = 200, description = "Replaced", body = t::Written, headers(("ETag" = String))),
        (status = 201, description = "Created", body = t::Written, headers(("ETag" = String))),
        (status = 412, description = "The condition did not hold", body = t::Error),
    ))]
async fn put_resource(
    api: &Api,
    req: &Request,
    access: &Access,
    id: &Hash,
    target: &str,
) -> Result {
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
    put(api, req, access, id, target, bytes, &mt).await
}

/// Change a JSON resource with a JSON Merge Patch (RFC 7396).
#[utoipa::path(patch, path = "/api/spaces/{space}/resources/{type}/{id}", tag = "resources",
    params(
        ("space" = String, Path, description = "A space's name, ID or unique ID prefix"),
        ("type" = String, Path, description = "The resource type"),
        ("id" = String, Path, description = "The resource ID"),
        ("If-Match" = Option<String>, Header, description = "Only if the current ETag matches"),
        ("Idempotency-Key" = Option<String>, Header, description = "Retries with the same key are applied once"),
    ),
    request_body(content((Object = "application/merge-patch+json"), (Object = "application/json"))),
    responses(
        (status = 200, description = "Patched", body = t::Written, headers(("ETag" = String))),
        (status = 412, description = "The condition did not hold", body = t::Error),
        (status = 415, description = "Not JSON", body = t::Error),
    ))]
async fn patch_resource(
    api: &Api,
    req: &Request,
    access: &Access,
    id: &Hash,
    target: &str,
) -> Result {
    let patch: Json = body(req)?;
    let Some((res, Some(bytes))) = api.node.get(id, target).await? else {
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
        id,
        target,
        (new, "application/json"),
        res.payload,
    )
    .await
}

/// Delete a resource. Its history stays until it is erased.
#[utoipa::path(delete, path = "/api/spaces/{space}/resources/{type}/{id}", tag = "resources",
    params(
        ("space" = String, Path, description = "A space's name, ID or unique ID prefix"),
        ("type" = String, Path, description = "The resource type"),
        ("id" = String, Path, description = "The resource ID"),
        ("If-Match" = Option<String>, Header, description = "Only if the current ETag matches"),
        ("Idempotency-Key" = Option<String>, Header, description = "Retries with the same key are applied once"),
    ),
    responses(
        (status = 204, description = "Deleted"),
        (status = 412, description = "The condition did not hold", body = t::Error),
    ))]
#[expect(
    dead_code,
    reason = "only describes the route; `delete_resource` answers it"
)]
fn delete() {}

/// Erase a resource: its payloads are forgotten everywhere, history
/// included.
#[utoipa::path(post, path = "/api/spaces/{space}/erase/{type}/{id}", tag = "resources",
    params(
        ("space" = String, Path, description = "A space's name, ID or unique ID prefix"),
        ("type" = String, Path, description = "The resource type"),
        ("id" = String, Path, description = "The resource ID"),
    ),
    responses((status = 204, description = "Erased")))]
async fn erase(api: &Api, access: &Access, s: &str, target: &str) -> Result {
    let id = space(api, access, s).await?;
    sync::write(api, access, &id, Kind::Erase, target, None, "", None).await?;
    Ok(empty(StatusCode::NO_CONTENT))
}

// Finding things

/// Full-text search in every space the caller can read.
#[utoipa::path(get, path = "/api/search", tag = "find",
    params(t::SearchQuery),
    responses((status = 200, body = Vec<t::Row>)))]
async fn search(api: &Api, req: &Request, access: &Access) -> Result {
    signed_in(access)?;
    let q: t::SearchQuery = params(req)?;
    let found = api
        .node
        .index
        .search(&q.q, &hexes(&readable(api, access)), limit(q.limit, 50))
        .await?;
    Ok(typed(StatusCode::OK, &rows(access, found)))
}

/// Everything by the time it is about, newest first.
#[utoipa::path(get, path = "/api/timeline", tag = "find",
    params(t::TimelineQuery),
    responses((status = 200, body = t::Listing)))]
async fn timeline(api: &Api, req: &Request, access: &Access) -> Result {
    signed_in(access)?;
    let q: t::TimelineQuery = params(req)?;
    let page = Paged::new(q.limit, q.offset, q.page, 50);
    let found = api
        .node
        .index
        .timeline_page(
            &hexes(&readable(api, access)),
            q.before,
            page.query_limit(),
            page.offset,
            page.types(access),
        )
        .await?;
    Ok(page.response(access, found))
}

/// What has a place near a point.
#[utoipa::path(get, path = "/api/near", tag = "find",
    params(t::NearQuery),
    responses((status = 200, body = Vec<t::Row>)))]
async fn near(api: &Api, req: &Request, access: &Access) -> Result {
    signed_in(access)?;
    let q: t::NearQuery = params(req)?;
    let found = api
        .node
        .index
        .near(
            q.lat,
            q.lon,
            q.km.unwrap_or(1.0),
            &hexes(&readable(api, access)),
            limit(q.limit, 50),
        )
        .await?;
    Ok(typed(StatusCode::OK, &rows(access, found)))
}

/// What links to a resource, or is linked from it.
#[utoipa::path(get, path = "/api/related/{space}/{type}/{id}", tag = "find",
    params(
        ("space" = String, Path, description = "A space's name, ID or unique ID prefix"),
        ("type" = String, Path, description = "The resource type"),
        ("id" = String, Path, description = "The resource ID"),
        t::Limit,
    ),
    responses((status = 200, body = Vec<t::Related>)))]
async fn related(api: &Api, req: &Request, access: &Access, s: &str, target: &str) -> Result {
    let id = space(api, access, s).await?;
    let q: t::Limit = params(req)?;
    let found = api
        .node
        .index
        .related(&id.to_hex(), target, limit(q.limit, 100))
        .await?;
    let allowed = hexes(&readable(api, access));
    let out: Vec<t::Related> = found
        .into_iter()
        .filter(|(_, r)| allowed.contains(&r.space) && type_ok(access, &r.kind))
        .map(|(rel, row)| t::Related {
            rel,
            row: row.into(),
        })
        .collect();
    Ok(typed(StatusCode::OK, &out))
}

// Blobs

fn can_write_somewhere(access: &Access) -> bool {
    match access {
        Access::Public => false,
        Access::Grant { role, .. } => *role >= Role::Writer,
        Access::Node { roles, .. } => roles.iter().any(|(_, r)| *r >= Role::Writer),
    }
}

/// Store bytes by their hash. A resource refers to them by `hash`.
#[utoipa::path(post, path = "/api/blobs", tag = "blobs",
    request_body(content(("application/octet-stream"))),
    responses((status = 201, body = t::Blob, headers(("Location" = String)))))]
fn put_blob(api: &Api, req: &Request, access: &Access) -> Result {
    if !can_write_somewhere(access) {
        return Err(if matches!(access, Access::Public) {
            ApiError::unauthorized()
        } else {
            ApiError::forbidden("needs writer")
        });
    }
    let hash = api.node.put_blob(req.body())?;
    let mut r = typed(
        StatusCode::CREATED,
        &t::Blob {
            hash: hash.to_hex(),
            size: req.body().len() as u64,
        },
    );
    set_header(
        &mut r,
        header::LOCATION,
        &format!("/api/blobs/{}", hash.to_hex()),
    );
    Ok(r)
}

/// Stored bytes, or a thumbnail of an image. Supports `Range`.
#[utoipa::path(get, path = "/api/blobs/{hash}", tag = "blobs",
    params(
        ("hash" = String, Path, description = "The blob's hash (hex)"),
        ("Range" = Option<String>, Header, description = "One range of bytes"),
        t::BlobQuery,
    ),
    responses(
        (status = 200, description = "The bytes", content_type = "application/octet-stream"),
        (status = 206, description = "Part of the bytes", content_type = "application/octet-stream"),
        (status = 304, description = "Not modified"),
        (status = 416, description = "The range is outside the blob"),
    ))]
async fn get_blob(api: &Api, req: &Request, access: &Access, h: &str) -> Result {
    signed_in(access)?;
    let hash = Hash::from_hex(h).ok_or_else(ApiError::not_found)?;
    let tag = etag(&hash);
    if hdr(req, header::IF_NONE_MATCH).is_some_and(|m| matches_etag(m, &tag)) {
        return Ok(empty(StatusCode::NOT_MODIFIED));
    }
    let bytes = api.node.blob(&hash)?.ok_or_else(ApiError::not_found)?;
    let q: t::BlobQuery = params(req)?;
    let mut mt = q
        .media_type
        .unwrap_or_else(|| "application/octet-stream".into());
    let mut bytes = bytes;
    if let Some(side) = q.thumb {
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

/// Changes as they happen, as Server-Sent Events. Each `data:` line is an
/// `Event`; `id:` is its sequence number.
#[utoipa::path(get, path = "/api/events", tag = "events",
    responses((status = 200, description = "An endless stream of `Event`s", content_type = "text/event-stream")))]
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
            let data = serde_json::to_string(&t::Event {
                space: ev.space.to_hex(),
                seq: ev.seq,
                kind: ev.kind.as_str().to_string(),
                target: ev.target.clone(),
            })
            .unwrap_or_default();
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

// Grants

/// The grants that are not revoked. Owner only.
#[utoipa::path(get, path = "/api/grants", tag = "grants",
    responses((status = 200, body = Vec<t::Grant>)))]
async fn list_grants(api: &Api, access: &Access) -> Result {
    require_owner(access)?;
    let list: Vec<t::Grant> = api
        .node
        .grants()
        .await
        .into_iter()
        .filter_map(|g| serde_json::from_value(g).ok())
        .collect();
    Ok(typed(StatusCode::OK, &list))
}

/// Make a token. Owner only.
#[utoipa::path(post, path = "/api/grants", tag = "grants",
    request_body = t::NewGrant,
    responses((status = 201, body = t::NewToken)))]
async fn new_grant(api: &Api, req: &Request, access: &Access) -> Result {
    require_owner(access)?;
    let new: t::NewGrant = body(req)?;
    Ok(typed(StatusCode::CREATED, &create_grant(api, &new).await?))
}

/// Revoke a grant; its token stops working. Owner only.
#[utoipa::path(delete, path = "/api/grants/{grant}", tag = "grants",
    params(("grant" = String, Path, description = "The grant's ID")),
    responses((status = 204, description = "Revoked")))]
async fn revoke_grant(api: &Api, access: &Access, gid: &str) -> Result {
    require_owner(access)?;
    api.node.revoke(gid).await?;
    Ok(empty(StatusCode::NO_CONTENT))
}

// The node

/// The owner's DID document.
#[utoipa::path(get, path = "/api/identity", tag = "node", security(()),
    responses((status = 200, description = "A DID document", content_type = "application/did+json")))]
fn identity(api: &Api) -> Response {
    let doc = hs_identity::did::document(&api.node.identity());
    respond(
        StatusCode::OK,
        "application/did+json",
        serde_json::to_vec(&doc).unwrap_or_default(),
    )
}

/// How the node is.
#[utoipa::path(get, path = "/api/health", tag = "node",
    responses((status = 200, body = t::Health)))]
async fn health(api: &Api, access: &Access) -> Result {
    signed_in(access)?;
    let index = api.node.index.healthy().await;
    let counts = api.node.index.counts().await?.into_iter().collect();
    Ok(typed(
        StatusCode::OK,
        &t::Health {
            ok: index,
            index,
            counts,
            spaces: api.node.space_ids().len() as u64,
            time: now_ms(),
        },
    ))
}

/// Everything the node keeps, as a tar archive. Owner only.
#[utoipa::path(get, path = "/api/export", tag = "node",
    responses((status = 200, description = "A tar archive", content_type = "application/x-tar")))]
#[expect(
    dead_code,
    reason = "only describes the route; `export::response` answers it"
)]
fn export() {}

/// The replicas the node copies itself to, without their secrets. Owner
/// only.
#[utoipa::path(get, path = "/api/replicas", tag = "node",
    responses((status = 200, body = Vec<String>)))]
fn list_replicas(api: &Api, access: &Access) -> Result {
    require_owner(access)?;
    let list: Vec<String> = crate::replica::configured(api)
        .iter()
        .map(|s| crate::replica::redact(s))
        .collect();
    Ok(typed(StatusCode::OK, &list))
}

/// Copy the node to every replica now. Owner only.
#[utoipa::path(post, path = "/api/replicas/sync", tag = "node",
    responses((status = 200, body = Vec<t::ReplicaSync>)))]
async fn sync_replicas(api: &Api, access: &Access) -> Result {
    require_owner(access)?;
    Ok(typed(StatusCode::OK, &crate::replica::sync_all(api).await))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn etags_match_lists_and_weak_tags() {
        let tag = "\"abc\"";
        assert!(matches_etag("\"abc\"", tag));
        assert!(matches_etag("\"x\", W/\"abc\"", tag));
        assert!(matches_etag("*", tag));
        assert!(!matches_etag("\"x\"", tag));
    }
}
