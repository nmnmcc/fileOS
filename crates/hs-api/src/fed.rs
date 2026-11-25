// SPDX-License-Identifier: AGPL-3.0-only

//! What nodes ask each other, under `/fed/`. Every request is signed by the
//! asking node; it gets the roles that node holds in this node's spaces.
//!
//! - `GET /fed/spaces/{space}/log?after=N`: entries, a CBOR array of byte
//!   strings, at most [`PAGE`] at a time.
//! - `GET /fed/spaces/{space}/head`: the signed head, from the primary.
//! - `GET /fed/blobs/{hash}`: a payload of a space the asker belongs to.
//! - `POST /fed/spaces/{space}/inbox`: `[proposal, payload|null]`, ordered by
//!   the primary; answers `{"seq": N}`.
//! - `POST /fed/spaces/{space}/inbox/conditional`: the same write with
//!   version preconditions in the signed request body.
//! - `POST /fed/spaces/{space}/push`: `[entry, payload|null]` from the
//!   primary.

use hs_core::Hash;
use hs_core::cbor::Value as Cbor;
use hs_node::Access;
use http::{Method, StatusCode};
use serde_json::json;

use crate::Api;
use crate::sync::{self, PAGE};
use crate::util::{ApiError, Result, json_ok, respond};
use hs_net::server::Request;

fn space(access: &Access, s: &str) -> Result<Hash> {
    let id = Hash::from_hex(s).ok_or_else(ApiError::not_found)?;
    if !access.can_read(&id) {
        return Err(ApiError::forbidden("not a member of this space"));
    }
    Ok(id)
}

pub async fn handle(api: &Api, req: &Request, access: &Access, segs: &[&str]) -> Result {
    if !matches!(access, Access::Node { .. }) {
        return Err(ApiError::unauthorized());
    }
    let m = req.method().clone();
    match (m, segs) {
        (Method::GET, ["spaces", s, "log"]) => {
            let id = space(access, s)?;
            let q = crate::util::query(req);
            let after = crate::util::param(&q, "after").and_then(|a| a.parse().ok());
            let entries = api.node.entries(&id, after, PAGE).await?;
            let body = Cbor::Array(entries.into_iter().map(Cbor::Bytes).collect()).encode();
            Ok(respond(StatusCode::OK, "application/cbor", body))
        }
        (Method::GET, ["spaces", s, "head"]) => {
            let id = space(access, s)?;
            let key = &api.node.key;
            match api.node.with_space(&id, |l| l.signed_head(key)).await? {
                Some(h) => Ok(respond(StatusCode::OK, "application/cose", h)),
                None => Err(ApiError::new(
                    StatusCode::MISDIRECTED_REQUEST,
                    "not the primary",
                )),
            }
        }
        (Method::GET, ["blobs", h]) => {
            let hash = Hash::from_hex(h).ok_or_else(ApiError::not_found)?;
            let Access::Node { roles, .. } = access else {
                return Err(ApiError::unauthorized());
            };
            let mut allowed = false;
            for (s, _) in roles {
                if api.node.space_has_payload(s, &hash).await {
                    allowed = true;
                    break;
                }
            }
            if !allowed {
                return Err(ApiError::not_found());
            }
            match api.node.blob(&hash)? {
                Some(b) => Ok(respond(StatusCode::OK, "application/octet-stream", b)),
                None => Err(ApiError::not_found()),
            }
        }
        (Method::POST, ["spaces", s, "inbox"]) => {
            let id = space(access, s)?;
            let (proposal, payload) = sync::unpair(req.body())
                .ok_or_else(|| ApiError::bad("expected [proposal, payload]"))?;
            let (seq, _) = api.node.submit(&id, &proposal, payload.as_deref()).await?;
            Ok(json_ok(&json!({ "seq": seq })))
        }
        (Method::POST, ["spaces", s, "inbox", "conditional"]) => {
            let id = space(access, s)?;
            let (proposal, payload, conditions) = sync::unpack_conditional(req.body())
                .ok_or_else(|| ApiError::bad("expected [proposal, payload, conditions]"))?;
            match api
                .node
                .submit_checked(&id, &proposal, payload.as_deref(), |hash| {
                    conditions.check(hash)
                })
                .await
            {
                Ok((seq, _)) => Ok(json_ok(&json!({ "seq": seq }))),
                Err(error) if matches!(error.kind(), hs_node::ErrorKind::NotFound) => {
                    let mut response = ApiError::not_found().response();
                    crate::util::set_header(&mut response, "hs-conditional-write", "1");
                    Ok(response)
                }
                Err(error) => Err(error.into()),
            }
        }
        (Method::POST, ["spaces", s, "push"]) => {
            let id = space(access, s)?;
            let (entry, payload) = sync::unpair(req.body())
                .ok_or_else(|| ApiError::bad("expected [entry, payload]"))?;
            let seq = sync::receive(api, &id, &entry, payload.as_deref()).await?;
            Ok(json_ok(&json!({ "seq": seq })))
        }
        _ => Err(ApiError::not_found()),
    }
}
