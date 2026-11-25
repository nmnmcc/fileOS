// SPDX-License-Identifier: AGPL-3.0-only

//! A password vault that Bitwarden apps can use: point an app's self-hosted
//! server URL at `https://<node>/bitwarden`.
//!
//! The apps encrypt everything before sending it; this node only keeps the
//! ciphertext. Items are `cipher/<id>` and folders `vault-folder/<id>` in the
//! private space; the account (keys wrapped by the master password, and a
//! salted hash of the client's password hash) is `vault-account/main`.
//!
//! This is the subset the apps need to sign in, sync, and edit items. One
//! account per node; registering is allowed only while `bitwarden_register`
//! is true in the config and no account exists yet.

use hs_core::event::Kind;
use hs_node::{Access, now_ms, random};
use http::{Method, StatusCode};
use ring::pbkdf2;
use serde_json::{Value as Json, json};
use std::num::NonZeroU32;

use crate::util::{ApiError, Result, b64url, body_json, json_ok, json_status, param, parse_form};
use crate::{Api, rest, sync};
use hs_net::server::Request;

const ACCOUNT: &str = "vault-account/main";
const SESSION_MS: u64 = 30 * 24 * 3600 * 1000;
const ACCESS_LIFETIME_S: u64 = 3600;
/// Marks a grant as a Bitwarden session; nothing else uses this type.
const MARK: &str = "vault-session";

/// `YYYY-MM-DDTHH:MM:SS.mmmZ` for Unix milliseconds.
pub fn iso(ms: u64) -> String {
    use time::macros::format_description;
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms) * 1_000_000)
        .ok()
        .and_then(|t| {
            t.format(format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second].[subsecond digits:3]Z"
            ))
            .ok()
        })
        .unwrap_or_default()
}

/// A field by camelCase or `PascalCase` name.
fn get<'a>(v: &'a Json, name: &str) -> Option<&'a Json> {
    v.get(name).or_else(|| {
        let mut c = name.chars();
        let pascal: String = c
            .next()
            .map(|f| f.to_ascii_uppercase())
            .into_iter()
            .chain(c)
            .collect();
        v.get(pascal)
    })
}

fn get_str<'a>(v: &'a Json, name: &str) -> Option<&'a str> {
    get(v, name).and_then(Json::as_str)
}

fn uuid() -> String {
    let b = random::<16>();
    let h = data_encoding::HEXLOWER.encode(&b);
    format!(
        "{}-{}-4{}-a{}-{}",
        &h[..8],
        &h[8..12],
        &h[13..16],
        &h[17..20],
        &h[20..32]
    )
}

const ITERATIONS: u32 = 100_000;

fn hash_password(password: &str, salt: &[u8]) -> Vec<u8> {
    let mut out = [0u8; 32];
    let n = NonZeroU32::new(ITERATIONS).unwrap_or(NonZeroU32::MIN);
    pbkdf2::derive(
        pbkdf2::PBKDF2_HMAC_SHA256,
        n,
        salt,
        password.as_bytes(),
        &mut out,
    );
    out.to_vec()
}

fn check_password(account: &Json, password: &str) -> bool {
    let (Some(salt), Some(hash)) = (
        account
            .get("salt")
            .and_then(Json::as_str)
            .and_then(crate::util::unb64),
        account
            .get("password")
            .and_then(Json::as_str)
            .and_then(crate::util::unb64),
    ) else {
        return false;
    };
    let n = NonZeroU32::new(ITERATIONS).unwrap_or(NonZeroU32::MIN);
    pbkdf2::verify(
        pbkdf2::PBKDF2_HMAC_SHA256,
        n,
        &salt,
        password.as_bytes(),
        &hash,
    )
    .is_ok()
}

async fn account(api: &Api) -> Option<Json> {
    let private = api.node.config().private;
    let (_, b) = api.node.get(&private, ACCOUNT).await.ok()??;
    serde_json::from_slice(&b?).ok()
}

async fn save(api: &Api, access: &Access, target: &str, v: &Json) -> Result<u64> {
    let private = api.node.config().private;
    let bytes = serde_json::to_vec(v).unwrap_or_default();
    sync::write(
        api,
        access,
        &private,
        Kind::Put,
        target,
        Some(&bytes),
        "application/json",
        None,
    )
    .await
}

fn prelogin(acct: Option<&Json>) -> Json {
    let kdf = acct.and_then(|a| a.get("kdf")).cloned().unwrap_or(json!(0));
    let it = acct
        .and_then(|a| a.get("kdfIterations"))
        .cloned()
        .unwrap_or(json!(600_000));
    json!({ "kdf": kdf, "kdfIterations": it, "kdfMemory": null, "kdfParallelism": null })
}

async fn register(api: &Api, b: &Json) -> Result {
    if !api
        .node
        .config()
        .raw
        .get("bitwarden_register")
        .and_then(Json::as_bool)
        .unwrap_or(false)
    {
        return Err(ApiError::forbidden(
            "registration is closed; set bitwarden_register to true",
        ));
    }
    if account(api).await.is_some() {
        return Err(ApiError::forbidden("this node already has a vault account"));
    }
    let email = get_str(b, "email")
        .ok_or_else(|| ApiError::bad("email missing"))?
        .to_lowercase();
    let password = get_str(b, "masterPasswordHash")
        .ok_or_else(|| ApiError::bad("masterPasswordHash missing"))?;
    let key = get_str(b, "key")
        .or_else(|| get_str(b, "userSymmetricKey"))
        .ok_or_else(|| ApiError::bad("key missing"))?;
    let keys = get(b, "keys")
        .or_else(|| get(b, "userAsymmetricKeys"))
        .cloned()
        .unwrap_or(Json::Null);
    let salt = random::<16>();
    let now = now_ms();
    let acct = json!({
        "title": "Password vault",
        "id": uuid(),
        "email": email,
        "name": get_str(b, "name").unwrap_or(&email),
        "hint": get_str(b, "masterPasswordHint"),
        "salt": b64url(&salt),
        "password": b64url(&hash_password(password, &salt)),
        "key": key,
        "publicKey": get_str(&keys, "publicKey"),
        "privateKey": get_str(&keys, "encryptedPrivateKey"),
        "kdf": get(b, "kdf").cloned().unwrap_or(json!(0)),
        "kdfIterations": get(b, "kdfIterations").cloned().unwrap_or(json!(600_000)),
        "securityStamp": uuid(),
        "created": now,
    });
    save(api, &api.node.owner(), ACCOUNT, &acct).await?;
    Ok(json_ok(
        &json!({ "object": "register", "captchaBypassToken": null }),
    ))
}

async fn token_response(api: &Api, acct: &Json) -> Result {
    let private = api.node.config().private;
    let grant = rest::create_grant(
        api,
        &crate::types::NewGrant {
            spaces: Some(vec![private.to_hex()]),
            types: Some(vec!["cipher".into(), "vault-folder".into(), MARK.into()]),
            expires: Some(now_ms() + SESSION_MS),
            session: Some(true),
            ..crate::types::NewGrant::new(crate::types::Role::Writer, "Bitwarden app")
        },
    )
    .await?;
    let token = grant.token.as_str();
    let mut r = json_ok(&json!({
        "access_token": token,
        "refresh_token": token,
        "expires_in": ACCESS_LIFETIME_S,
        "token_type": "Bearer",
        "scope": "api offline_access",
        "Key": acct.get("key"),
        "PrivateKey": acct.get("privateKey"),
        "Kdf": acct.get("kdf"),
        "KdfIterations": acct.get("kdfIterations"),
        "KdfMemory": null,
        "KdfParallelism": null,
        "ResetMasterPassword": false,
        "ForcePasswordReset": false,
        "MasterPasswordPolicy": { "object": "masterPasswordPolicy" },
        "UserDecryptionOptions": { "HasMasterPassword": true, "Object": "userDecryptionOptions" },
    }));
    crate::util::set_header(&mut r, http::header::CACHE_CONTROL, "no-store");
    Ok(r)
}

fn token_error(msg: &str) -> hs_net::server::Response {
    json_status(
        StatusCode::BAD_REQUEST,
        &json!({ "error": "invalid_grant", "error_description": msg, "ErrorModel": { "Message": msg, "Object": "error" } }),
    )
}

async fn connect_token(api: &Api, req: &Request) -> Result {
    let f = parse_form(&String::from_utf8_lossy(req.body()));
    let Some(acct) = account(api).await else {
        return Ok(token_error("no vault account on this node"));
    };
    match param(&f, "grant_type") {
        Some("password") => {
            let user = param(&f, "username").unwrap_or("").to_lowercase();
            let pass = param(&f, "password").unwrap_or("");
            if acct.get("email").and_then(Json::as_str) != Some(user.as_str())
                || !check_password(&acct, pass)
            {
                return Ok(token_error("Username or password is incorrect. Try again."));
            }
            token_response(api, &acct).await
        }
        Some("refresh_token") => {
            let old = param(&f, "refresh_token").unwrap_or("");
            let Some(access) = api.node.authenticate(old).await else {
                return Ok(token_error("session expired"));
            };
            if !is_session(&access) {
                return Ok(token_error("not a vault session"));
            }
            if let Some(id) = access.grant_id() {
                let _ = api.node.revoke(&id).await;
            }
            token_response(api, &acct).await
        }
        _ => Ok(token_error("unsupported grant_type")),
    }
}

fn is_session(a: &Access) -> bool {
    matches!(a, Access::Grant { types: Some(t), .. } if t.iter().any(|x| x == MARK))
}

fn cipher_json(id: &str, v: &Json, revised: u64, created: u64) -> Json {
    let mut c = v.clone();
    if let Some(o) = c.as_object_mut() {
        for k in ["title", "Title"] {
            o.remove(k);
        }
        o.insert("id".into(), id.into());
        o.insert("object".into(), "cipherDetails".into());
        o.insert("revisionDate".into(), iso(revised).into());
        o.insert("creationDate".into(), iso(created).into());
        o.entry("deletedDate").or_insert(Json::Null);
        o.insert("organizationId".into(), Json::Null);
        o.insert("collectionIds".into(), json!([]));
        o.insert("edit".into(), true.into());
        o.insert("viewPassword".into(), true.into());
        o.insert("organizationUseTotp".into(), true.into());
        o.entry("attachments").or_insert(Json::Null);
        o.entry("favorite").or_insert(false.into());
        o.entry("reprompt").or_insert(0.into());
    }
    c
}

async fn listed(api: &Api, kind: &str) -> Vec<(String, Json, u64)> {
    let private = api.node.config().private;
    let rows = api
        .node
        .index
        .list(&private.to_hex(), Some(kind), 100_000, 0)
        .await
        .unwrap_or_default();
    rows.into_iter()
        .filter_map(|r| {
            let id = r.target.split_once('/')?.1.to_string();
            Some((id, r.body?, r.updated))
        })
        .collect()
}

async fn sync_all(api: &Api, acct: &Json) -> Json {
    let ciphers: Vec<Json> = listed(api, "cipher")
        .await
        .iter()
        .map(|(id, v, t)| {
            cipher_json(
                id,
                v,
                *t,
                v.get("created").and_then(Json::as_u64).unwrap_or(*t),
            )
        })
        .collect();
    let folders: Vec<Json> = listed(api, "vault-folder")
        .await
        .iter()
        .map(|(id, v, t)| json!({ "id": id, "name": get_str(v, "name"), "revisionDate": iso(*t), "object": "folder" }))
        .collect();
    json!({
        "object": "sync",
        "profile": profile(acct),
        "folders": folders,
        "collections": [],
        "ciphers": ciphers,
        "domains": null,
        "policies": [],
        "sends": [],
    })
}

fn profile(acct: &Json) -> Json {
    json!({
        "object": "profile",
        "id": acct.get("id"),
        "name": acct.get("name"),
        "email": acct.get("email"),
        "emailVerified": true,
        "premium": true,
        "premiumFromOrganization": false,
        "masterPasswordHint": null,
        "culture": "en-US",
        "twoFactorEnabled": false,
        "key": acct.get("key"),
        "privateKey": acct.get("privateKey"),
        "securityStamp": acct.get("securityStamp"),
        "forcePasswordReset": false,
        "usesKeyConnector": false,
        "avatarColor": null,
        "creationDate": iso(acct.get("created").and_then(Json::as_u64).unwrap_or(0)),
        "organizations": [],
        "providers": [],
        "providerOrganizations": [],
        "_status": 0,
    })
}

async fn put_cipher(api: &Api, access: &Access, id: &str, body: &Json) -> Result {
    let private = api.node.config().private;
    let target = format!("cipher/{id}");
    let created = match api.node.get(&private, &target).await? {
        Some((_, Some(b))) => serde_json::from_slice::<Json>(&b)
            .ok()
            .and_then(|v| v.get("created").and_then(Json::as_u64)),
        _ => None,
    }
    .unwrap_or_else(now_ms);
    let mut v = body.get("cipher").cloned().unwrap_or_else(|| body.clone());
    if let Some(o) = v.as_object_mut() {
        for k in [
            "id",
            "Id",
            "revisionDate",
            "RevisionDate",
            "lastKnownRevisionDate",
            "collectionIds",
        ] {
            o.remove(k);
        }
        o.insert("created".into(), created.into());
        o.insert("title".into(), "Vault item".into());
    }
    save(api, access, &target, &v).await?;
    Ok(json_ok(&cipher_json(id, &v, now_ms(), created)))
}

async fn delete(api: &Api, access: &Access, target: &str) -> Result {
    let private = api.node.config().private;
    sync::write(api, access, &private, Kind::Delete, target, None, "", None).await?;
    Ok(json_ok(&json!({})))
}

async fn soft_delete(api: &Api, access: &Access, id: &str, restore: bool) -> Result {
    let private = api.node.config().private;
    let target = format!("cipher/{id}");
    let Some((_, Some(b))) = api.node.get(&private, &target).await? else {
        return Err(ApiError::not_found());
    };
    let mut v: Json = serde_json::from_slice(&b).unwrap_or_default();
    v["deletedDate"] = if restore {
        Json::Null
    } else {
        iso(now_ms()).into()
    };
    save(api, access, &target, &v).await?;
    let created = v.get("created").and_then(Json::as_u64).unwrap_or(0);
    Ok(json_ok(&cipher_json(id, &v, now_ms(), created)))
}

async fn folder(api: &Api, access: &Access, id: &str, body: &Json) -> Result {
    let name = get_str(body, "name").ok_or_else(|| ApiError::bad("name missing"))?;
    save(
        api,
        access,
        &format!("vault-folder/{id}"),
        &json!({ "name": name, "title": "Vault folder" }),
    )
    .await?;
    Ok(json_ok(
        &json!({ "id": id, "name": name, "revisionDate": iso(now_ms()), "object": "folder" }),
    ))
}

pub async fn handle(api: &Api, req: &Request, segs: &[&str]) -> Result {
    let m = req.method().clone();
    // Signing in and registering need no session.
    match (&m, segs) {
        (&Method::POST, ["identity" | "api", "accounts", "prelogin"]) => {
            return Ok(json_ok(&prelogin(account(api).await.as_ref())));
        }
        (
            &Method::POST,
            [
                "identity",
                "accounts",
                "register",
                "send-verification-email",
            ],
        ) => {
            // No mail here: hand the token straight back, as the apps allow.
            return Ok(json_ok(&Json::String(b64url(&random::<16>()))));
        }
        (
            &Method::POST,
            ["api", "accounts", "register"] | ["identity", "accounts", "register", "finish"],
        ) => {
            return register(api, &body_json(req)?).await;
        }
        (&Method::POST, ["identity", "connect", "token"]) => return connect_token(api, req).await,
        (&Method::GET, ["api", "config"]) => {
            return Ok(json_ok(&json!({
                "version": "2025.1.0",
                "gitHash": "",
                "server": { "name": "homeserver", "url": "https://github.com/" },
                "environment": {},
                "featureStates": {},
                "object": "config",
            })));
        }
        (&Method::GET, ["api", "accounts", "revision-date"]) => {
            return Ok(json_ok(&json!(now_ms())));
        }
        (&Method::GET, ["alive"] | ["api", "alive"]) => return Ok(json_ok(&json!(iso(now_ms())))),
        _ => {}
    }
    let access = api.access(req).await;
    if !is_session(&access) && !access.is_owner() {
        return Err(ApiError::unauthorized());
    }
    let acct = account(api).await.ok_or_else(ApiError::not_found)?;
    match (m, segs) {
        (Method::GET, ["api", "sync"]) => Ok(json_ok(&sync_all(api, &acct).await)),
        (Method::GET, ["api", "accounts", "profile"]) => Ok(json_ok(&profile(&acct))),
        (Method::POST, ["api", "ciphers"] | ["api", "ciphers", "create"]) => {
            put_cipher(api, &access, &uuid(), &body_json(req)?).await
        }
        (Method::PUT | Method::POST, ["api", "ciphers", id]) => {
            put_cipher(api, &access, id, &body_json(req)?).await
        }
        (Method::DELETE, ["api", "ciphers", id])
        | (Method::POST, ["api", "ciphers", id, "delete"]) => {
            delete(api, &access, &format!("cipher/{id}")).await
        }
        (Method::PUT, ["api", "ciphers", id, "delete"]) => {
            soft_delete(api, &access, id, false).await
        }
        (Method::PUT, ["api", "ciphers", id, "restore"]) => {
            soft_delete(api, &access, id, true).await
        }
        (Method::GET, ["api", "folders"]) => {
            let folders: Vec<Json> = listed(api, "vault-folder")
                .await
                .iter()
                .map(|(id, v, t)| json!({ "id": id, "name": get_str(v, "name"), "revisionDate": iso(*t), "object": "folder" }))
                .collect();
            Ok(json_ok(
                &json!({ "data": folders, "object": "list", "continuationToken": null }),
            ))
        }
        (Method::POST, ["api", "folders"]) => folder(api, &access, &uuid(), &body_json(req)?).await,
        (Method::PUT | Method::POST, ["api", "folders", id]) => {
            folder(api, &access, id, &body_json(req)?).await
        }
        (Method::DELETE, ["api", "folders", id])
        | (Method::POST, ["api", "folders", id, "delete"]) => {
            delete(api, &access, &format!("vault-folder/{id}")).await
        }
        _ => Err(ApiError::not_found()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_dates() {
        assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(iso(1_764_028_800_123), "2025-11-25T00:00:00.123Z");
        assert_eq!(iso(951_782_400_000), "2000-02-29T00:00:00.000Z");
    }

    #[test]
    fn passwords() {
        let salt = [7u8; 16];
        let acct =
            json!({ "salt": b64url(&salt), "password": b64url(&hash_password("hunter2", &salt)) });
        assert!(check_password(&acct, "hunter2"));
        assert!(!check_password(&acct, "hunter3"));
    }

    #[test]
    fn case_insensitive_fields() {
        let v = json!({ "MasterPasswordHash": "x" });
        assert_eq!(get_str(&v, "masterPasswordHash"), Some("x"));
    }
}
