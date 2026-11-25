// SPDX-License-Identifier: AGPL-3.0-only

//! Certificates from an ACME certificate authority (RFC 8555), such as
//! Let's Encrypt, using the HTTP-01 challenge.
//!
//! The client is generic over a [`Transport`] that sends one HTTP request, so
//! it can run over any HTTP client and be tested without a network. The
//! HTTP server answers `/.well-known/acme-challenge/<token>` from the shared
//! [`Challenges`] table while an order is in progress.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::digest;
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use serde_json::{Value, json};

/// The Let's Encrypt production directory.
pub const LETS_ENCRYPT: &str = "https://acme-v02.api.letsencrypt.org/directory";

/// The Let's Encrypt staging directory, for testing.
pub const LETS_ENCRYPT_STAGING: &str = "https://acme-staging-v02.api.letsencrypt.org/directory";

/// The path prefix under which HTTP-01 tokens are served.
pub const CHALLENGE_PATH: &str = "/.well-known/acme-challenge/";

/// An ACME failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "acme: {}", self.0)
    }
}

impl std::error::Error for Error {}

fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

/// Sends one HTTP request and returns the response.
pub trait Transport {
    /// Send `req`; transport failures are reported as text.
    fn send(
        &self,
        req: http::Request<Vec<u8>>,
    ) -> impl Future<Output = Result<http::Response<Vec<u8>>, String>> + Send;
}

/// Pending HTTP-01 challenges: token to key authorization. Clones share the
/// same table.
#[derive(Clone, Default, Debug)]
pub struct Challenges(Arc<Mutex<HashMap<String, String>>>);

impl Challenges {
    /// An empty table.
    pub fn new() -> Challenges {
        Challenges::default()
    }

    /// The key authorization to serve for `token`, if one is pending.
    pub fn get(&self, token: &str) -> Option<String> {
        self.0.lock().ok()?.get(token).cloned()
    }

    /// Start serving a key authorization for `token`.
    pub fn insert(&self, token: String, key_authorization: String) {
        if let Ok(mut m) = self.0.lock() {
            m.insert(token, key_authorization);
        }
    }

    /// Stop serving `token`.
    pub fn remove(&self, token: &str) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(token);
        }
    }
}

#[derive(Clone)]
struct Directory {
    new_nonce: String,
    new_account: String,
    new_order: String,
}

/// An ACME client bound to one account key and one directory.
pub struct Acme<T> {
    transport: T,
    directory_url: String,
    pkcs8: Vec<u8>,
    key: EcdsaKeyPair,
    jwk: Value,
    thumbprint: String,
    directory: Mutex<Option<Directory>>,
    nonce: Mutex<Option<String>>,
    kid: Mutex<Option<String>>,
    poll_interval: Duration,
    poll_attempts: u32,
}

impl<T> std::fmt::Debug for Acme<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Acme").finish_non_exhaustive()
    }
}

fn b64(b: impl AsRef<[u8]>) -> String {
    URL_SAFE_NO_PAD.encode(b)
}

fn lock<V: Clone>(m: &Mutex<Option<V>>) -> Option<V> {
    m.lock().ok()?.clone()
}

fn store<V>(m: &Mutex<Option<V>>, v: Option<V>) {
    if let Ok(mut g) = m.lock() {
        *g = v;
    }
}

fn header(resp: &http::Response<Vec<u8>>, name: &str) -> Option<String> {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

fn parse_json(resp: &http::Response<Vec<u8>>) -> Result<Value, Error> {
    serde_json::from_slice(resp.body()).or(err("response is not JSON"))
}

fn str_field<'a>(v: &'a Value, name: &str) -> Result<&'a str, Error> {
    v.get(name)
        .and_then(Value::as_str)
        .ok_or(Error(format!("response lacks {name}")))
}

impl<T: Transport> Acme<T> {
    /// A client for the directory at `directory_url`. Pass the stored
    /// account key, or `None` to create one (save [`Acme::account_key_pkcs8`]).
    ///
    /// # Errors
    ///
    /// Returns an error if the stored account key is invalid or a new key cannot be generated.
    pub fn new(
        transport: T,
        directory_url: &str,
        account_key_pkcs8: Option<Vec<u8>>,
    ) -> Result<Acme<T>, Error> {
        let rng = SystemRandom::new();
        let pkcs8 = match account_key_pkcs8 {
            Some(k) => k,
            None => EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
                .or(err("key generation"))?
                .as_ref()
                .to_vec(),
        };
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8, &rng)
            .or(err("bad account key"))?;
        let point = key.public_key().as_ref();
        if point.len() != 65 {
            return err("unexpected public key");
        }
        let (x, y) = (b64(&point[1..33]), b64(&point[33..65]));
        // RFC 7638: the required members in lexicographic order, no spaces.
        let canonical = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#);
        let thumbprint = b64(digest::digest(&digest::SHA256, canonical.as_bytes()));
        Ok(Acme {
            transport,
            directory_url: directory_url.to_string(),
            pkcs8,
            key,
            jwk: json!({ "crv": "P-256", "kty": "EC", "x": x, "y": y }),
            thumbprint,
            directory: Mutex::new(None),
            nonce: Mutex::new(None),
            kid: Mutex::new(None),
            poll_interval: Duration::from_secs(2),
            poll_attempts: 30,
        })
    }

    /// Change how often and how many times authorizations and orders are
    /// polled (default: every 2 s, 30 times).
    #[must_use]
    pub fn with_polling(mut self, interval: Duration, attempts: u32) -> Acme<T> {
        self.poll_interval = interval;
        self.poll_attempts = attempts.max(1);
        self
    }

    /// The account key in PKCS #8, for storage.
    pub fn account_key_pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }

    /// The HTTP-01 key authorization for a token.
    pub fn key_authorization(&self, token: &str) -> String {
        format!("{token}.{}", self.thumbprint)
    }

    async fn send(&self, req: http::Request<Vec<u8>>) -> Result<http::Response<Vec<u8>>, Error> {
        let resp = self.transport.send(req).await.map_err(Error)?;
        if let Some(n) = header(&resp, "replay-nonce") {
            store(&self.nonce, Some(n));
        }
        Ok(resp)
    }

    async fn directory(&self) -> Result<Directory, Error> {
        if let Some(d) = lock(&self.directory) {
            return Ok(d);
        }
        let req = http::Request::get(self.directory_url.as_str())
            .body(Vec::new())
            .or(err("bad directory URL"))?;
        let resp = self.send(req).await?;
        if !resp.status().is_success() {
            return err(format!("directory: HTTP {}", resp.status()));
        }
        let v = parse_json(&resp)?;
        let d = Directory {
            new_nonce: str_field(&v, "newNonce")?.into(),
            new_account: str_field(&v, "newAccount")?.into(),
            new_order: str_field(&v, "newOrder")?.into(),
        };
        store(&self.directory, Some(d.clone()));
        Ok(d)
    }

    async fn nonce(&self) -> Result<String, Error> {
        let taken = self.nonce.lock().ok().and_then(|mut n| n.take());
        if let Some(n) = taken {
            return Ok(n);
        }
        let dir = self.directory().await?;
        let req = http::Request::head(dir.new_nonce.as_str())
            .body(Vec::new())
            .or(err("bad newNonce URL"))?;
        let resp = self.transport.send(req).await.map_err(Error)?;
        header(&resp, "replay-nonce").ok_or(Error("no Replay-Nonce".into()))
    }

    /// A flattened JWS (RFC 7515) signed with the account key. `payload`
    /// `None` is a POST-as-GET.
    fn jws(&self, url: &str, nonce: &str, payload: Option<&Value>) -> Result<Vec<u8>, Error> {
        let mut protected = json!({ "alg": "ES256", "nonce": nonce, "url": url });
        match lock(&self.kid) {
            Some(kid) => protected["kid"] = kid.into(),
            None => protected["jwk"] = self.jwk.clone(),
        }
        let protected = b64(protected.to_string());
        let payload = payload.map(|p| b64(p.to_string())).unwrap_or_default();
        let sig = self
            .key
            .sign(
                &SystemRandom::new(),
                format!("{protected}.{payload}").as_bytes(),
            )
            .or(err("signing"))?;
        Ok(
            json!({ "protected": protected, "payload": payload, "signature": b64(sig) })
                .to_string()
                .into_bytes(),
        )
    }

    /// POST a signed request; retries once on `badNonce`. Errors on a
    /// non-success status, with the problem document's text.
    async fn post(
        &self,
        url: &str,
        payload: Option<&Value>,
    ) -> Result<http::Response<Vec<u8>>, Error> {
        let mut retried = false;
        loop {
            let nonce = self.nonce().await?;
            let req = http::Request::post(url)
                .header("content-type", "application/jose+json")
                .body(self.jws(url, &nonce, payload)?)
                .or(err("bad URL"))?;
            let resp = self.send(req).await?;
            if resp.status().is_success() {
                return Ok(resp);
            }
            let problem: Value = serde_json::from_slice(resp.body()).unwrap_or(Value::Null);
            let kind = problem.get("type").and_then(Value::as_str).unwrap_or("");
            if kind == "urn:ietf:params:acme:error:badNonce" && !retried {
                retried = true;
                continue;
            }
            let detail = problem.get("detail").and_then(Value::as_str).unwrap_or("");
            return err(format!("{url}: HTTP {} {kind} {detail}", resp.status()));
        }
    }

    async fn account(&self, contact: Option<&str>) -> Result<(), Error> {
        if lock(&self.kid).is_some() {
            return Ok(());
        }
        let dir = self.directory().await?;
        let mut payload = json!({ "termsOfServiceAgreed": true });
        if let Some(c) = contact {
            let c = if c.contains(':') {
                c.to_string()
            } else {
                format!("mailto:{c}")
            };
            payload["contact"] = json!([c]);
        }
        let resp = self.post(&dir.new_account, Some(&payload)).await?;
        let kid = header(&resp, "location").ok_or(Error("account has no Location".into()))?;
        store(&self.kid, Some(kid));
        Ok(())
    }

    /// POST-as-GET `url` until its `status` leaves `waiting`; returns the
    /// final object.
    async fn poll(&self, url: &str, waiting: &[&str]) -> Result<Value, Error> {
        for attempt in 0..self.poll_attempts {
            if attempt > 0 {
                tokio::time::sleep(self.poll_interval).await;
            }
            let v = parse_json(&self.post(url, None).await?)?;
            let status = str_field(&v, "status")?;
            if !waiting.contains(&status) {
                return Ok(v);
            }
        }
        err(format!("{url}: still pending"))
    }

    async fn authorize(
        &self,
        url: &str,
        challenges: &Challenges,
        tokens: &mut Vec<String>,
    ) -> Result<(), Error> {
        let authz = parse_json(&self.post(url, None).await?)?;
        if str_field(&authz, "status")? == "valid" {
            return Ok(());
        }
        let challenge = authz
            .get("challenges")
            .and_then(Value::as_array)
            .and_then(|cs| {
                cs.iter()
                    .find(|c| c.get("type").and_then(Value::as_str) == Some("http-01"))
            })
            .ok_or(Error("no http-01 challenge".into()))?;
        let token = str_field(challenge, "token")?;
        if token.is_empty()
            || !token
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        {
            return err("bad challenge token");
        }
        challenges.insert(token.to_string(), self.key_authorization(token));
        tokens.push(token.to_string());
        self.post(str_field(challenge, "url")?, Some(&json!({})))
            .await?;
        let authz = self.poll(url, &["pending", "processing"]).await?;
        match str_field(&authz, "status")? {
            "valid" => Ok(()),
            status => {
                let detail = authz
                    .get("challenges")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|c| c.get("error")?.get("detail")?.as_str())
                    .collect::<Vec<_>>()
                    .join("; ");
                err(format!("authorization {status}: {detail}"))
            }
        }
    }

    /// Get a certificate for `domains`. The account is registered (agreeing
    /// to the terms of service) on first use. HTTP-01 tokens are published
    /// in `challenges` while the CA validates and removed afterwards.
    /// Returns the PEM certificate chain and the PEM private key.
    ///
    /// # Errors
    ///
    /// Returns an error if account registration, validation, signing or certificate retrieval
    /// fails.
    pub async fn obtain(
        &self,
        domains: &[String],
        contact: Option<&str>,
        challenges: &Challenges,
    ) -> Result<(String, String), Error> {
        let mut tokens = Vec::new();
        let result = self
            .obtain_inner(domains, contact, challenges, &mut tokens)
            .await;
        for t in &tokens {
            challenges.remove(t);
        }
        result
    }

    async fn obtain_inner(
        &self,
        domains: &[String],
        contact: Option<&str>,
        challenges: &Challenges,
        tokens: &mut Vec<String>,
    ) -> Result<(String, String), Error> {
        if domains.is_empty() {
            return err("no domains");
        }
        self.account(contact).await?;
        let dir = self.directory().await?;
        let identifiers: Vec<Value> = domains
            .iter()
            .map(|d| json!({ "type": "dns", "value": d }))
            .collect();
        let resp = self
            .post(&dir.new_order, Some(&json!({ "identifiers": identifiers })))
            .await?;
        let order_url = header(&resp, "location").ok_or(Error("order has no Location".into()))?;
        let order = parse_json(&resp)?;
        let finalize = str_field(&order, "finalize")?.to_string();
        let authorizations = order
            .get("authorizations")
            .and_then(Value::as_array)
            .ok_or(Error("order lacks authorizations".into()))?;
        for a in authorizations {
            let url = a.as_str().ok_or(Error("bad authorization URL".into()))?;
            self.authorize(url, challenges, tokens).await?;
        }
        let order = self.poll(&order_url, &["pending"]).await?;
        if !matches!(
            str_field(&order, "status")?,
            "ready" | "valid" | "processing"
        ) {
            return err(format!("order is {}", str_field(&order, "status")?));
        }

        let (csr, key_pem) = {
            let key = rcgen::KeyPair::generate().or(err("certificate key"))?;
            let params = rcgen::CertificateParams::new(domains.to_vec()).or(err("bad domain"))?;
            let csr = params.serialize_request(&key).or(err("csr"))?;
            (csr.der().to_vec(), key.serialize_pem())
        };
        if str_field(&order, "status")? == "ready" {
            self.post(&finalize, Some(&json!({ "csr": b64(csr) })))
                .await?;
        }
        let order = self.poll(&order_url, &["ready", "processing"]).await?;
        if str_field(&order, "status")? != "valid" {
            return err(format!("order is {}", str_field(&order, "status")?));
        }
        let cert_url = str_field(&order, "certificate")?;
        let chain = self.post(cert_url, None).await?;
        let chain = String::from_utf8(chain.into_body()).or(err("certificate is not text"))?;
        if !chain.contains("-----BEGIN CERTIFICATE-----") {
            return err("certificate is not PEM");
        }
        Ok((chain, key_pem))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};

    const BASE: &str = "https://acme.test";
    const CHAIN: &str = "-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n";

    #[derive(Default)]
    struct State {
        nonces: Vec<String>,
        counter: u32,
        jwk: Option<Value>,
        bad_nonce_sent: bool,
        authz_valid: bool,
        order_status: &'static str,
        finalized: bool,
        certificate_polls: u32,
        log: Vec<String>,
    }

    /// A minimal in-memory ACME server.
    #[derive(Clone)]
    struct Fake {
        state: Arc<Mutex<State>>,
        challenges: Challenges,
    }

    fn reply(
        status: u16,
        nonce: &str,
        location: Option<&str>,
        body: Value,
    ) -> http::Response<Vec<u8>> {
        let mut b = http::Response::builder()
            .status(status)
            .header("replay-nonce", nonce);
        if let Some(l) = location {
            b = b.header("location", l);
        }
        b.body(body.to_string().into_bytes()).unwrap()
    }

    fn thumbprint(jwk: &Value) -> String {
        let canonical = format!(
            r#"{{"crv":"P-256","kty":"EC","x":"{}","y":"{}"}}"#,
            jwk["x"].as_str().unwrap(),
            jwk["y"].as_str().unwrap()
        );
        b64(digest::digest(&digest::SHA256, canonical.as_bytes()))
    }

    impl Fake {
        fn handle(&self, req: http::Request<Vec<u8>>) -> http::Response<Vec<u8>> {
            let mut st = self.state.lock().unwrap();
            st.counter += 1;
            let nonce = format!("nonce-{}", st.counter);
            st.nonces.push(nonce.clone());
            let url = req.uri().to_string();
            let path = req.uri().path().to_string();
            st.log.push(format!("{} {path}", req.method()));
            if req.method() == http::Method::GET && path == "/dir" {
                let dir = json!({
                    "newNonce": format!("{BASE}/nonce"),
                    "newAccount": format!("{BASE}/acct"),
                    "newOrder": format!("{BASE}/order"),
                });
                return http::Response::new(dir.to_string().into_bytes());
            }
            if req.method() == http::Method::HEAD && path == "/nonce" {
                return reply(200, &nonce, None, Value::Null);
            }
            assert_eq!(req.method(), http::Method::POST);
            assert_eq!(req.headers()["content-type"], "application/jose+json");

            // Check the JWS.
            let jws: Value = serde_json::from_slice(req.body()).unwrap();
            let protected_b64 = jws["protected"].as_str().unwrap();
            let payload_b64 = jws["payload"].as_str().unwrap();
            let protected: Value =
                serde_json::from_slice(&URL_SAFE_NO_PAD.decode(protected_b64).unwrap()).unwrap();
            assert_eq!(protected["alg"], "ES256");
            assert_eq!(protected["url"], url.as_str());
            let used = protected["nonce"].as_str().unwrap().to_string();
            let pos = st
                .nonces
                .iter()
                .position(|n| *n == used)
                .expect("unknown nonce");
            st.nonces.remove(pos);
            if path == "/acct" {
                assert!(protected.get("kid").is_none());
                st.jwk = Some(protected["jwk"].clone());
            } else {
                assert!(protected.get("jwk").is_none());
                assert_eq!(protected["kid"], format!("{BASE}/acct/1"));
            }
            let jwk = st.jwk.clone().unwrap();
            let mut point = vec![4u8];
            point.extend(URL_SAFE_NO_PAD.decode(jwk["x"].as_str().unwrap()).unwrap());
            point.extend(URL_SAFE_NO_PAD.decode(jwk["y"].as_str().unwrap()).unwrap());
            UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &point)
                .verify(
                    format!("{protected_b64}.{payload_b64}").as_bytes(),
                    &URL_SAFE_NO_PAD
                        .decode(jws["signature"].as_str().unwrap())
                        .unwrap(),
                )
                .expect("bad JWS signature");
            let payload: Value = if payload_b64.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload_b64).unwrap()).unwrap()
            };

            let order = |st: &State| {
                let mut o = json!({
                    "status": st.order_status,
                    "identifiers": [{ "type": "dns", "value": "home.example" }],
                    "authorizations": [format!("{BASE}/authz/1")],
                    "finalize": format!("{BASE}/order/1/finalize"),
                });
                if st.order_status == "valid" {
                    o["certificate"] = format!("{BASE}/cert/1").into();
                }
                o
            };
            match path.as_str() {
                "/acct" => {
                    assert_eq!(payload["termsOfServiceAgreed"], true);
                    assert_eq!(payload["contact"][0], "mailto:me@home.example");
                    reply(
                        201,
                        &nonce,
                        Some(&format!("{BASE}/acct/1")),
                        json!({ "status": "valid" }),
                    )
                }
                "/order" if !st.bad_nonce_sent => {
                    st.bad_nonce_sent = true;
                    reply(
                        400,
                        &nonce,
                        None,
                        json!({ "type": "urn:ietf:params:acme:error:badNonce" }),
                    )
                }
                "/order" => {
                    assert_eq!(payload["identifiers"][0]["value"], "home.example");
                    st.order_status = "pending";
                    reply(201, &nonce, Some(&format!("{BASE}/order/1")), order(&st))
                }
                "/authz/1" => {
                    assert!(payload.is_null());
                    let status = if st.authz_valid { "valid" } else { "pending" };
                    reply(
                        200,
                        &nonce,
                        None,
                        json!({
                            "status": status,
                            "identifier": { "type": "dns", "value": "home.example" },
                            "challenges": [
                                { "type": "dns-01", "url": format!("{BASE}/chall/0"), "token": "dns" },
                                { "type": "http-01", "url": format!("{BASE}/chall/1"), "token": "tok_1-A" },
                            ],
                        }),
                    )
                }
                "/chall/1" => {
                    assert_eq!(payload, json!({}));
                    let expected = format!("tok_1-A.{}", thumbprint(&jwk));
                    assert_eq!(self.challenges.get("tok_1-A"), Some(expected));
                    st.authz_valid = true;
                    st.order_status = "ready";
                    reply(
                        200,
                        &nonce,
                        None,
                        json!({ "type": "http-01", "status": "processing" }),
                    )
                }
                "/order/1" => {
                    if st.finalized {
                        st.certificate_polls += 1;
                        if st.certificate_polls > 1 {
                            st.order_status = "valid";
                        }
                    }
                    reply(200, &nonce, None, order(&st))
                }
                "/order/1/finalize" => {
                    assert_eq!(st.order_status, "ready");
                    let csr = URL_SAFE_NO_PAD
                        .decode(payload["csr"].as_str().unwrap())
                        .unwrap();
                    assert_eq!(csr[0], 0x30); // A DER SEQUENCE.
                    st.finalized = true;
                    st.order_status = "processing";
                    reply(200, &nonce, None, order(&st))
                }
                "/cert/1" => {
                    assert!(payload.is_null());
                    http::Response::builder()
                        .status(200)
                        .header("replay-nonce", &nonce)
                        .header("content-type", "application/pem-certificate-chain")
                        .body(CHAIN.as_bytes().to_vec())
                        .unwrap()
                }
                _ => reply(
                    404,
                    &nonce,
                    None,
                    json!({ "type": "urn:ietf:params:acme:error:malformed" }),
                ),
            }
        }
    }

    impl Transport for Fake {
        fn send(
            &self,
            req: http::Request<Vec<u8>>,
        ) -> impl Future<Output = Result<http::Response<Vec<u8>>, String>> + Send {
            let resp = self.handle(req);
            async move { Ok(resp) }
        }
    }

    #[tokio::test]
    async fn full_flow() {
        fn assert_send<F: Send>(_: &F) {}

        let challenges = Challenges::new();
        let fake = Fake {
            state: Arc::default(),
            challenges: challenges.clone(),
        };
        let acme = Acme::new(fake.clone(), &format!("{BASE}/dir"), None)
            .unwrap()
            .with_polling(Duration::ZERO, 5);
        let domains = ["home.example".to_string()];
        let fut = acme.obtain(&domains, Some("me@home.example"), &challenges);
        assert_send(&fut);
        let (chain, key) = fut.await.unwrap();
        assert_eq!(chain, CHAIN);
        assert!(key.contains("PRIVATE KEY"));
        // Tokens are withdrawn once the order is done.
        assert_eq!(challenges.get("tok_1-A"), None);

        let log = fake.state.lock().unwrap().log.clone();
        assert_eq!(
            log,
            [
                "GET /dir",
                "HEAD /nonce",
                "POST /acct",
                "POST /order",
                "POST /order",
                "POST /authz/1",
                "POST /chall/1",
                "POST /authz/1",
                "POST /order/1",
                "POST /order/1/finalize",
                "POST /order/1",
                "POST /order/1",
                "POST /cert/1",
            ]
        );

        // The same key loads again with the same thumbprint.
        let again = Acme::new(fake, BASE, Some(acme.account_key_pkcs8().to_vec())).unwrap();
        assert_eq!(again.key_authorization("t"), acme.key_authorization("t"));
    }

    #[test]
    fn thumbprint_rfc7638_shape() {
        struct Never;
        impl Transport for Never {
            #[expect(
                clippy::manual_async_fn,
                reason = "the mock mirrors the transport trait signature"
            )]
            fn send(
                &self,
                _: http::Request<Vec<u8>>,
            ) -> impl Future<Output = Result<http::Response<Vec<u8>>, String>> + Send {
                async { Err("offline".to_string()) }
            }
        }
        let acme = Acme::new(Never, BASE, None).unwrap();
        assert_eq!(
            acme.key_authorization("abc"),
            format!("abc.{}", thumbprint(&acme.jwk))
        );
        assert_eq!(acme.thumbprint.len(), 43);
    }
}
