// SPDX-License-Identifier: AGPL-3.0-only

//! Web Push: tell the owner's browsers when something changes in a shared
//! space.
//!
//! Subscriptions are `push/<hash>` resources in the private space, so they
//! travel with the owner's data. The VAPID key is `keys/vapid`. Changes are
//! gathered for a few seconds and sent as one notification per
//! subscription; a push service that answers 404 or 410 drops the
//! subscription.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use hs_core::Hash;
use hs_core::event::Kind;
use hs_net::push::Vapid;
use hs_node::{Access, now_ms};
use http::{Method, StatusCode};
use serde_json::{Value as Json, json};

use crate::Api;
use crate::util::{ApiError, Result, body_json, json_ok, json_status};
use hs_net::server::Request;

const GATHER: Duration = Duration::from_secs(5);

fn vapid(api: &Api) -> Option<Vapid> {
    let path = api.node.dir().join("keys").join("vapid");
    if let Ok(b) = std::fs::read(&path) {
        return Vapid::from_pkcs8(&b).ok();
    }
    let v = Vapid::generate().ok()?;
    std::fs::write(&path, v.pkcs8()).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Some(v)
}

fn subject(api: &Api) -> String {
    let c = api.node.config();
    c.raw
        .get("push_subject")
        .and_then(Json::as_str)
        .map(String::from)
        .or(c.url)
        .unwrap_or_else(|| "mailto:owner@homeserver.invalid".into())
}

async fn subscriptions(api: &Api) -> Vec<(String, Json)> {
    let private = api.node.config().private;
    let rows = api
        .node
        .index
        .list(&private.to_hex(), Some("push"), 1000, 0)
        .await
        .unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| Some((r.target, r.body?)))
        .collect()
}

/// Send one notification to every subscription.
pub async fn notify(api: &Api, message: &Json) {
    let Some(vapid) = vapid(api) else { return };
    let subject = subject(api);
    let payload = serde_json::to_vec(message).unwrap_or_default();
    let private = api.node.config().private;
    for (target, sub) in subscriptions(api).await {
        let Ok(req) = hs_net::push::request(&sub, &payload, &vapid, &subject, now_ms() / 1000)
        else {
            continue;
        };
        match api.web.send(req).await {
            Ok(r) if r.status() == StatusCode::NOT_FOUND || r.status() == StatusCode::GONE => {
                let _ = api
                    .node
                    .write(
                        &api.node.owner(),
                        &private,
                        Kind::Delete,
                        &target,
                        None,
                        "",
                        None,
                    )
                    .await;
            }
            _ => {}
        }
    }
}

fn worth_telling(api: &Api, kind: Kind, space: &Hash, target: &str) -> bool {
    if kind != Kind::Put || *space == api.node.config().private {
        return false;
    }
    let t = target.split('/').next().unwrap_or("");
    !matches!(
        t,
        "push" | "credential" | "oauth-client" | "grant" | "cipher"
    )
}

/// Watch for changes and notify.
pub fn start(api: Arc<Api>) {
    tokio::spawn(async move {
        let mut events = api.node.subscribe();
        loop {
            let first = match events.recv().await {
                Ok(ev) => ev,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => return,
            };
            if !worth_telling(&api, first.kind, &first.space, &first.target) {
                continue;
            }
            let mut changed: BTreeMap<Hash, Vec<String>> = BTreeMap::new();
            changed.entry(first.space).or_default().push(first.target);
            let deadline = tokio::time::Instant::now() + GATHER;
            while let Ok(Ok(ev)) = tokio::time::timeout_at(deadline, events.recv()).await {
                if worth_telling(&api, ev.kind, &ev.space, &ev.target) {
                    changed.entry(ev.space).or_default().push(ev.target);
                }
            }
            let mut names = Vec::new();
            let mut total = 0;
            for (id, targets) in &changed {
                total += targets.len();
                if let Ok(n) = api
                    .node
                    .with_space(id, |l| l.space.name().to_string())
                    .await
                {
                    names.push(n);
                }
            }
            let (space, target) = changed
                .iter()
                .next()
                .map(|(s, t)| (s.to_hex(), t[0].clone()))
                .unwrap_or_default();
            let body = if total == 1 {
                format!("{target} changed")
            } else {
                format!("{total} changes")
            };
            notify(&api, &json!({ "title": names.join(", "), "body": body, "space": space, "target": target })).await;
        }
    });
}

/// `/api/push/key` and `/api/push/subscribe`.
pub async fn handle(api: &Api, req: &Request, access: &Access, rest: &[&str]) -> Result {
    if !access.is_owner() {
        return Err(ApiError::unauthorized());
    }
    match (req.method().clone(), rest) {
        (Method::GET, ["key"]) => {
            let v = vapid(api)
                .ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "no push key"))?;
            Ok(json_ok(&json!({ "key": v.public_key() })))
        }
        (Method::POST, ["subscribe"]) => {
            let sub = body_json(req)?;
            let endpoint = sub
                .get("endpoint")
                .and_then(Json::as_str)
                .ok_or_else(|| ApiError::bad("endpoint missing"))?;
            if !endpoint.starts_with("https://")
                || sub.pointer("/keys/p256dh").is_none()
                || sub.pointer("/keys/auth").is_none()
            {
                return Err(ApiError::bad("not a push subscription"));
            }
            let target = format!("push/{}", &Hash::of(endpoint.as_bytes()).to_hex()[..32]);
            let mut v = sub.clone();
            v["title"] = "Push subscription".into();
            let bytes = serde_json::to_vec(&v).unwrap_or_default();
            let private = api.node.config().private;
            api.node
                .write(
                    access,
                    &private,
                    Kind::Put,
                    &target,
                    Some(&bytes),
                    "application/json",
                    None,
                )
                .await?;
            Ok(json_status(StatusCode::CREATED, &json!({ "id": target })))
        }
        (Method::POST, ["test"]) => {
            notify(
                api,
                &json!({ "title": api.node.config().name, "body": "Notifications work." }),
            )
            .await;
            Ok(json_ok(&json!({ "ok": true })))
        }
        _ => Err(ApiError::not_found()),
    }
}
