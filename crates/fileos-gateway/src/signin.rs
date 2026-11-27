// SPDX-License-Identifier: AGPL-3.0-only

//! Sign-in: OAuth 2.0 (RFC 6749) for apps in the browser, with PKCE
//! (RFC 7636), as "OAuth 2.0 for Browser-Based Applications" says.
//!
//! The kernel is the authorization server. An app is a public client, named
//! by its origin (`http://<app-id>.localhost:<port>`). The flow:
//!
//! 1. The app sends the browser to `/authorize` with a PKCE challenge.
//! 2. The kernel's page signs the person in and asks if the app may act for
//!    them. Today the person signs in with a one-time code from
//!    `fileos node sign-in-code`; passkeys come next.
//! 3. The kernel sends the browser back to the app with a code, which lives
//!    for one minute and is good once.
//! 4. The app changes the code for an access token at `/token`, with the
//!    PKCE verifier.
//! 5. The app sends the token in `Authorization: Bearer`. The kernel takes
//!    it away and gives the app the person in `FileOS-User`.
//!
//! This module has the state and the checks, with no I/O: the time is given
//! to each call.

use std::collections::BTreeMap;

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::{Digest as _, Sha256};

/// How long a sign-in code from the command line is good, in seconds.
const SIGN_IN_CODE_LIFE: u64 = 10 * 60;
/// How long an authorization code is good, in seconds.
const CODE_LIFE: u64 = 60;
/// How long an access token is good, in seconds.
pub const TOKEN_LIFE: u64 = 60 * 60;
/// The most codes and tokens kept; the oldest go first.
const MOST: usize = 1000;

/// A code given to the browser, for one app.
#[derive(Clone, Debug)]
struct Code {
    client: String,
    redirect: String,
    challenge: String,
    until: u64,
}

/// An access token, for one app.
#[derive(Clone, Debug)]
struct Token {
    app: String,
    until: u64,
}

/// What an app sends to `/authorize`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthorizeRequest {
    pub response_type: String,
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
}

impl AuthorizeRequest {
    /// Reads the request from its query or form fields.
    pub fn from_fields(fields: &BTreeMap<String, String>) -> Self {
        let get = |k: &str| fields.get(k).cloned().unwrap_or_default();
        Self {
            response_type: get("response_type"),
            client_id: get("client_id"),
            redirect_uri: get("redirect_uri"),
            code_challenge: get("code_challenge"),
            code_challenge_method: get("code_challenge_method"),
            state: get("state"),
        }
    }
}

/// The app ID of a client: `http://<app-id>.localhost[:port]` gives
/// `<app-id>`.
pub fn client_app(client_id: &str) -> Option<&str> {
    let host = client_id.strip_prefix("http://")?;
    let host = host.split(':').next()?;
    let app = host.strip_suffix(".localhost")?;
    (!app.is_empty() && app.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')).then_some(app)
}

/// The PKCE `S256` challenge of a verifier.
pub fn challenge_of(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// The state of sign-in on one node.
#[derive(Debug, Default)]
pub struct SignIn {
    /// One-time sign-in codes from the command line, with when they end.
    sign_in_codes: BTreeMap<String, u64>,
    codes: BTreeMap<String, Code>,
    tokens: BTreeMap<String, Token>,
}

/// Why a request was not taken, as an OAuth error code and a text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused {
    pub error: &'static str,
    pub text: String,
}

fn refused(error: &'static str, text: impl Into<String>) -> Refused {
    Refused {
        error,
        text: text.into(),
    }
}

impl SignIn {
    /// Makes a one-time sign-in code for the person, from random bytes.
    pub fn new_sign_in_code(&mut self, random: [u8; 5], now: u64) -> String {
        self.drop_old(now);
        let code: String = random
            .iter()
            .map(|b| char::from(b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789"[usize::from(*b) % 32]))
            .collect();
        self.sign_in_codes
            .insert(code.clone(), now + SIGN_IN_CODE_LIFE);
        code
    }

    /// Checks a request to `/authorize` before the page is shown. Gives the
    /// app ID.
    ///
    /// # Errors
    ///
    /// When the request is not one this kernel takes. The page shows the
    /// error; it does not send the browser back, since the redirect URI may
    /// be false.
    pub fn check_request(&self, r: &AuthorizeRequest) -> Result<String, Refused> {
        if r.response_type != "code" {
            return Err(refused(
                "unsupported_response_type",
                "only response_type=code",
            ));
        }
        let app = client_app(&r.client_id)
            .ok_or_else(|| refused("invalid_client", "client_id is not the origin of an app"))?;
        if !r.redirect_uri.starts_with(&format!("{}/", r.client_id)) {
            return Err(refused(
                "invalid_request",
                "redirect_uri must be on the origin of the app",
            ));
        }
        if r.code_challenge_method != "S256" || r.code_challenge.len() != 43 {
            return Err(refused(
                "invalid_request",
                "a PKCE code_challenge with S256 is needed",
            ));
        }
        Ok(app.to_string())
    }

    /// The person signed in with `sign_in_code` and said yes. Gives the URL
    /// to send the browser back to, with a new code.
    ///
    /// # Errors
    ///
    /// When the request is not good, or the sign-in code is not good.
    pub fn approve(
        &mut self,
        r: &AuthorizeRequest,
        sign_in_code: &str,
        random: [u8; 32],
        now: u64,
    ) -> Result<String, Refused> {
        self.check_request(r)?;
        self.drop_old(now);
        let code = sign_in_code.trim().to_ascii_uppercase();
        if self.sign_in_codes.remove(&code).is_none() {
            return Err(refused("access_denied", "the sign-in code is not good"));
        }
        let code = URL_SAFE_NO_PAD.encode(random);
        self.codes.insert(
            code.clone(),
            Code {
                client: r.client_id.clone(),
                redirect: r.redirect_uri.clone(),
                challenge: r.code_challenge.clone(),
                until: now + CODE_LIFE,
            },
        );
        let join = if r.redirect_uri.contains('?') {
            '&'
        } else {
            '?'
        };
        let mut url = format!("{}{join}code={code}", r.redirect_uri);
        if !r.state.is_empty() {
            url.push_str("&state=");
            url.push_str(
                &percent_encoding::utf8_percent_encode(
                    &r.state,
                    percent_encoding::NON_ALPHANUMERIC,
                )
                .to_string(),
            );
        }
        Ok(url)
    }

    /// The URL that says no to the app, for when the person says no.
    pub fn deny_url(r: &AuthorizeRequest) -> String {
        let join = if r.redirect_uri.contains('?') {
            '&'
        } else {
            '?'
        };
        format!("{}{join}error=access_denied", r.redirect_uri)
    }

    /// Changes a code for an access token (`grant_type=authorization_code`).
    /// Gives the token.
    ///
    /// # Errors
    ///
    /// When the code is not good, or not for this client and redirect URI,
    /// or the verifier does not match the challenge.
    pub fn token(
        &mut self,
        fields: &BTreeMap<String, String>,
        random: [u8; 32],
        now: u64,
    ) -> Result<String, Refused> {
        let get = |k: &str| fields.get(k).map(String::as_str).unwrap_or_default();
        if get("grant_type") != "authorization_code" {
            return Err(refused(
                "unsupported_grant_type",
                "only grant_type=authorization_code",
            ));
        }
        self.drop_old(now);
        // A code is good once, also when the rest is wrong.
        let code = self
            .codes
            .remove(get("code"))
            .ok_or_else(|| refused("invalid_grant", "the code is not good"))?;
        if code.client != get("client_id") || code.redirect != get("redirect_uri") {
            return Err(refused(
                "invalid_grant",
                "the code is for another client or redirect_uri",
            ));
        }
        if challenge_of(get("code_verifier")) != code.challenge {
            return Err(refused(
                "invalid_grant",
                "the code_verifier does not match the code_challenge",
            ));
        }
        let app = client_app(&code.client)
            .ok_or_else(|| refused("invalid_client", "not an app"))?
            .to_string();
        let token = URL_SAFE_NO_PAD.encode(random);
        self.tokens.insert(
            token.clone(),
            Token {
                app,
                until: now + TOKEN_LIFE,
            },
        );
        Ok(token)
    }

    /// Says if `token` is a good access token for the app `app`.
    pub fn check(&self, token: &str, app: &str, now: u64) -> bool {
        self.tokens
            .get(token)
            .is_some_and(|t| t.app == app && t.until > now)
    }

    /// Takes a token back (RFC 7009). Says nothing about whether it was
    /// there, as the RFC says.
    pub fn revoke(&mut self, token: &str) {
        self.tokens.remove(token);
    }

    /// Drops what has ended, and the oldest when there are too many.
    fn drop_old(&mut self, now: u64) {
        self.sign_in_codes.retain(|_, until| *until > now);
        self.codes.retain(|_, c| c.until > now);
        self.tokens.retain(|_, t| t.until > now);
        while self.tokens.len() > MOST {
            let oldest = self
                .tokens
                .iter()
                .min_by_key(|(_, t)| t.until)
                .map(|(k, _)| k.clone());
            if let Some(k) = oldest {
                self.tokens.remove(&k);
            }
        }
        while self.codes.len() > MOST {
            let oldest = self.codes.keys().next().cloned();
            if let Some(k) = oldest {
                self.codes.remove(&k);
            }
        }
    }
}

/// Puts text into HTML safely.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// The page that signs the person in and asks about the app.
pub fn page(r: &AuthorizeRequest, app: &str, error: Option<&str>) -> String {
    let hidden: String = [
        ("response_type", &r.response_type),
        ("client_id", &r.client_id),
        ("redirect_uri", &r.redirect_uri),
        ("code_challenge", &r.code_challenge),
        ("code_challenge_method", &r.code_challenge_method),
        ("state", &r.state),
    ]
    .iter()
    .map(|(k, v)| format!("<input type=hidden name={k} value=\"{}\">", escape(v)))
    .collect();
    let error = error.map_or_else(String::new, |e| format!("<p role=alert>{}</p>", escape(e)));
    format!(
        "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
         <title>Sign in</title>\
         <h1>Sign in to fileOS</h1>\
         <p>The app <b>{app}</b> wants to act for you on this node.</p>{error}\
         <form method=post action=/authorize>{hidden}\
         <p><label>Sign-in code (from <code>fileos node sign-in-code</code>)<br>\
         <input name=sign_in_code autocomplete=one-time-code required autofocus></label></p>\
         <p><button name=answer value=yes>Let it act for me</button> \
         <button name=answer value=no formnovalidate>No</button></p></form>",
        app = escape(app)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AuthorizeRequest {
        AuthorizeRequest {
            response_type: "code".into(),
            client_id: "http://photos-abc.localhost:7341".into(),
            redirect_uri: "http://photos-abc.localhost:7341/back".into(),
            code_challenge: challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            code_challenge_method: "S256".into(),
            state: "xyz".into(),
        }
    }

    fn fields(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect()
    }

    #[test]
    fn the_pkce_example_of_rfc_7636() {
        assert_eq!(
            challenge_of("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn an_app_signs_in_and_gets_a_token() {
        let mut s = SignIn::default();
        let r = request();
        assert_eq!(s.check_request(&r).unwrap(), "photos-abc");
        let code = s.new_sign_in_code([1, 2, 3, 4, 5], 100);
        s.approve(&r, "WRONG", [0; 32], 100).unwrap_err();
        let url = s.approve(&r, &code.to_lowercase(), [7; 32], 101).unwrap();
        assert!(url.starts_with("http://photos-abc.localhost:7341/back?code="));
        assert!(url.ends_with("&state=xyz"));
        // The sign-in code is good once.
        s.approve(&r, &code, [8; 32], 102).unwrap_err();
        let code = url
            .split("code=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        let mut form = fields(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("client_id", "http://photos-abc.localhost:7341"),
            ("redirect_uri", "http://photos-abc.localhost:7341/back"),
            (
                "code_verifier",
                "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            ),
        ]);
        let token = s.token(&form, [9; 32], 110).unwrap();
        assert!(s.check(&token, "photos-abc", 120));
        assert!(!s.check(&token, "chat-abc", 120), "a token is for one app");
        assert!(!s.check(&token, "photos-abc", 110 + TOKEN_LIFE));
        // The code is good once.
        s.token(&form, [10; 32], 111).unwrap_err();
        s.revoke(&token);
        assert!(!s.check(&token, "photos-abc", 120));
        // A wrong verifier gets nothing, and spends the code.
        let code2 = s.new_sign_in_code([5; 5], 200);
        let url = s.approve(&r, &code2, [11; 32], 200).unwrap();
        let code = url
            .split("code=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap();
        form.insert("code".into(), code.into());
        form.insert("code_verifier".into(), "not it".into());
        s.token(&form, [12; 32], 201).unwrap_err();
    }

    #[test]
    fn bad_requests_are_refused() {
        let s = SignIn::default();
        let mut r = request();
        r.redirect_uri = "http://evil.example/back".into();
        assert_eq!(s.check_request(&r).unwrap_err().error, "invalid_request");
        let mut r = request();
        r.client_id = "http://evil.example".into();
        assert_eq!(s.check_request(&r).unwrap_err().error, "invalid_client");
        let mut r = request();
        r.code_challenge_method = "plain".into();
        s.check_request(&r).unwrap_err();
        assert_eq!(client_app("http://chat-1.localhost"), Some("chat-1"));
        assert_eq!(client_app("http://a.b.localhost"), None);
    }

    #[test]
    fn codes_end() {
        let mut s = SignIn::default();
        let r = request();
        let code = s.new_sign_in_code([1; 5], 0);
        s.approve(&r, &code, [1; 32], SIGN_IN_CODE_LIFE + 1)
            .unwrap_err();
        assert!(page(&r, "<b>", None).contains("&lt;b&gt;"));
    }
}
