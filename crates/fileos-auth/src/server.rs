// SPDX-License-Identifier: AGPL-3.0-only

//! The authorization server of a node, with no I/O.
//!
//! The flow is OAuth 2.0 for browser-based apps: the authorization code
//! grant with PKCE, a passkey to sign the person in, and `DPoP`-bound access
//! tokens and refresh tokens. Refresh tokens change at each use; a refresh
//! token that is used a second time takes back all the tokens that came
//! from the same sign-in, because one of the two users stole it.

use std::collections::BTreeMap;

use fileos_core::{Hash, SecretKey};
use serde::{Deserialize, Serialize};

use crate::webauthn::{self, Passkey};
use crate::{Error, b64, dpop, pkce, sha256, token};

/// How long an access token lasts, in seconds.
pub const ACCESS_TTL: u64 = 10 * 60;
/// How long a refresh token lasts, in seconds, if it is not used.
pub const REFRESH_TTL: u64 = 30 * 24 * 60 * 60;
/// How long an authorization code lasts.
const CODE_TTL: u64 = 60;
/// How long a challenge for a passkey lasts.
const CHALLENGE_TTL: u64 = 5 * 60;
/// How long a setup code for a new passkey lasts.
const SETUP_TTL: u64 = 10 * 60;

/// What the node keeps on disk.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthState {
    pub passkeys: Vec<Passkey>,
    refresh: Vec<Refresh>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Refresh {
    /// The SHA-256 of the token; the token itself is not kept.
    hash: Hash,
    sub: String,
    aud: String,
    jkt: String,
    /// All the refresh tokens of one sign-in.
    family: Hash,
    exp: u64,
    /// Used already: a second use takes back the family.
    used: bool,
}

/// A request to `/authorize`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Authorize {
    /// The origin of the app, such as `http://photos.localhost:7341`.
    pub client_id: String,
    /// Where the browser goes back to, on the app's origin.
    pub redirect_uri: String,
    /// The PKCE challenge (`S256`).
    pub code_challenge: String,
    /// Given back to the app as it is.
    pub state: String,
}

/// The answer of `/token`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Tokens {
    pub access_token: String,
    pub token_type: &'static str,
    pub expires_in: u64,
    pub refresh_token: String,
}

#[derive(Clone, Debug)]
struct Code {
    aud: String,
    redirect: String,
    challenge: String,
    exp: u64,
}

/// The authorization server of one node.
pub struct Auth {
    state: AuthState,
    key: SecretKey,
    /// The person the node signs in: a DID.
    sub: String,
    origin: String,
    rp_id: String,
    seed: [u8; 32],
    counter: u64,
    challenges: BTreeMap<String, u64>,
    setup: BTreeMap<String, u64>,
    codes: BTreeMap<String, Code>,
    /// The IDs of `DPoP` proofs taken, with the time they may be dropped.
    jtis: BTreeMap<String, u64>,
}

impl std::fmt::Debug for Auth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Auth({}, {} passkeys)",
            self.sub,
            self.state.passkeys.len()
        )
    }
}

impl Auth {
    /// `origin` is the kernel's origin, such as `http://fileos.localhost:7341`;
    /// `rp_id` its host, `fileos.localhost`. `seed` must be random.
    pub fn new(
        key: SecretKey,
        sub: String,
        origin: String,
        rp_id: String,
        state: AuthState,
        seed: [u8; 32],
    ) -> Self {
        Self {
            state,
            key,
            sub,
            origin,
            rp_id,
            seed,
            counter: 0,
            challenges: BTreeMap::new(),
            setup: BTreeMap::new(),
            codes: BTreeMap::new(),
            jtis: BTreeMap::new(),
        }
    }

    /// What to keep on disk after a change.
    pub fn state(&self) -> &AuthState {
        &self.state
    }

    /// The person this node signs in.
    pub fn sub(&self) -> &str {
        &self.sub
    }

    /// The person changes, for example when the node joins an identity.
    pub fn set_sub(&mut self, sub: String) {
        self.sub = sub;
    }

    fn random(&mut self) -> String {
        self.counter += 1;
        let mut input = self.seed.to_vec();
        input.extend(self.counter.to_le_bytes());
        b64::encode(&sha256(&input))
    }

    /// Drops what has ended, so that nothing grows without a limit.
    fn prune(&mut self, now: u64) {
        self.challenges.retain(|_, exp| *exp > now);
        self.setup.retain(|_, exp| *exp > now);
        self.codes.retain(|_, c| c.exp > now);
        self.jtis.retain(|_, exp| *exp > now);
        self.state.refresh.retain(|r| r.exp > now);
    }

    /// A one-time code to register a passkey, for the owner of the node.
    pub fn setup_code(&mut self, now: u64) -> String {
        self.prune(now);
        let code = self.random();
        self.setup.insert(code.clone(), now + SETUP_TTL);
        code
    }

    /// A challenge for a passkey, which the page gives the browser. Each
    /// is good for one answer.
    pub fn challenge(&mut self, now: u64) -> String {
        self.prune(now);
        let c = self.random();
        self.challenges.insert(c.clone(), now + CHALLENGE_TTL);
        c
    }

    fn take_challenge(&mut self, challenge: &str, now: u64) -> Result<Vec<u8>, Error> {
        self.prune(now);
        self.challenges
            .remove(challenge)
            .and_then(|_| b64::decode(challenge))
            .ok_or_else(|| Error::invalid("the challenge is not good, or has ended".into()))
    }

    /// Registers a passkey, with a setup code and a challenge.
    ///
    /// # Errors
    ///
    /// When the setup code or the answer is not good.
    pub fn register(
        &mut self,
        setup_code: &str,
        challenge: &str,
        client_data: &[u8],
        attestation: &[u8],
        now: u64,
    ) -> Result<(), Error> {
        self.prune(now);
        if self.setup.remove(setup_code).is_none() {
            return Err(Error::invalid(
                "the setup code is not good, or has ended".into(),
            ));
        }
        let c = self.take_challenge(challenge, now)?;
        let key = webauthn::register(client_data, attestation, &c, &self.origin, &self.rp_id)?;
        self.state.passkeys.retain(|k| k.id != key.id);
        self.state.passkeys.push(key);
        Ok(())
    }

    /// The IDs of the passkeys, for the page to give the browser.
    pub fn passkey_ids(&self) -> Vec<String> {
        self.state
            .passkeys
            .iter()
            .map(|k| b64::encode(&k.id.0))
            .collect()
    }

    /// Signs the person in with a passkey, and lets the app act for them.
    /// Gives back the URL to send the browser to, with the code. The caller
    /// checks first that `client_id` is the origin of an app on this node.
    ///
    /// # Errors
    ///
    /// When the request or the passkey answer is not good.
    #[expect(
        clippy::too_many_arguments,
        reason = "the fields of one WebAuthn answer"
    )]
    pub fn authorize(
        &mut self,
        req: &Authorize,
        credential: &[u8],
        client_data: &[u8],
        auth_data: &[u8],
        signature: &[u8],
        challenge: &str,
        now: u64,
    ) -> Result<String, Error> {
        if !req.redirect_uri.starts_with(&format!("{}/", req.client_id)) {
            return Err(Error::invalid(
                "the redirect URI is not on the app's origin".into(),
            ));
        }
        if req.code_challenge.len() != 43 {
            return Err(Error::invalid("a PKCE challenge (S256) is needed".into()));
        }
        let c = self.take_challenge(challenge, now)?;
        let (origin, rp_id) = (self.origin.clone(), self.rp_id.clone());
        let key = self
            .state
            .passkeys
            .iter_mut()
            .find(|k| k.id.0 == credential)
            .ok_or_else(|| Error::invalid("no such passkey".into()))?;
        key.count = webauthn::verify(key, client_data, auth_data, signature, &c, &origin, &rp_id)?;
        let code = self.random();
        self.codes.insert(
            code.clone(),
            Code {
                aud: req.client_id.clone(),
                redirect: req.redirect_uri.clone(),
                challenge: req.code_challenge.clone(),
                exp: now + CODE_TTL,
            },
        );
        let sep = if req.redirect_uri.contains('?') {
            '&'
        } else {
            '?'
        };
        Ok(format!(
            "{}{sep}code={code}&state={}",
            req.redirect_uri,
            percent(&req.state)
        ))
    }

    fn proof(
        &mut self,
        proof: &str,
        url: &str,
        now: u64,
        token: Option<&str>,
        method: &str,
    ) -> Result<String, Error> {
        let p = dpop::check(proof, method, url, now, token)?;
        if self.jtis.contains_key(&p.jti) {
            return Err(Error::invalid("a DPoP proof that was used before".into()));
        }
        self.jtis.insert(p.jti, now + 2 * dpop::SKEW);
        Ok(p.jkt)
    }

    fn tokens(&mut self, aud: String, jkt: String, family: Hash, now: u64) -> Tokens {
        let access_token = token::issue(
            &token::Claims {
                sub: self.sub.clone(),
                aud: aud.clone(),
                jkt: jkt.clone(),
                exp: now + ACCESS_TTL,
            },
            &self.key,
        );
        let refresh_token = self.random();
        self.state.refresh.push(Refresh {
            hash: Hash(sha256(refresh_token.as_bytes())),
            sub: self.sub.clone(),
            aud,
            jkt,
            family,
            exp: now + REFRESH_TTL,
            used: false,
        });
        Tokens {
            access_token,
            token_type: "DPoP",
            expires_in: ACCESS_TTL,
            refresh_token,
        }
    }

    /// Changes a code for tokens (`grant_type=authorization_code`). `proof`
    /// is the `DPoP` header of the request, made for `url`.
    ///
    /// # Errors
    ///
    /// When the code, the verifier or the proof is not good. A code is good
    /// for one try.
    #[expect(clippy::too_many_arguments, reason = "the fields of one token request")]
    pub fn token_from_code(
        &mut self,
        code: &str,
        verifier: &str,
        client_id: &str,
        redirect_uri: &str,
        proof: &str,
        url: &str,
        now: u64,
    ) -> Result<Tokens, Error> {
        self.prune(now);
        let c = self
            .codes
            .remove(code)
            .ok_or_else(|| Error::invalid("the code is not good, or has ended".into()))?;
        if c.aud != client_id || c.redirect != redirect_uri || !pkce::check(verifier, &c.challenge)
        {
            return Err(Error::invalid("the code is not for this app".into()));
        }
        let jkt = self.proof(proof, url, now, None, "POST")?;
        let family = Hash(sha256(format!("{code}{}", self.random()).as_bytes()));
        Ok(self.tokens(c.aud, jkt, family, now))
    }

    /// Changes a refresh token for new tokens (`grant_type=refresh_token`).
    /// The refresh token is used up.
    ///
    /// # Errors
    ///
    /// When the token is not good, is for another app or key, or was used
    /// before: then all the tokens of that sign-in are taken back.
    pub fn token_from_refresh(
        &mut self,
        refresh: &str,
        client_id: &str,
        proof: &str,
        url: &str,
        now: u64,
    ) -> Result<Tokens, Error> {
        self.prune(now);
        let hash = Hash(sha256(refresh.as_bytes()));
        let Some(r) = self.state.refresh.iter_mut().find(|r| r.hash == hash) else {
            return Err(Error::invalid("the refresh token is not good".into()));
        };
        if r.used {
            let family = r.family;
            self.state.refresh.retain(|r| r.family != family);
            return Err(Error::invalid(
                "the refresh token was used before; the sign-in is taken back".into(),
            ));
        }
        r.used = true;
        let (aud, jkt, family) = (r.aud.clone(), r.jkt.clone(), r.family);
        if aud != client_id {
            return Err(Error::invalid(
                "the refresh token is not for this app".into(),
            ));
        }
        if self.proof(proof, url, now, None, "POST")? != jkt {
            return Err(Error::invalid(
                "the refresh token is bound to another key".into(),
            ));
        }
        Ok(self.tokens(aud, jkt, family, now))
    }

    /// Takes back a refresh token and all the tokens of its sign-in (RFC
    /// 7009). Unknown tokens are not an error.
    pub fn revoke(&mut self, refresh: &str) {
        let hash = Hash(sha256(refresh.as_bytes()));
        if let Some(family) = self
            .state
            .refresh
            .iter()
            .find(|r| r.hash == hash)
            .map(|r| r.family)
        {
            self.state.refresh.retain(|r| r.family != family);
        }
    }

    /// Takes back every sign-in, for example when a passkey is lost.
    pub fn revoke_all(&mut self) {
        self.state.refresh.clear();
    }

    /// Checks a request to an app: `Authorization: DPoP <token>` and the
    /// `DPoP` proof for this method and URL. Gives back the person.
    ///
    /// # Errors
    ///
    /// When the token or proof is not good, or the token is for another
    /// app.
    pub fn check(
        &mut self,
        access_token: &str,
        proof: &str,
        method: &str,
        url: &str,
        app_origin: &str,
        now: u64,
    ) -> Result<String, Error> {
        let claims = token::check(access_token, self.key.public(), now)?;
        if claims.aud != app_origin {
            return Err(Error::invalid("the access token is for another app".into()));
        }
        // A token stays good only while its sign-in is not taken back.
        if !self
            .state
            .refresh
            .iter()
            .any(|r| r.jkt == claims.jkt && r.aud == claims.aud)
        {
            return Err(Error::invalid("the sign-in was taken back".into()));
        }
        if self.proof(proof, url, now, Some(access_token), method)? != claims.jkt {
            return Err(Error::invalid(
                "the access token is bound to another key".into(),
            ));
        }
        Ok(claims.sub)
    }
}

/// Percent-encodes text for a query.
fn percent(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dpop::testing::Key;
    use crate::webauthn::testing::Authenticator;

    const KERNEL: &str = "http://fileos.localhost:7341";
    const APP: &str = "http://photos.localhost:7341";
    const TOKEN_URL: &str = "http://fileos.localhost:7341/token";

    fn auth() -> Auth {
        Auth::new(
            SecretKey::from_seed(&[9; 32]),
            "did:fos:ming".into(),
            KERNEL.into(),
            "fileos.localhost".into(),
            AuthState::default(),
            [1; 32],
        )
    }

    /// Registers a passkey, and signs in for the photos app. Gives back the
    /// code and the verifier.
    fn sign_in(auth: &mut Auth, a: &mut Authenticator) -> (String, String) {
        let verifier = "a".repeat(43);
        let req = Authorize {
            client_id: APP.into(),
            redirect_uri: format!("{APP}/callback"),
            code_challenge: pkce::challenge(&verifier),
            state: "s 1".into(),
        };
        let c = auth.challenge(100);
        let ans = a.get(&b64::decode(&c).unwrap(), KERNEL, "fileos.localhost");
        let url = auth
            .authorize(
                &req,
                &a.id,
                &ans.client_data,
                &ans.auth_data,
                &ans.signature,
                &c,
                100,
            )
            .unwrap();
        assert!(url.starts_with(&format!("{APP}/callback?code=")));
        assert!(url.ends_with("&state=s%201"));
        let code = url
            .split("code=")
            .nth(1)
            .unwrap()
            .split('&')
            .next()
            .unwrap()
            .to_string();
        (code, verifier)
    }

    fn register(auth: &mut Auth, a: &mut Authenticator) {
        let setup = auth.setup_code(0);
        let c = auth.challenge(0);
        let (client, object) = a.create(&b64::decode(&c).unwrap(), KERNEL, "fileos.localhost");
        auth.register(&setup, &c, &client, &object, 1).unwrap();
        // A setup code is good once.
        let c = auth.challenge(0);
        let (client, object) = a.create(&b64::decode(&c).unwrap(), KERNEL, "fileos.localhost");
        auth.register(&setup, &c, &client, &object, 1).unwrap_err();
    }

    #[test]
    fn the_whole_flow() {
        let (mut auth, mut a, browser) = (auth(), Authenticator::new(), Key::new());
        register(&mut auth, &mut a);
        let (code, verifier) = sign_in(&mut auth, &mut a);

        // A wrong verifier uses up the code.
        let p = browser.proof("POST", TOKEN_URL, 100, "j1", None);
        auth.token_from_code(
            &code,
            &"b".repeat(43),
            APP,
            &format!("{APP}/callback"),
            &p,
            TOKEN_URL,
            100,
        )
        .unwrap_err();
        let (code, verifier) = {
            let _ = verifier;
            sign_in(&mut auth, &mut a)
        };
        let p = browser.proof("POST", TOKEN_URL, 100, "j2", None);
        let t = auth
            .token_from_code(
                &code,
                &verifier,
                APP,
                &format!("{APP}/callback"),
                &p,
                TOKEN_URL,
                100,
            )
            .unwrap();
        assert_eq!(t.token_type, "DPoP");

        // The app sends the token with a proof; the kernel says who it is.
        let url = format!("{APP}/photos");
        let p = browser.proof("GET", &url, 110, "j3", Some(&t.access_token));
        assert_eq!(
            auth.check(&t.access_token, &p, "GET", &url, APP, 110)
                .unwrap(),
            "did:fos:ming"
        );
        // The same proof again, another app, another key, or later.
        auth.check(&t.access_token, &p, "GET", &url, APP, 110)
            .unwrap_err();
        let p = browser.proof("GET", &url, 110, "j4", Some(&t.access_token));
        auth.check(
            &t.access_token,
            &p,
            "GET",
            &url,
            "http://chat.localhost:7341",
            110,
        )
        .unwrap_err();
        let thief = Key::new().proof("GET", &url, 110, "j5", Some(&t.access_token));
        auth.check(&t.access_token, &thief, "GET", &url, APP, 110)
            .unwrap_err();
        let p = browser.proof("GET", &url, 100 + ACCESS_TTL, "j6", Some(&t.access_token));
        auth.check(&t.access_token, &p, "GET", &url, APP, 100 + ACCESS_TTL)
            .unwrap_err();

        // Refresh: a new pair; the old refresh token used again takes back
        // the whole sign-in.
        let p = browser.proof("POST", TOKEN_URL, 200, "j7", None);
        let t2 = auth
            .token_from_refresh(&t.refresh_token, APP, &p, TOKEN_URL, 200)
            .unwrap();
        let p = browser.proof("POST", TOKEN_URL, 201, "j8", None);
        auth.token_from_refresh(&t.refresh_token, APP, &p, TOKEN_URL, 201)
            .unwrap_err();
        let p = browser.proof("POST", TOKEN_URL, 202, "j9", None);
        auth.token_from_refresh(&t2.refresh_token, APP, &p, TOKEN_URL, 202)
            .unwrap_err();
        let p = browser.proof("GET", &url, 203, "j10", Some(&t2.access_token));
        auth.check(&t2.access_token, &p, "GET", &url, APP, 203)
            .unwrap_err();
    }

    #[test]
    fn revoke_takes_back_the_sign_in() {
        let (mut auth, mut a, browser) = (auth(), Authenticator::new(), Key::new());
        register(&mut auth, &mut a);
        let (code, verifier) = sign_in(&mut auth, &mut a);
        let p = browser.proof("POST", TOKEN_URL, 100, "j1", None);
        let t = auth
            .token_from_code(
                &code,
                &verifier,
                APP,
                &format!("{APP}/callback"),
                &p,
                TOKEN_URL,
                100,
            )
            .unwrap();
        auth.revoke(&t.refresh_token);
        let url = format!("{APP}/x");
        let p = browser.proof("GET", &url, 101, "j2", Some(&t.access_token));
        auth.check(&t.access_token, &p, "GET", &url, APP, 101)
            .unwrap_err();
    }

    #[test]
    fn a_redirect_to_another_origin_is_not_taken() {
        let (mut auth, mut a) = (auth(), Authenticator::new());
        register(&mut auth, &mut a);
        let req = Authorize {
            client_id: APP.into(),
            redirect_uri: "http://evil.localhost:7341/x".into(),
            code_challenge: pkce::challenge(&"a".repeat(43)),
            state: String::new(),
        };
        let c = auth.challenge(100);
        let ans = a.get(&b64::decode(&c).unwrap(), KERNEL, "fileos.localhost");
        auth.authorize(
            &req,
            &a.id,
            &ans.client_data,
            &ans.auth_data,
            &ans.signature,
            &c,
            100,
        )
        .unwrap_err();
    }
}
