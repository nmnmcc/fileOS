// SPDX-License-Identifier: AGPL-3.0-only

//! Keeping spaces in step between nodes.
//!
//! The primary of a space orders its entries and pushes each new one to the
//! homes of the other members. Followers apply what is pushed, pull from the
//! primary when they notice a gap, and pull every minute anyway in case a
//! push was lost. A follower that is asked to write signs the proposal
//! itself and forwards it to the primary's inbox, then pulls the result.
//!
//! Every request between nodes is signed with the node key (HTTP Message
//! Signatures); the receiving node gives it the roles that node holds in
//! its spaces.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hs_core::cbor::Value as Cbor;
use hs_core::cose::Sign1;
use hs_core::event::{Entry, Kind, Proposal};
use hs_core::{Hash, PublicKey};
use hs_node::{Access, now_ms};
use http::{Method, StatusCode};

use crate::Api;
use crate::condition::Conditions;
use crate::util::{ApiError, Result};

/// Entries per page of `/fed/spaces/{space}/log`.
pub const PAGE: usize = 1000;

/// How often followers pull, in case a push was lost.
pub const PULL_EVERY: Duration = Duration::from_secs(60);

/// Send a signed request to another node and return the response body.
///
/// # Errors
///
/// Returns an error if the URL is not valid, signing or transport fails, or the peer answers with a
/// status that is not success.
pub async fn fed(
    api: &Api,
    method: Method,
    url: &str,
    body: Vec<u8>,
) -> std::result::Result<Vec<u8>, String> {
    let response = send_fed(api, method, url, body).await?;
    if !response.status().is_success() {
        return Err(peer_error(url, &response));
    }
    Ok(response.into_body())
}

async fn send_fed(
    api: &Api,
    method: Method,
    url: &str,
    body: Vec<u8>,
) -> std::result::Result<http::Response<Vec<u8>>, String> {
    let req = http::Request::builder()
        .method(method)
        .uri(url)
        .header(http::header::CONTENT_TYPE, "application/cbor")
        .body(())
        .map_err(|e| format!("{url}: {e}"))?;
    let (mut parts, ()) = req.into_parts();
    hs_net::sig::sign(&mut parts, &body, &api.node.key, now_ms() / 1000)
        .map_err(|e| e.to_string())?;
    api.nodes
        .send(http::Request::from_parts(parts, body))
        .await
        .map_err(String::from)
}

fn peer_error(url: &str, resp: &http::Response<Vec<u8>>) -> String {
    let status = resp.status();
    let text = String::from_utf8_lossy(resp.body())
        .chars()
        .take(300)
        .collect::<String>();
    format!("{url}: {status} {text}")
}

fn url(home: &str, path: &str) -> String {
    format!("{}{path}", home.trim_end_matches('/'))
}

/// The payload hash an entry refers to.
pub fn payload_of(entry: &[u8]) -> Option<Hash> {
    let e = Entry::decode(&Sign1::decode(entry).ok()?.payload).ok()?;
    Proposal::decode(&Sign1::decode(&e.proposal).ok()?.payload)
        .ok()?
        .payload
}

fn seq_of(entry: &[u8]) -> Option<u64> {
    Entry::decode(&Sign1::decode(entry).ok()?.payload)
        .ok()
        .map(|e| e.seq)
}

/// Encode `[a, b|null]`.
pub fn pair(a: &[u8], b: Option<&[u8]>) -> Vec<u8> {
    Cbor::Array(vec![
        Cbor::Bytes(a.to_vec()),
        b.map_or(Cbor::Null, |b| Cbor::Bytes(b.to_vec())),
    ])
    .encode()
}

/// Decode `[a, b|null]`.
pub fn unpair(bytes: &[u8]) -> Option<(Vec<u8>, Option<Vec<u8>>)> {
    let v = Cbor::decode(bytes).ok()?;
    let a = v.as_array()?;
    let first = a.first()?.as_bytes()?.to_vec();
    let second = a.get(1).and_then(Cbor::as_bytes).map(<[u8]>::to_vec);
    Some((first, second))
}

pub(crate) fn unpack_conditional(bytes: &[u8]) -> Option<(Vec<u8>, Option<Vec<u8>>, Conditions)> {
    let value = Cbor::decode(bytes).ok()?;
    let [proposal, payload, conditions] = value.as_array()? else {
        return None;
    };
    let payload = match payload {
        Cbor::Null => None,
        other => Some(other.as_bytes()?.to_vec()),
    };
    Some((
        proposal.as_bytes()?.to_vec(),
        payload,
        Conditions::decode(conditions)?,
    ))
}

fn pack_conditional(proposal: &[u8], payload: Option<&[u8]>, conditions: &Conditions) -> Vec<u8> {
    Cbor::Array(vec![
        Cbor::Bytes(proposal.to_vec()),
        payload.map_or(Cbor::Null, |bytes| Cbor::Bytes(bytes.to_vec())),
        conditions.encode(),
    ])
    .encode()
}

/// Fetch entries from a home, after `after` (or from the start).
async fn fetch_entries(
    api: &Api,
    home: &str,
    id: &Hash,
    after: Option<u64>,
) -> std::result::Result<Vec<Vec<u8>>, String> {
    let mut out = Vec::new();
    let mut after = after;
    loop {
        let q = after.map(|a| format!("?after={a}")).unwrap_or_default();
        let body = fed(
            api,
            Method::GET,
            &url(home, &format!("/fed/spaces/{}/log{q}", id.to_hex())),
            Vec::new(),
        )
        .await?;
        let v = Cbor::decode(&body).map_err(|e| e.0.to_string())?;
        let page: Vec<Vec<u8>> = v
            .as_array()
            .ok_or("log is not an array")?
            .iter()
            .filter_map(|e| e.as_bytes().map(<[u8]>::to_vec))
            .collect();
        let n = page.len();
        if let Some(last) = page.last().and_then(|e| seq_of(e)) {
            after = Some(last);
        }
        out.extend(page);
        if n < PAGE {
            return Ok(out);
        }
    }
}

/// Fetch a payload from a home, if it still exists there.
async fn fetch_blob(api: &Api, home: &str, hash: &Hash) -> Option<Vec<u8>> {
    if let Ok(Some(b)) = api.node.blob(hash) {
        return Some(b);
    }
    let b = fed(
        api,
        Method::GET,
        &url(home, &format!("/fed/blobs/{}", hash.to_hex())),
        Vec::new(),
    )
    .await
    .ok()?;
    (Hash::of(&b) == *hash).then_some(b)
}

/// Start following a space whose primary is at `home`: copy its log and
/// payloads. This node must already be a member.
///
/// # Errors
///
/// Returns an error if the primary cannot be reached, rejects the request, or its history cannot be
/// verified or stored.
pub async fn join(api: &Api, home: &str, id: Hash) -> Result<u64> {
    let entries = fetch_entries(api, home, &id, None)
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    if entries.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_GATEWAY,
            "the home returned no entries",
        ));
    }
    let mut payloads = BTreeMap::new();
    for e in &entries {
        if let Some(h) = payload_of(e)
            && !payloads.contains_key(&h)
            && let Some(b) = fetch_blob(api, home, &h).await
        {
            payloads.insert(h, b);
        }
    }
    api.node.follow(id, &entries, &payloads, home).await?;
    update_home(api, &id).await;
    Ok(api.node.with_space(&id, |l| l.space.seq).await?)
}

/// Pull new entries of a space from its primary. Returns how many were
/// applied.
///
/// # Errors
///
/// Returns an error if the primary cannot be reached or an entry cannot be verified or stored.
pub async fn pull(api: &Api, id: &Hash) -> Result<usize> {
    let (home, seq, primary) = api
        .node
        .with_space(id, |l| (l.home.clone(), l.space.seq, l.space.primary))
        .await?;
    if primary == api.node.key.public() {
        return Ok(0);
    }
    let Some(home) = home else { return Ok(0) };
    let entries = fetch_entries(api, &home, id, Some(seq))
        .await
        .map_err(|e| ApiError::new(StatusCode::BAD_GATEWAY, e))?;
    let mut n = 0;
    for e in &entries {
        let payload = match payload_of(e) {
            Some(h) => fetch_blob(api, &home, &h).await,
            None => None,
        };
        api.node.apply_remote(id, e, payload.as_deref()).await?;
        n += 1;
    }
    if n > 0 {
        update_home(api, id).await;
    }
    Ok(n)
}

/// After a handover, point the space at the new primary's home.
async fn update_home(api: &Api, id: &Hash) {
    let Ok(Some(home)) = api
        .node
        .with_space(id, |l| {
            if l.space.primary == api.node.key.public() {
                return None;
            }
            l.space
                .members
                .get(&l.space.primary)
                .and_then(|m| m.home.clone())
                .filter(|h| l.home.as_ref() != Some(h))
        })
        .await
    else {
        return;
    };
    let _ = api.node.set_space_home(id, &home).await;
}

/// Write for a caller. If another node is the primary, the signed
/// proposal goes to its inbox and the result is pulled back. Returns the
/// sequence number.
///
/// # Errors
///
/// Returns an error if access is denied, the proposal is invalid, or the primary or local storage
/// fails.
#[expect(
    clippy::too_many_arguments,
    reason = "the signature carries the signed proposal fields"
)]
pub async fn write(
    api: &Api,
    access: &Access,
    id: &Hash,
    kind: Kind,
    target: &str,
    payload: Option<&[u8]>,
    media_type: &str,
    nonce: Option<Vec<u8>>,
) -> Result<u64> {
    let signed = api
        .node
        .propose(access, id, kind, target, payload, media_type, nonce)?;
    Ok(
        write_checked(api, id, &signed, payload, &Conditions::default())
            .await?
            .seq,
    )
}

#[derive(serde::Deserialize)]
struct Receipt {
    seq: u64,
    created: Option<bool>,
}

pub(crate) async fn write_checked(
    api: &Api,
    id: &Hash,
    signed: &[u8],
    payload: Option<&[u8]>,
    conditions: &Conditions,
) -> Result<hs_node::Committed> {
    let local = conditions.clone();
    match api
        .node
        .commit(id, signed, payload, move |current| local.check(current))
        .await
    {
        Ok(committed) => Ok(committed),
        Err(error) => {
            let hs_node::ErrorKind::NotPrimary {
                home: Some(home), ..
            } = error.kind()
            else {
                return Err(error.into());
            };
            let inbox = format!("/fed/spaces/{}/inbox", id.to_hex());
            let gateway = |message| ApiError::new(StatusCode::BAD_GATEWAY, message);
            let (path, request) = if conditions.is_empty() {
                (inbox, pair(signed, payload))
            } else {
                // Older primaries say no to the separate endpoint. They do not take
                // a write with conditions they do not understand.
                (
                    format!("{inbox}/conditional"),
                    pack_conditional(signed, payload, conditions),
                )
            };
            let endpoint = url(home, &path);
            let response = send_fed(api, Method::POST, &endpoint, request)
                .await
                .map_err(gateway)?;
            match response.status() {
                StatusCode::PRECONDITION_FAILED => {
                    return Err(ApiError::new(
                        StatusCode::PRECONDITION_FAILED,
                        "the resource has changed",
                    ));
                }
                StatusCode::CONFLICT => {
                    return Err(ApiError::new(
                        StatusCode::CONFLICT,
                        "idempotency key already used for a different request",
                    ));
                }
                StatusCode::NOT_FOUND
                    if response.headers().contains_key("hs-conditional-write") =>
                {
                    return Err(ApiError::not_found());
                }
                status if status.is_success() => {}
                _ => return Err(gateway(peer_error(&endpoint, &response))),
            }
            let receipt: Receipt = serde_json::from_slice(response.body())
                .map_err(|error| gateway(format!("commit receipt is not valid: {error}")))?;
            pull(api, id).await?;
            let mut committed = api.node.committed(id, receipt.seq, signed).await?;
            committed.created = receipt.created;
            Ok(committed)
        }
    }
}

/// Apply an entry pushed by a primary; on a gap, pull instead.
///
/// # Errors
///
/// Returns an error if the space cannot be loaded, pulled or updated.
pub async fn receive(api: &Api, id: &Hash, entry: &[u8], payload: Option<&[u8]>) -> Result<u64> {
    let have = api.node.with_space(id, |l| l.space.seq).await?;
    match seq_of(entry) {
        Some(s) if s <= have.saturating_add(1) => {
            let seq = api.node.apply_remote(id, entry, payload).await?;
            update_home(api, id).await;
            Ok(seq)
        }
        _ => {
            pull(api, id).await?;
            Ok(api.node.with_space(id, |l| l.space.seq).await?)
        }
    }
}

/// The homes of the other members of a space.
async fn member_homes(api: &Api, id: &Hash) -> Vec<(PublicKey, String)> {
    let me = api.node.key.public();
    api.node
        .with_space(id, |l| {
            l.space
                .members
                .iter()
                .filter(|(k, _)| **k != me)
                .filter_map(|(k, m)| m.home.clone().map(|h| (*k, h)))
                .collect()
        })
        .await
        .unwrap_or_default()
}

/// Start pushing and pulling.
pub fn start(api: Arc<Api>) {
    let pusher = Arc::clone(&api);
    tokio::spawn(async move {
        let mut events = pusher.node.subscribe();
        loop {
            let ev = match events.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            };
            if !ev.ordered_here {
                continue;
            }
            let payload = payload_of(&ev.entry).and_then(|h| pusher.node.blob(&h).ok().flatten());
            let body = pair(&ev.entry, payload.as_deref());
            for (_, home) in member_homes(&pusher, &ev.space).await {
                let api = Arc::clone(&pusher);
                let body = body.clone();
                let path = format!("/fed/spaces/{}/push", ev.space.to_hex());
                tokio::spawn(async move {
                    let _ = fed(&api, Method::POST, &url(&home, &path), body).await;
                });
            }
            if ev.kind == Kind::Handover {
                update_home(&pusher, &ev.space).await;
            }
        }
    });
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(PULL_EVERY).await;
            for id in api.node.space_ids() {
                let _ = pull(&api, &id).await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs() {
        assert_eq!(
            unpair(&pair(b"a", Some(b"b"))),
            Some((b"a".to_vec(), Some(b"b".to_vec())))
        );
        assert_eq!(unpair(&pair(b"a", None)), Some((b"a".to_vec(), None)));
    }
}
