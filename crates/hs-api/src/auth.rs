// SPDX-License-Identifier: AGPL-3.0-only

//! Signing in, and letting other things sign in with this node.
//!
//! - Passkeys (WebAuthn) for the web app: `/auth/passkey/{register,login}/
//!   {start,finish}`. Credentials are `credential/<id>` resources in the
//!   private space; a passkey sign-in creates a session grant.
//! - `OpenID` Connect with PKCE, so other apps can use this node as their
//!   identity provider: `/.well-known/openid-configuration`, `/oauth/*`.
//!   Clients are `oauth-client/<id>` resources in the private space.
//! - Forward auth for reverse proxies: `/auth/verify` answers 200 with
//!   `Remote-User`, or redirects to `/auth/login`.

use std::collections::HashMap;
use std::sync::Mutex;

use hs_core::event::Kind;
use hs_idp::forward::{Decision, Sessions};
use hs_idp::jose::SigningKey;
use hs_idp::{oidc, webauthn};
use hs_node::{Access, Node, now_ms, random};
use hs_space::Role;
use http::{Method, StatusCode, header};
use serde_json::{Value as Json, json};

use crate::util::{
    ApiError, Result, b64url, body_json, cookie, empty, escape, header as hdr, json_ok,
    json_status, origin, param, parse_form, query, respond, set_header, unb64,
};
use crate::{Api, rest};
use hs_net::server::{Peer, Request, Response};

/// The session cookie (holds a bearer token).
pub const COOKIE: &str = "hs_session";
/// The forward-auth cookie (an HMAC-signed subject and expiry).
pub const FORWARD_COOKIE: &str = "hs_forward";
/// How long a passkey session lasts (ms).
pub const SESSION_MS: u64 = 30 * 24 * 3600 * 1000;
const CHALLENGE_MS: u64 = 5 * 60 * 1000;

/// Keys and short-lived state for signing in.
pub struct State {
    challenges: Mutex<HashMap<String, u64>>,
    codes: oidc::Codes,
    signing: Option<SigningKey>,
    sessions: Sessions,
}

fn key_file(node: &Node, name: &str, make: impl FnOnce() -> Option<Vec<u8>>) -> Option<Vec<u8>> {
    let path = node.dir().join("keys").join(name);
    if let Ok(b) = std::fs::read(&path) {
        return Some(b);
    }
    let b = make()?;
    std::fs::write(&path, &b).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Some(b)
}

impl State {
    /// Load (or create) the `OpenID` signing key and forward-auth secret.
    pub fn load(node: &Node) -> State {
        let signing = key_file(node, "oidc", || {
            SigningKey::generate_es256().ok().map(|k| k.to_bytes())
        })
        .and_then(|b| SigningKey::from_bytes(&b).ok());
        let secret = key_file(node, "forward", || Some(random::<32>().to_vec()))
            .and_then(|b| <[u8; 32]>::try_from(b.as_slice()).ok())
            .unwrap_or_else(random::<32>);
        State {
            challenges: Mutex::new(HashMap::new()),
            codes: oidc::Codes::new(),
            signing,
            sessions: Sessions::new(secret),
        }
    }

    fn challenge(&self) -> String {
        let c = b64url(&webauthn::challenge());
        let now = now_ms();
        let mut m = self
            .challenges
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        m.retain(|_, exp| *exp > now);
        m.insert(c.clone(), now + CHALLENGE_MS);
        c
    }

    fn take_challenge(&self, c: &str) -> bool {
        let mut m = self
            .challenges
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        m.remove(c).is_some_and(|exp| exp > now_ms())
    }
}

/// Set the session cookie.
pub fn set_session_cookie(r: &mut Response, token: &str, secure: bool, max_age_s: Option<u64>) {
    let mut c = format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax");
    if secure {
        c.push_str("; Secure");
    }
    if let Some(a) = max_age_s {
        c.push_str(&format!("; Max-Age={a}"));
    }
    if let Ok(v) = http::HeaderValue::from_str(&c) {
        r.headers_mut().append(header::SET_COOKIE, v);
    }
}

/// Clear the session cookie.
pub fn clear_session_cookie(r: &mut Response, secure: bool) {
    set_session_cookie(r, "", secure, Some(0));
}

fn page(title: &str, body: &str) -> Response {
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\"><title>{t}</title>\
         <link rel=stylesheet href=/app.css><main class=narrow><h1>{t}</h1>{body}</main>",
        t = escape(title)
    );
    respond(StatusCode::OK, "text/html; charset=utf-8", html)
}

fn redirect(to: &str) -> Response {
    let mut r = empty(StatusCode::FOUND);
    set_header(&mut r, header::LOCATION, to);
    r
}

fn issuer(api: &Api, req: &Request, peer: Peer) -> String {
    origin(req, peer.tls, api.node.config().url.as_deref())
}

fn rp_id(origin: &str) -> String {
    let host = origin.split("://").nth(1).unwrap_or(origin);
    let host = host.split('/').next().unwrap_or(host);
    match host.rsplit_once(':') {
        Some((h, p))
            if p.chars().all(|c| c.is_ascii_digit()) && !h.ends_with(']')
                || host.starts_with('[') && host.contains("]:") =>
        {
            h.to_string()
        }
        _ => host.to_string(),
    }
}

fn field(v: &Json, k: &str) -> Result<Vec<u8>> {
    v.get(k)
        .and_then(Json::as_str)
        .and_then(unb64)
        .ok_or_else(|| ApiError::bad(format!("{k} missing")))
}

async fn credentials(api: &Api) -> Vec<(String, webauthn::Credential)> {
    let private = api.node.config().private;
    let targets: Vec<String> = api
        .node
        .with_space(&private, |l| {
            l.space
                .resources
                .iter()
                .filter(|(t, r)| t.starts_with("credential/") && r.payload.is_some())
                .map(|(t, _)| t.clone())
                .collect()
        })
        .await
        .unwrap_or_default();
    let mut out = Vec::new();
    for t in targets {
        if let Ok(Some((_, Some(b)))) = api.node.get(&private, &t).await
            && let Ok(v) = serde_json::from_slice::<Json>(&b)
            && let Ok(c) = webauthn::Credential::from_json(&v)
        {
            out.push((t, c));
        }
    }
    out
}

async fn save_credential(api: &Api, target: &str, c: &webauthn::Credential) -> Result<()> {
    let private = api.node.config().private;
    let mut v = c.to_json();
    v["title"] = "Passkey".into();
    let bytes = serde_json::to_vec(&v).unwrap_or_default();
    api.node
        .write(
            &api.node.owner(),
            &private,
            Kind::Put,
            target,
            Some(&bytes),
            "application/json",
            None,
        )
        .await?;
    Ok(())
}

fn challenge_in(client_data: &[u8]) -> Option<String> {
    let v: Json = serde_json::from_slice(client_data).ok()?;
    v.get("challenge")?.as_str().map(String::from)
}

async fn passkey(api: &Api, req: &Request, peer: Peer, access: &Access, step: &[&str]) -> Result {
    let origin = issuer(api, req, peer);
    let rp = rp_id(&origin);
    match step {
        ["register", "start"] => {
            if !access.is_owner() {
                return Err(ApiError::unauthorized());
            }
            let c = api.node.config();
            Ok(json_ok(&json!({
                "challenge": api.auth.challenge(),
                "rp": { "id": rp, "name": "homeserver" },
                "user": { "id": b64url(&c.identity.0), "name": c.name },
            })))
        }
        ["register", "finish"] => {
            if !access.is_owner() {
                return Err(ApiError::unauthorized());
            }
            let b = body_json(req)?;
            let client_data = field(&b, "clientDataJSON")?;
            let challenge =
                challenge_in(&client_data).ok_or_else(|| ApiError::bad("no challenge"))?;
            if !api.auth.take_challenge(&challenge) {
                return Err(ApiError::bad("unknown or expired challenge"));
            }
            let alg = b
                .get("publicKeyAlgorithm")
                .and_then(Json::as_i64)
                .ok_or_else(|| ApiError::bad("publicKeyAlgorithm missing"))?;
            let id = field(&b, "id")?;
            let cred = webauthn::verify_registration(
                &rp,
                &origin,
                &unb64(&challenge).unwrap_or_default(),
                &client_data,
                &field(&b, "authenticatorData")?,
                &field(&b, "publicKey")?,
                alg,
                &id,
            )
            .map_err(|e| ApiError::bad(e.0))?;
            let target = format!("credential/{}", &blake3::hash(&id).to_hex()[..32]);
            save_credential(api, &target, &cred).await?;
            Ok(json_status(StatusCode::CREATED, &json!({ "id": target })))
        }
        ["login", "start"] => {
            let allow: Vec<String> = credentials(api)
                .await
                .iter()
                .map(|(_, c)| b64url(&c.id))
                .collect();
            Ok(json_ok(
                &json!({ "challenge": api.auth.challenge(), "rpId": rp, "allowCredentials": allow }),
            ))
        }
        ["login", "finish"] => {
            let b = body_json(req)?;
            let client_data = field(&b, "clientDataJSON")?;
            let challenge =
                challenge_in(&client_data).ok_or_else(|| ApiError::bad("no challenge"))?;
            if !api.auth.take_challenge(&challenge) {
                return Err(ApiError::bad("unknown or expired challenge"));
            }
            let id = field(&b, "id")?;
            let (target, cred) = credentials(api)
                .await
                .into_iter()
                .find(|(_, c)| c.id == id)
                .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "unknown passkey"))?;
            let count = webauthn::verify_assertion(
                &rp,
                &origin,
                &unb64(&challenge).unwrap_or_default(),
                &cred,
                &client_data,
                &field(&b, "authenticatorData")?,
                &field(&b, "signature")?,
            )
            .map_err(|e| ApiError::new(StatusCode::UNAUTHORIZED, e.0))?;
            if count != cred.sign_count {
                let mut c = cred.clone();
                c.sign_count = count;
                save_credential(api, &target, &c).await?;
            }
            let expires = now_ms() + SESSION_MS;
            let grant = rest::create_grant(
                api,
                &crate::types::NewGrant {
                    expires: Some(expires),
                    session: Some(true),
                    ..crate::types::NewGrant::new(crate::types::Role::Owner, "passkey session")
                },
            )
            .await?;
            let mut r = json_ok(&json!({ "role": "owner", "owner": true, "grant": grant.id }));
            set_session_cookie(&mut r, &grant.token, peer.tls, Some(SESSION_MS / 1000));
            Ok(r)
        }
        _ => Err(ApiError::not_found()),
    }
}

async fn clients(api: &Api) -> Vec<oidc::Client> {
    let private = api.node.config().private;
    let found = api
        .node
        .index
        .list(&private.to_hex(), Some("oauth-client"), 1000, 0)
        .await
        .unwrap_or_default();
    found
        .into_iter()
        .filter_map(|r| r.body)
        .filter_map(|b| oidc::Client::from_json(&b).ok())
        .collect()
}

fn oauth_error(status: StatusCode, e: &str) -> Response {
    let (code, desc) = e.split_once(": ").unwrap_or(("invalid_request", e));
    json_status(status, &json!({ "error": code, "error_description": desc }))
}

async fn oauth(api: &Api, req: &Request, peer: Peer, access: &Access, step: &str) -> Result {
    let iss = issuer(api, req, peer);
    let signing = api
        .auth
        .signing
        .as_ref()
        .ok_or_else(|| ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "no signing key"))?;
    let now = now_ms() / 1000;
    match (req.method().clone(), step) {
        (Method::GET, "jwks") => Ok(json_ok(&hs_idp::jose::jwks(&[signing.public_key()]))),
        (Method::GET, "clients") => {
            if !access.is_owner() {
                return Err(ApiError::unauthorized());
            }
            Ok(json_ok(&Json::Array(
                clients(api)
                    .await
                    .iter()
                    .map(oidc::Client::to_json)
                    .collect(),
            )))
        }
        (Method::POST, "clients") => {
            if !access.is_owner() {
                return Err(ApiError::unauthorized());
            }
            let b = body_json(req)?;
            let id = hs_core::base32::encode(&random::<10>()).to_lowercase();
            let secret = b
                .get("confidential")
                .and_then(Json::as_bool)
                .unwrap_or(false)
                .then(|| b64url(&random::<24>()));
            let client = oidc::Client {
                id: id.clone(),
                name: b
                    .get("name")
                    .and_then(Json::as_str)
                    .unwrap_or("app")
                    .to_string(),
                redirect_uris: b
                    .get("redirect_uris")
                    .and_then(Json::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|u| u.as_str().map(String::from))
                            .collect()
                    })
                    .unwrap_or_default(),
                secret_hash: secret.as_deref().map(oidc::hash_secret),
            };
            if client.redirect_uris.is_empty() {
                return Err(ApiError::bad("redirect_uris missing"));
            }
            let mut v = client.to_json();
            v["title"] = client.name.clone().into();
            let private = api.node.config().private;
            let bytes = serde_json::to_vec(&v).unwrap_or_default();
            api.node
                .write(
                    &api.node.owner(),
                    &private,
                    Kind::Put,
                    &format!("oauth-client/{id}"),
                    Some(&bytes),
                    "application/json",
                    None,
                )
                .await?;
            Ok(json_status(
                StatusCode::CREATED,
                &json!({ "client_id": id, "client_secret": secret, "issuer": iss }),
            ))
        }
        (Method::GET, "authorize") => {
            let p = match oidc::parse_authorize_query(req.uri().query().unwrap_or("")) {
                Ok(p) => p,
                Err(e) => return Ok(page("Cannot sign in", &format!("<p>{}</p>", escape(&e.0)))),
            };
            let Some(client) = clients(api).await.into_iter().find(|c| c.id == p.client_id) else {
                return Ok(page("Cannot sign in", "<p>Unknown app.</p>"));
            };
            if !client.check_redirect(&p.redirect_uri) {
                return Ok(page(
                    "Cannot sign in",
                    "<p>That return address is not registered for this app.</p>",
                ));
            }
            if !access.is_owner() {
                let back = format!("/oauth/authorize?{}", req.uri().query().unwrap_or(""));
                return Ok(page(
                    "Sign in first",
                    &format!(
                        "<p>{} asks to sign you in with this homeserver. Sign in here, then <a href=\"{}\">continue</a>.</p><p><a href=\"/\">Sign in</a></p>",
                        escape(&client.name),
                        escape(&back)
                    ),
                ));
            }
            let code = api.auth.codes.issue(
                oidc::AuthRequest {
                    client_id: p.client_id,
                    redirect_uri: p.redirect_uri.clone(),
                    code_challenge: p.code_challenge,
                    nonce: p.nonce,
                    scope: p.scope,
                    sub: api.node.config().identity.to_id(),
                },
                now,
            );
            Ok(redirect(&oidc::code_redirect(
                &p.redirect_uri,
                &code,
                p.state.as_deref(),
            )))
        }
        (Method::POST, "token") => {
            let f = parse_form(&String::from_utf8_lossy(req.body()));
            let get = |k: &str| param(&f, k).map(String::from);
            if get("grant_type").as_deref() != Some("authorization_code") {
                return Ok(oauth_error(
                    StatusCode::BAD_REQUEST,
                    "unsupported_grant_type: only authorization_code",
                ));
            }
            let (mut client_id, mut secret) = (get("client_id"), get("client_secret"));
            if let Some(basic) =
                hdr(req, header::AUTHORIZATION).and_then(|h| h.strip_prefix("Basic "))
                && let Some(text) = unb64(basic.trim()).and_then(|b| String::from_utf8(b).ok())
                && let Some((id, s)) = text.split_once(':')
            {
                client_id = Some(crate::util::percent_decode(id, true));
                secret = Some(crate::util::percent_decode(s, true));
            }
            let client_id = client_id.unwrap_or_default();
            let Some(client) = clients(api).await.into_iter().find(|c| c.id == client_id) else {
                return Ok(oauth_error(
                    StatusCode::UNAUTHORIZED,
                    "invalid_client: unknown client",
                ));
            };
            if !client.check_secret(secret.as_deref()) {
                return Ok(oauth_error(
                    StatusCode::UNAUTHORIZED,
                    "invalid_client: bad secret",
                ));
            }
            let granted = match api.auth.codes.take(
                &get("code").unwrap_or_default(),
                &client_id,
                &get("redirect_uri").unwrap_or_default(),
                &get("code_verifier").unwrap_or_default(),
                now,
            ) {
                Ok(g) => g,
                Err(e) => return Ok(oauth_error(StatusCode::BAD_REQUEST, &e.0)),
            };
            let name = api.node.config().name;
            let lifetime = 3600;
            let id_token = oidc::id_token(
                signing,
                &iss,
                &granted.sub,
                &client_id,
                granted.nonce.as_deref(),
                Some(&name),
                now,
                lifetime,
            )
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.0))?;
            let access_token = oidc::access_token(
                signing,
                &iss,
                &granted.sub,
                &client_id,
                &granted.scope,
                now,
                lifetime,
            )
            .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.0))?;
            let mut r = json_ok(&json!({
                "access_token": access_token,
                "id_token": id_token,
                "token_type": "Bearer",
                "expires_in": lifetime,
                "scope": granted.scope,
            }));
            set_header(&mut r, header::CACHE_CONTROL, "no-store");
            Ok(r)
        }
        (Method::GET | Method::POST, "userinfo") => {
            let token = hdr(req, header::AUTHORIZATION)
                .and_then(|h| h.strip_prefix("Bearer "))
                .unwrap_or("");
            let claims = oidc::verify_access_token(signing, &iss, token.trim(), now)
                .map_err(|e| ApiError::new(StatusCode::UNAUTHORIZED, e.0))?;
            let c = api.node.config();
            Ok(json_ok(
                &json!({ "sub": claims.get("sub"), "name": c.name }),
            ))
        }
        _ => Err(ApiError::not_found()),
    }
}

fn forward_domain(api: &Api) -> Option<String> {
    api.node
        .config()
        .raw
        .get("forward_domain")
        .and_then(Json::as_str)
        .map(String::from)
}

fn allowed_rd(api: &Api, rd: &str) -> bool {
    let Some(rest) = rd
        .strip_prefix("https://")
        .or_else(|| rd.strip_prefix("http://"))
    else {
        return false;
    };
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.split(':').next().unwrap_or(host);
    match forward_domain(api) {
        Some(d) => host == d || host.ends_with(&format!(".{d}")),
        None => false,
    }
}

fn forward(api: &Api, req: &Request, peer: Peer, access: &Access, step: &str) -> Result {
    let now = now_ms() / 1000;
    match step {
        "verify" => {
            let proto = hdr(req, "x-forwarded-proto").unwrap_or("https");
            let host = hdr(req, "x-forwarded-host").unwrap_or("");
            let uri = hdr(req, "x-forwarded-uri").unwrap_or("/");
            let original = format!("{proto}://{host}{uri}");
            let login = format!("{}/auth/login", issuer(api, req, peer));
            match api
                .auth
                .sessions
                .decide(cookie(req, FORWARD_COOKIE), now, &login, &original)
            {
                Decision::Allow { sub } => {
                    let mut r = empty(StatusCode::OK);
                    set_header(&mut r, "remote-user", &sub);
                    set_header(&mut r, "remote-name", &api.node.config().name);
                    Ok(r)
                }
                Decision::Redirect(to) => {
                    if host.is_empty() {
                        return Err(ApiError::unauthorized());
                    }
                    Ok(redirect(&to))
                }
            }
        }
        "login" => {
            let q = query(req);
            let rd = param(&q, "rd").unwrap_or("/").to_string();
            if !access.is_owner() {
                let back = format!("/auth/login?rd={}", crate::util::percent_encode(&rd, false));
                return Ok(page(
                    "Sign in first",
                    &format!(
                        "<p>Sign in to this homeserver, then <a href=\"{}\">continue</a>.</p><p><a href=\"/\">Sign in</a></p>",
                        escape(&back)
                    ),
                ));
            }
            let Some(domain) = forward_domain(api) else {
                return Ok(page(
                    "Not set up",
                    "<p>Set <code>forward_domain</code> in the node's config first.</p>",
                ));
            };
            if !allowed_rd(api, &rd) {
                return Ok(page(
                    "Cannot continue",
                    "<p>That address is not under the forward-auth domain.</p>",
                ));
            }
            let lifetime = 7 * 24 * 3600;
            let value = api
                .auth
                .sessions
                .issue(&api.node.config().identity.to_id(), now, lifetime);
            let mut r = redirect(&rd);
            let c = format!(
                "{FORWARD_COOKIE}={value}; Domain={domain}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={lifetime}"
            );
            if let Ok(v) = http::HeaderValue::from_str(&c) {
                r.headers_mut().append(header::SET_COOKIE, v);
            }
            Ok(r)
        }
        _ => Err(ApiError::not_found()),
    }
}

pub async fn handle(
    api: &Api,
    req: &Request,
    peer: Peer,
    access: &Access,
    segs: &[&str],
) -> Result {
    match segs {
        [".well-known", "openid-configuration"] => {
            Ok(json_ok(&oidc::discovery(&issuer(api, req, peer))))
        }
        ["auth", "passkey", step @ ..] if req.method() == Method::POST => {
            passkey(api, req, peer, access, step).await
        }
        ["auth", step @ ("verify" | "login")] => forward(api, req, peer, access, step),
        ["oauth", step] => oauth(api, req, peer, access, step).await,
        _ => Err(ApiError::not_found()),
    }
}

/// Whether an access may act as the owner (for pages that need it).
pub fn _is_owner(a: &Access) -> bool {
    matches!(
        a,
        Access::Grant {
            role: Role::Owner,
            ..
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relying_party_ids() {
        assert_eq!(rp_id("https://example.org"), "example.org");
        assert_eq!(rp_id("http://localhost:8080"), "localhost");
        assert_eq!(rp_id("https://[::1]:8443"), "[::1]");
    }
}
