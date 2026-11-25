// SPDX-License-Identifier: AGPL-3.0-only

//! `OpenID` Connect provider logic: discovery, clients, authorization codes
//! with PKCE, and ID and access tokens.

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{Value, json};

use crate::jose::{SigningKey, b64e};
use crate::{Error, Result, err, hex, percent_decode, percent_encode, sha256};

/// How long an authorization code stays valid, in seconds.
pub const CODE_LIFETIME: u64 = 60;

/// The discovery document served at `/.well-known/openid-configuration`.
pub fn discovery(issuer: &str) -> Value {
    json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/oauth/authorize"),
        "token_endpoint": format!("{issuer}/oauth/token"),
        "jwks_uri": format!("{issuer}/oauth/jwks"),
        "userinfo_endpoint": format!("{issuer}/oauth/userinfo"),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code"],
        "code_challenge_methods_supported": ["S256"],
        "id_token_signing_alg_values_supported": ["ES256", "EdDSA"],
        "scopes_supported": ["openid", "profile", "email"],
        "subject_types_supported": ["public"],
        "token_endpoint_auth_methods_supported": ["none", "client_secret_post", "client_secret_basic"],
        "claims_supported": ["iss", "sub", "aud", "exp", "iat", "nonce", "name"],
    })
}

/// Check a PKCE S256 code verifier against its challenge (RFC 7636).
pub fn pkce_s256_ok(verifier: &str, challenge: &str) -> bool {
    let well_formed = (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'));
    well_formed
        && hs_core::ct_eq(
            b64e(&sha256(verifier.as_bytes())).as_bytes(),
            challenge.as_bytes(),
        )
}

/// Hash a client secret for storage: lowercase hex SHA-256.
pub fn hash_secret(secret: &str) -> String {
    hex(&sha256(secret.as_bytes()))
}

/// A registered relying party.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    /// The client ID.
    pub id: String,
    /// A display name for consent screens.
    pub name: String,
    /// Allowed redirect URIs, matched exactly.
    pub redirect_uris: Vec<String>,
    /// SHA-256 hex of the secret, or `None` for a public client.
    pub secret_hash: Option<String>,
}

impl Client {
    /// Serialize as JSON.
    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "redirect_uris": self.redirect_uris,
            "secret_hash": self.secret_hash,
        })
    }

    /// Parse the output of [`Client::to_json`].
    ///
    /// # Errors
    ///
    /// Returns an error if required client fields are missing or invalid.
    pub fn from_json(v: &Value) -> Result<Client> {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or(Error(format!("missing {k}")))
        };
        let redirect_uris = v
            .get("redirect_uris")
            .and_then(Value::as_array)
            .ok_or(Error("missing redirect_uris".into()))?
            .iter()
            .map(|u| {
                u.as_str()
                    .map(str::to_owned)
                    .ok_or(Error("bad redirect URI".into()))
            })
            .collect::<Result<Vec<_>>>()?;
        let secret_hash = match v.get("secret_hash") {
            None | Some(Value::Null) => None,
            Some(Value::String(h)) => Some(h.clone()),
            Some(_) => return err("bad secret_hash"),
        };
        Ok(Client {
            id: s("id")?,
            name: s("name")?,
            redirect_uris,
            secret_hash,
        })
    }

    /// Whether `uri` is one of the registered redirect URIs.
    pub fn check_redirect(&self, uri: &str) -> bool {
        self.redirect_uris.iter().any(|u| u == uri)
    }

    /// Whether the presented secret authenticates this client. Public clients
    /// (no stored hash) need no secret; confidential clients must present one.
    pub fn check_secret(&self, secret: Option<&str>) -> bool {
        match (&self.secret_hash, secret) {
            (None, _) => true,
            (Some(h), Some(s)) => hs_core::ct_eq(hash_secret(s).as_bytes(), h.as_bytes()),
            (Some(_), None) => false,
        }
    }
}

/// An approved authorization request, bound to an authorization code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthRequest {
    /// The client the code was issued to.
    pub client_id: String,
    /// The redirect URI used in the request.
    pub redirect_uri: String,
    /// The PKCE S256 challenge.
    pub code_challenge: String,
    /// The OIDC nonce, if the client sent one.
    pub nonce: Option<String>,
    /// The granted scope, space separated.
    pub scope: String,
    /// The signed-in subject.
    pub sub: String,
}

/// An in-memory store of single-use authorization codes.
#[derive(Debug, Default)]
pub struct Codes {
    codes: Mutex<HashMap<String, (AuthRequest, u64)>>,
}

impl Codes {
    /// An empty store.
    pub fn new() -> Codes {
        Codes::default()
    }

    /// Issue a code for an approved request. Expired codes are pruned.
    pub fn issue(&self, req: AuthRequest, now: u64) -> String {
        let code = b64e(&crate::random::<32>());
        let mut codes = self
            .codes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        codes.retain(|_, (_, exp)| *exp > now);
        codes.insert(code.clone(), (req, now + CODE_LIFETIME));
        code
    }

    /// Redeem a code. It is consumed even if a check fails.
    ///
    /// # Errors
    ///
    /// Returns an error for a missing, expired or mismatched code, or an invalid PKCE verifier.
    pub fn take(
        &self,
        code: &str,
        client_id: &str,
        redirect_uri: &str,
        verifier: &str,
        now: u64,
    ) -> Result<AuthRequest> {
        let entry = self
            .codes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(code);
        let Some((req, exp)) = entry else {
            return err("invalid_grant: unknown code");
        };
        if exp <= now {
            return err("invalid_grant: code expired");
        }
        if req.client_id != client_id || req.redirect_uri != redirect_uri {
            return err("invalid_grant: client or redirect URI mismatch");
        }
        if !pkce_s256_ok(verifier, &req.code_challenge) {
            return err("invalid_grant: PKCE verification failed");
        }
        Ok(req)
    }
}

/// Create a signed ID token.
///
/// # Errors
///
/// Returns an error if the token cannot be serialized or signed.
#[expect(
    clippy::too_many_arguments,
    reason = "issuer, audience and claims are distinct signing inputs"
)]
pub fn id_token(
    key: &SigningKey,
    issuer: &str,
    sub: &str,
    aud: &str,
    nonce: Option<&str>,
    name: Option<&str>,
    now: u64,
    lifetime: u64,
) -> Result<String> {
    let mut claims = json!({
        "iss": issuer,
        "sub": sub,
        "aud": aud,
        "iat": now,
        "exp": now + lifetime,
    });
    if let Some(n) = nonce {
        claims["nonce"] = n.into();
    }
    if let Some(n) = name {
        claims["name"] = n.into();
    }
    key.sign(&Value::Null, &claims)
}

/// Create a signed JWT access token (RFC 9068, `typ: at+jwt`).
///
/// # Errors
///
/// Returns an error if the token cannot be serialized or signed.
pub fn access_token(
    key: &SigningKey,
    issuer: &str,
    sub: &str,
    client_id: &str,
    scope: &str,
    now: u64,
    lifetime: u64,
) -> Result<String> {
    let claims = json!({
        "iss": issuer,
        "sub": sub,
        "aud": issuer,
        "client_id": client_id,
        "scope": scope,
        "iat": now,
        "exp": now + lifetime,
        "jti": b64e(&crate::random::<16>()),
    });
    key.sign(&json!({"typ": "at+jwt"}), &claims)
}

/// Verify an access token from [`access_token`] and return its claims.
/// ID tokens are rejected by their `typ`.
///
/// # Errors
///
/// Returns an error for an invalid signature, JWT type, issuer or expiration.
pub fn verify_access_token(key: &SigningKey, issuer: &str, token: &str, now: u64) -> Result<Value> {
    let (header, claims) = crate::jose::verify_full(token, &key.public_key())?;
    if header.get("typ").and_then(Value::as_str) != Some("at+jwt") {
        return err("not an access token");
    }
    if claims.get("iss").and_then(Value::as_str) != Some(issuer)
        || claims.get("aud").and_then(Value::as_str) != Some(issuer)
    {
        return err("wrong issuer or audience");
    }
    match claims.get("exp").and_then(Value::as_u64) {
        Some(exp) if exp > now => Ok(claims),
        _ => err("access token expired"),
    }
}

/// Validated parameters of an authorization request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizeParams {
    /// The client ID.
    pub client_id: String,
    /// The redirect URI; check it with [`Client::check_redirect`].
    pub redirect_uri: String,
    /// The requested scope, space separated; contains `openid`.
    pub scope: String,
    /// Opaque client state, echoed back.
    pub state: Option<String>,
    /// The OIDC nonce.
    pub nonce: Option<String>,
    /// The PKCE S256 challenge.
    pub code_challenge: String,
}

/// Parse and validate the query string of an authorization request.
///
/// # Errors
///
/// Returns an error when a required parameter is malformed or the request is unsupported.
pub fn parse_authorize_query(query: &str) -> Result<AuthorizeParams> {
    let mut params: HashMap<String, String> = HashMap::new();
    for pair in query
        .trim_start_matches('?')
        .split('&')
        .filter(|p| !p.is_empty())
    {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        let k = percent_decode(k)?;
        if params.insert(k.clone(), percent_decode(v)?).is_some() {
            return err(format!("invalid_request: repeated parameter {k}"));
        }
    }
    let mut take = |k: &str| params.remove(k).filter(|v| !v.is_empty());
    let need = |v: Option<String>, k: &str| v.ok_or(Error(format!("invalid_request: missing {k}")));
    if take("response_type").as_deref() != Some("code") {
        return err("unsupported_response_type: only code is supported");
    }
    if take("code_challenge_method").as_deref() != Some("S256") {
        return err("invalid_request: code_challenge_method must be S256");
    }
    let scope = need(take("scope"), "scope")?;
    if !scope.split(' ').any(|s| s == "openid") {
        return err("invalid_scope: openid is required");
    }
    Ok(AuthorizeParams {
        client_id: need(take("client_id"), "client_id")?,
        redirect_uri: need(take("redirect_uri"), "redirect_uri")?,
        code_challenge: need(take("code_challenge"), "code_challenge")?,
        state: take("state"),
        nonce: take("nonce"),
        scope,
    })
}

/// The redirect URL that returns a code (and state) to the client.
pub fn code_redirect(redirect_uri: &str, code: &str, state: Option<&str>) -> String {
    let sep = if redirect_uri.contains('?') { '&' } else { '?' };
    let mut url = format!("{redirect_uri}{sep}code={}", percent_encode(code));
    if let Some(s) = state {
        url.push_str(&format!("&state={}", percent_encode(s)));
    }
    url
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISS: &str = "https://id.example";
    // RFC 7636 appendix B.
    const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
    const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

    #[test]
    fn pkce() {
        assert!(pkce_s256_ok(VERIFIER, CHALLENGE));
        assert!(!pkce_s256_ok(
            VERIFIER,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cN"
        ));
        assert!(!pkce_s256_ok("short", CHALLENGE));
    }

    #[test]
    fn discovery_doc() {
        let d = discovery(ISS);
        assert_eq!(d["token_endpoint"], "https://id.example/oauth/token");
        assert_eq!(d["code_challenge_methods_supported"][0], "S256");
    }

    #[test]
    fn clients() {
        let c = Client {
            id: "app".into(),
            name: "App".into(),
            redirect_uris: vec!["https://app.example/cb".into()],
            secret_hash: Some(hash_secret("s3cret")),
        };
        assert_eq!(Client::from_json(&c.to_json()).unwrap(), c);
        assert!(c.check_redirect("https://app.example/cb"));
        assert!(!c.check_redirect("https://app.example/cb/"));
        assert!(c.check_secret(Some("s3cret")));
        assert!(!c.check_secret(Some("wrong")));
        assert!(!c.check_secret(None));
        let public = Client {
            secret_hash: None,
            ..c
        };
        assert!(public.check_secret(None));
        assert!(Client::from_json(&json!({"id": "x"})).is_err());
    }

    fn request() -> AuthRequest {
        AuthRequest {
            client_id: "app".into(),
            redirect_uri: "https://app.example/cb".into(),
            code_challenge: CHALLENGE.into(),
            nonce: Some("n".into()),
            scope: "openid".into(),
            sub: "alice".into(),
        }
    }

    #[test]
    fn codes() {
        let codes = Codes::new();
        let cb = "https://app.example/cb";
        let c = codes.issue(request(), 100);
        assert_eq!(codes.take(&c, "app", cb, VERIFIER, 130).unwrap(), request());
        assert!(
            codes.take(&c, "app", cb, VERIFIER, 130).is_err(),
            "single use"
        );

        let c = codes.issue(request(), 100);
        assert!(codes.take(&c, "app", cb, VERIFIER, 160).is_err(), "expired");
        let c = codes.issue(request(), 100);
        assert!(codes.take(&c, "other", cb, VERIFIER, 101).is_err());
        let c = codes.issue(request(), 100);
        assert!(
            codes
                .take(&c, "app", "https://evil/cb", VERIFIER, 101)
                .is_err()
        );
        let c = codes.issue(request(), 100);
        assert!(codes.take(&c, "app", cb, &"a".repeat(43), 101).is_err());
        assert!(
            codes.take(&c, "app", cb, VERIFIER, 101).is_err(),
            "consumed by failure"
        );
        assert!(codes.take("nope", "app", cb, VERIFIER, 101).is_err());
    }

    #[test]
    fn tokens() {
        for key in [
            SigningKey::generate_es256().unwrap(),
            SigningKey::generate_eddsa().unwrap(),
        ] {
            let id = id_token(
                &key,
                ISS,
                "alice",
                "app",
                Some("n"),
                Some("Alice"),
                100,
                300,
            )
            .unwrap();
            let claims = crate::jose::verify(&id, &key.public_key()).unwrap();
            assert_eq!(claims["nonce"], "n");
            assert_eq!(claims["exp"], 400);
            assert!(
                verify_access_token(&key, ISS, &id, 100).is_err(),
                "ID token is not an access token"
            );

            let at = access_token(&key, ISS, "alice", "app", "openid", 100, 300).unwrap();
            let claims = verify_access_token(&key, ISS, &at, 399).unwrap();
            assert_eq!(claims["sub"], "alice");
            assert!(verify_access_token(&key, ISS, &at, 400).is_err());
            assert!(verify_access_token(&key, "https://other", &at, 100).is_err());
        }
    }

    #[test]
    fn authorize_query() {
        let q = "response_type=code&client_id=app&redirect_uri=https%3A%2F%2Fapp.example%2Fcb\
                 &scope=openid+profile&state=xyz&code_challenge=abc&code_challenge_method=S256";
        let p = parse_authorize_query(q).unwrap();
        assert_eq!(p.redirect_uri, "https://app.example/cb");
        assert_eq!(p.scope, "openid profile");
        assert_eq!(p.state.as_deref(), Some("xyz"));
        assert_eq!(p.nonce, None);

        assert!(parse_authorize_query(&q.replace("S256", "plain")).is_err());
        assert!(
            parse_authorize_query(&q.replace("response_type=code", "response_type=token")).is_err()
        );
        assert!(parse_authorize_query(&q.replace("openid+", "")).is_err());
        assert!(parse_authorize_query(&q.replace("&code_challenge=abc", "")).is_err());
        assert!(parse_authorize_query(&format!("{q}&state=again")).is_err());
        assert!(parse_authorize_query(&format!("{q}&x=%zz")).is_err());
        assert!(parse_authorize_query("").is_err());
    }

    #[test]
    fn redirect() {
        assert_eq!(
            code_redirect("https://a/cb", "c", Some("s t")),
            "https://a/cb?code=c&state=s%20t"
        );
        assert_eq!(
            code_redirect("https://a/cb?x=1", "c", None),
            "https://a/cb?x=1&code=c"
        );
    }
}
