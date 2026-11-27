// SPDX-License-Identifier: AGPL-3.0-only

//! Sign-in at the kernel's origin, `fileos.localhost`.
//!
//! | Request                        | What it does                                        |
//! | ------------------------------ | --------------------------------------------------- |
//! | `GET /authorize?…`             | The page where the person lets an app act for them  |
//! | `POST /authorize/challenge`    | A challenge for a passkey, and the passkey IDs      |
//! | `POST /authorize/finish`       | The passkey answer; gives where the browser goes    |
//! | `GET /setup#<code>`            | The page that adds a passkey, with a setup code     |
//! | `POST /setup/challenge`        | A challenge for a new passkey                       |
//! | `POST /setup/finish`           | The new passkey                                     |
//! | `POST /token`                  | Codes and refresh tokens for tokens (with `DPoP`)   |
//! | `POST /revoke`                 | Takes back a sign-in (RFC 7009)                     |
//! | `GET /signin.js`               | The script of the two pages                         |
//!
//! `/token` and `/revoke` take requests from the origins of the apps on
//! this node only (CORS). The pages and their requests are same-origin.

use std::time::{SystemTime, UNIX_EPOCH};

use fileos_core::Error;
use fileos_node::Node;
use hyper::header::{self, HeaderValue};
use hyper::{Method, StatusCode};
use serde::Deserialize;

use super::{Reply, reply};

/// The paths of sign-in.
pub fn is_sign_in(path: &str) -> bool {
    matches!(
        path,
        "/authorize"
            | "/authorize/challenge"
            | "/authorize/finish"
            | "/setup"
            | "/setup/challenge"
            | "/setup/finish"
            | "/token"
            | "/revoke"
            | "/signin.js"
    )
}

/// The paths that apps call from their own origins.
pub fn is_cross_origin(path: &str) -> bool {
    matches!(path, "/token" | "/revoke")
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// The origin of the app with this ID, from the host the request came to.
fn app_origin(id: &str, kernel_host: &str) -> String {
    let port = kernel_host.rsplit_once(':').map_or("", |(_, p)| p);
    if port.is_empty() {
        format!("http://{id}.localhost")
    } else {
        format!("http://{id}.localhost:{port}")
    }
}

/// Says if `origin` is the origin of an app on this node.
pub fn is_app_origin(node: &Node, origin: &str, kernel_host: &str) -> bool {
    node.http_apps()
        .iter()
        .any(|id| app_origin(id, kernel_host) == origin)
}

const PAGE_CSP: &str = "default-src 'none'; script-src 'self'; connect-src 'self'; \
                        style-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'";

fn page(kind: &str) -> Reply {
    let (title, body) = match kind {
        "authorize" => (
            "Sign in",
            "<h1>Sign in</h1>\
             <p><b id=app></b> wants to act for you: to see your name, and to use what you \
             gave it.</p>\
             <p><button id=go>Sign in with a passkey</button> <a id=no href=#>No</a></p>",
        ),
        _ => (
            "Add a passkey",
            "<h1>Add a passkey</h1>\
             <p>A passkey signs you in to the apps on this node. It stays on your device.</p>\
             <p><button id=go>Add a passkey</button></p>",
        ),
    };
    let html = format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
         <title>{title}</title><body data-page={kind}>{body}<p id=error role=alert></p>\
         <script src=/signin.js></script>"
    );
    let mut answer = reply(StatusCode::OK, html);
    let h = answer.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    h.insert(
        "content-security-policy",
        HeaderValue::from_static(PAGE_CSP),
    );
    answer
}

const SCRIPT: &str = r#"// SPDX-License-Identifier: AGPL-3.0-only
"use strict";
const enc = (buf) => btoa(String.fromCharCode(...new Uint8Array(buf)))
  .replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
const dec = (s) => Uint8Array.from(atob(s.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
const post = async (path, data) => {
  const r = await fetch(path, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(data),
  });
  if (!r.ok) throw new Error(await r.text());
  return r.json();
};
const fail = (e) => { document.getElementById("error").textContent = String(e.message || e); };
const go = document.getElementById("go");
if (document.body.dataset.page === "authorize") {
  const q = new URLSearchParams(location.search);
  document.getElementById("app").textContent = q.get("client_id");
  document.getElementById("no").onclick = (ev) => {
    ev.preventDefault();
    const back = new URL(q.get("redirect_uri"));
    back.searchParams.set("error", "access_denied");
    back.searchParams.set("state", q.get("state") || "");
    location.href = back;
  };
  go.onclick = async () => {
    try {
      const { challenge, ids } = await post("/authorize/challenge", {});
      const cred = await navigator.credentials.get({ publicKey: {
        challenge: dec(challenge),
        rpId: "fileos.localhost",
        allowCredentials: ids.map((id) => ({ type: "public-key", id: dec(id) })),
        userVerification: "required",
      } });
      const r = cred.response;
      const { redirect } = await post("/authorize/finish", {
        client_id: q.get("client_id"),
        redirect_uri: q.get("redirect_uri"),
        code_challenge: q.get("code_challenge"),
        state: q.get("state") || "",
        challenge,
        id: enc(cred.rawId),
        client_data: enc(r.clientDataJSON),
        auth_data: enc(r.authenticatorData),
        signature: enc(r.signature),
      });
      location.href = redirect;
    } catch (e) { fail(e); }
  };
} else {
  go.onclick = async () => {
    try {
      const code = location.hash.slice(1);
      const { challenge, user } = await post("/setup/challenge", {});
      const cred = await navigator.credentials.create({ publicKey: {
        challenge: dec(challenge),
        rp: { id: "fileos.localhost", name: "fileOS" },
        user: { id: crypto.getRandomValues(new Uint8Array(16)), name: user, displayName: user },
        pubKeyCredParams: [{ type: "public-key", alg: -7 }, { type: "public-key", alg: -8 }],
        authenticatorSelection: { residentKey: "preferred", userVerification: "required" },
        attestation: "none",
      } });
      await post("/setup/finish", {
        code, challenge,
        client_data: enc(cred.response.clientDataJSON),
        attestation: enc(cred.response.attestationObject),
      });
      go.replaceWith("Done: you can sign in to the apps of this node now.");
    } catch (e) { fail(e); }
  };
}
"#;

fn json(status: StatusCode, value: &serde_json::Value) -> Reply {
    let mut answer = reply(status, value.to_string());
    answer.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    answer
}

/// An OAuth error (RFC 6749, section 5.2). The reason goes to the log, not
/// to the browser.
fn oauth_error(code: &str, why: &Error) -> Reply {
    tracing::info!("sign-in: {code}: {why}");
    json(
        StatusCode::BAD_REQUEST,
        &serde_json::json!({ "error": code }),
    )
}

fn field<'a>(form: &'a [(String, String)], name: &str) -> &'a str {
    form.iter()
        .find(|(k, _)| k == name)
        .map_or("", |(_, v)| v.as_str())
}

fn form(body: &[u8]) -> Vec<(String, String)> {
    String::from_utf8_lossy(body)
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            let d = |s: &str| super::decode(&s.replace('+', " "));
            (d(k), d(v))
        })
        .collect()
}

fn b64(text: &str) -> Result<Vec<u8>, Error> {
    fileos_auth::b64::decode(text).ok_or_else(|| Error("not base64url".into()))
}

#[derive(Deserialize)]
struct Finish {
    client_id: String,
    redirect_uri: String,
    code_challenge: String,
    state: String,
    challenge: String,
    id: String,
    client_data: String,
    auth_data: String,
    signature: String,
}

#[derive(Deserialize)]
struct Setup {
    code: String,
    challenge: String,
    client_data: String,
    attestation: String,
}

/// What a sign-in request needs from the HTTP request.
pub struct Request<'a> {
    pub method: &'a Method,
    pub path: &'a str,
    pub query: &'a std::collections::BTreeMap<String, String>,
    pub host: &'a str,
    pub origin: Option<&'a str>,
    pub dpop: Option<&'a str>,
    pub body: Vec<u8>,
}

/// Answers a sign-in request.
pub fn handle(node: &Node, req: &Request<'_>) -> Reply {
    let mut answer = match handle_inner(node, req) {
        Ok(answer) => answer,
        Err(e) => {
            tracing::info!("sign-in: {e}");
            reply(StatusCode::BAD_REQUEST, format!("{e}\n"))
        }
    };
    // `/token` and `/revoke`: only the origins of the apps on this node.
    if is_cross_origin(req.path)
        && let Some(origin) = req.origin
        && is_app_origin(node, origin, req.host)
        && let Ok(value) = HeaderValue::from_str(origin)
    {
        let h = answer.headers_mut();
        h.insert("access-control-allow-origin", value);
        h.insert("vary", HeaderValue::from_static("origin"));
        h.insert(
            "access-control-allow-headers",
            HeaderValue::from_static("content-type, dpop"),
        );
        h.insert(
            "access-control-allow-methods",
            HeaderValue::from_static("POST"),
        );
    }
    answer
}

fn handle_inner(node: &Node, req: &Request<'_>) -> Result<Reply, Error> {
    let now = now();
    let parse = |body: &[u8]| -> Result<serde_json::Value, Error> {
        serde_json::from_slice(body).map_err(|e| Error(format!("bad JSON: {e}")))
    };
    match (req.method, req.path) {
        (&Method::GET, "/signin.js") => {
            let mut answer = reply(StatusCode::OK, SCRIPT);
            answer.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("text/javascript; charset=utf-8"),
            );
            Ok(answer)
        }
        (&Method::GET, "/authorize") => {
            let q = |k: &str| req.query.get(k).map_or("", String::as_str);
            let client_id = q("client_id");
            let good = q("response_type") == "code"
                && q("code_challenge_method") == "S256"
                && q("code_challenge").len() == 43
                && is_app_origin(node, client_id, req.host)
                && q("redirect_uri").starts_with(&format!("{client_id}/"));
            if !good {
                return Ok(reply(
                    StatusCode::BAD_REQUEST,
                    "not a good sign-in request: it needs response_type=code, a PKCE \
                     challenge (S256), the origin of an app on this node as client_id, and \
                     a redirect_uri on that origin\n",
                ));
            }
            Ok(page("authorize"))
        }
        (&Method::GET, "/setup") => Ok(page("setup")),
        (&Method::POST, "/authorize/challenge") => {
            let (challenge, ids) = node.sign_in(|a| Ok((a.challenge(now), a.passkey_ids())))?;
            if ids.is_empty() {
                return Err(Error(
                    "this node has no passkey yet: run `fileos identity passkey` and open the link"
                        .into(),
                ));
            }
            Ok(json(
                StatusCode::OK,
                &serde_json::json!({ "challenge": challenge, "ids": ids }),
            ))
        }
        (&Method::POST, "/setup/challenge") => {
            let (challenge, user) =
                node.sign_in(|a| Ok((a.challenge(now), a.sub().to_string())))?;
            Ok(json(
                StatusCode::OK,
                &serde_json::json!({ "challenge": challenge, "user": user }),
            ))
        }
        (&Method::POST, "/setup/finish") => {
            let s: Setup = serde_json::from_value(parse(&req.body)?)
                .map_err(|e| Error(format!("bad request: {e}")))?;
            let (client, att) = (b64(&s.client_data)?, b64(&s.attestation)?);
            node.sign_in(|a| a.register(&s.code, &s.challenge, &client, &att, now))?;
            Ok(json(StatusCode::OK, &serde_json::json!({})))
        }
        (&Method::POST, "/authorize/finish") => {
            let f: Finish = serde_json::from_value(parse(&req.body)?)
                .map_err(|e| Error(format!("bad request: {e}")))?;
            if !is_app_origin(node, &f.client_id, req.host) {
                return Err(Error("no such app on this node".into()));
            }
            let request = fileos_auth::Authorize {
                client_id: f.client_id,
                redirect_uri: f.redirect_uri,
                code_challenge: f.code_challenge,
                state: f.state,
            };
            let (id, client, data, sig) = (
                b64(&f.id)?,
                b64(&f.client_data)?,
                b64(&f.auth_data)?,
                b64(&f.signature)?,
            );
            let redirect = node
                .sign_in(|a| a.authorize(&request, &id, &client, &data, &sig, &f.challenge, now))
                .map_err(|e| {
                    tracing::info!("sign-in: {e}");
                    Error("the sign-in did not work".into())
                })?;
            Ok(json(
                StatusCode::OK,
                &serde_json::json!({ "redirect": redirect }),
            ))
        }
        (&Method::OPTIONS, "/token" | "/revoke") => Ok(reply(StatusCode::NO_CONTENT, "")),
        (&Method::POST, "/token") => {
            let form = form(&req.body);
            let client_id = field(&form, "client_id");
            if req.origin != Some(client_id) || !is_app_origin(node, client_id, req.host) {
                return Ok(oauth_error(
                    "invalid_client",
                    &Error("not an app of this node".into()),
                ));
            }
            let proof = req.dpop.unwrap_or_default();
            let url = format!("http://{}/token", req.host);
            let result = match field(&form, "grant_type") {
                "authorization_code" => node.sign_in(|a| {
                    a.token_from_code(
                        field(&form, "code"),
                        field(&form, "code_verifier"),
                        client_id,
                        field(&form, "redirect_uri"),
                        proof,
                        &url,
                        now,
                    )
                }),
                "refresh_token" => node.sign_in(|a| {
                    a.token_from_refresh(field(&form, "refresh_token"), client_id, proof, &url, now)
                }),
                other => {
                    return Ok(oauth_error(
                        "unsupported_grant_type",
                        &Error(format!("grant type {other:?}")),
                    ));
                }
            };
            match result {
                Ok(tokens) => Ok(json(
                    StatusCode::OK,
                    &serde_json::to_value(&tokens).map_err(|e| Error(e.to_string()))?,
                )),
                Err(e) => Ok(oauth_error("invalid_grant", &e)),
            }
        }
        (&Method::POST, "/revoke") => {
            let form = form(&req.body);
            if !req.origin.is_some_and(|o| is_app_origin(node, o, req.host)) {
                return Ok(oauth_error(
                    "invalid_client",
                    &Error("not an app of this node".into()),
                ));
            }
            node.sign_in(|a| {
                a.revoke(field(&form, "token"));
                Ok(())
            })?;
            Ok(reply(StatusCode::OK, ""))
        }
        _ => Ok(reply(StatusCode::METHOD_NOT_ALLOWED, "")),
    }
}

/// Checks the sign-in of a request to an app. Gives back the person, or
/// `None` when the request has no `DPoP` token.
///
/// # Errors
///
/// When the request has a token that is not good.
pub fn person(
    node: &Node,
    authorization: Option<&str>,
    dpop: Option<&str>,
    method: &str,
    host: &str,
    path_and_query: &str,
) -> Result<Option<String>, Error> {
    let Some(token) = authorization.and_then(|a| a.strip_prefix("DPoP ")) else {
        return Ok(None);
    };
    let proof = dpop.ok_or_else(|| Error("a DPoP proof is needed".into()))?;
    let url = format!("http://{host}{path_and_query}");
    let origin = format!("http://{host}");
    node.sign_in(|a| a.check(token, proof, method, &url, &origin, now()))
        .map(Some)
}
