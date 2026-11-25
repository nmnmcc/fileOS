// SPDX-License-Identifier: AGPL-3.0-only

//! Web Push sending.
//!
//! A browser subscription gives an endpoint at a push service and two keys.
//! We encrypt the message for the browser (RFC 8291, `aes128gcm` content
//! coding of RFC 8188) and authenticate ourselves to the push service with a
//! VAPID token (RFC 8292). The push service only sees ciphertext.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::{URL_SAFE_NO_PAD, URL_SAFE_NO_PAD_INDIFFERENT};
use ring::rand::{SecureRandom, SystemRandom};
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair};
use ring::{aead, agreement, hkdf};

/// The record size we declare; one record holds the whole message.
const RECORD_SIZE: u32 = 4096;

/// The largest plaintext a push service must accept (RFC 8291 §4).
pub const MAX_PLAINTEXT: usize = 3993;

/// How long the push service should keep an undelivered message, in seconds.
const TTL: u32 = 24 * 3600;

/// How long a VAPID token is valid, in seconds (RFC 8292 allows up to 24 h).
const VAPID_LIFETIME: u64 = 12 * 3600;

/// A Web Push failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "web push: {}", self.0)
    }
}

impl std::error::Error for Error {}

fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

pub(crate) fn b64(bytes: impl AsRef<[u8]>) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

/// A VAPID key: the ECDSA P-256 key that identifies this server to push
/// services.
pub struct Vapid {
    pkcs8: Vec<u8>,
    pair: EcdsaKeyPair,
}

impl Vapid {
    /// Generate a new key. Store [`Vapid::pkcs8`] to keep it.
    ///
    /// # Errors
    ///
    /// Returns an error if the signing backend cannot generate or load the key.
    pub fn generate() -> Result<Vapid, Error> {
        let rng = SystemRandom::new();
        let doc = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .or(err("key generation"))?;
        Vapid::from_pkcs8(doc.as_ref())
    }

    /// Load a key from its PKCS #8 bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the encoded signing key is invalid.
    pub fn from_pkcs8(pkcs8: &[u8]) -> Result<Vapid, Error> {
        let pair = EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            pkcs8,
            &SystemRandom::new(),
        )
        .or(err("bad PKCS #8 key"))?;
        Ok(Vapid {
            pkcs8: pkcs8.to_vec(),
            pair,
        })
    }

    /// The PKCS #8 bytes, for storage.
    pub fn pkcs8(&self) -> &[u8] {
        &self.pkcs8
    }

    /// The public key as base64url of the uncompressed point: the
    /// `applicationServerKey` browsers need when subscribing.
    pub fn public_key(&self) -> String {
        b64(self.pair.public_key())
    }

    /// The `Authorization` header value for a push service at
    /// `audience_origin` (such as `https://fcm.googleapis.com`):
    /// `vapid t=<JWT>, k=<public key>`. `subject` is a `mailto:` or `https:`
    /// contact; `exp` is the token's expiry in Unix seconds.
    ///
    /// # Errors
    ///
    /// Returns an error if the VAPID token cannot be signed.
    pub fn authorization(
        &self,
        audience_origin: &str,
        subject: &str,
        exp: u64,
    ) -> Result<String, Error> {
        let header = b64(br#"{"typ":"JWT","alg":"ES256"}"#);
        let claims = serde_json::json!({ "aud": audience_origin, "exp": exp, "sub": subject });
        let signing_input = format!("{header}.{}", b64(claims.to_string()));
        let sig = self
            .pair
            .sign(&SystemRandom::new(), signing_input.as_bytes())
            .or(err("signing"))?;
        Ok(format!(
            "vapid t={signing_input}.{}, k={}",
            b64(sig),
            self.public_key()
        ))
    }
}

impl fmt::Debug for Vapid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Vapid({})", self.public_key())
    }
}

/// An HKDF output length.
struct Len(usize);

impl hkdf::KeyType for Len {
    fn len(&self) -> usize {
        self.0
    }
}

fn hkdf(salt: &[u8], ikm: &[u8], info: &[&[u8]], out: &mut [u8]) -> Result<(), Error> {
    hkdf::Salt::new(hkdf::HKDF_SHA256, salt)
        .extract(ikm)
        .expand(info, Len(out.len()))
        .and_then(|okm| okm.fill(out))
        .or(err("hkdf"))
}

/// Derive the content encryption key and nonce (RFC 8291 §3.3–3.4).
fn derive(
    ecdh_secret: &[u8],
    auth: &[u8],
    ua_public: &[u8],
    as_public: &[u8],
    salt: &[u8],
) -> Result<([u8; 16], [u8; 12]), Error> {
    let mut ikm = [0u8; 32];
    hkdf(
        auth,
        ecdh_secret,
        &[b"WebPush: info\0", ua_public, as_public],
        &mut ikm,
    )?;
    let mut cek = [0u8; 16];
    let mut nonce = [0u8; 12];
    hkdf(salt, &ikm, &[b"Content-Encoding: aes128gcm\0"], &mut cek)?;
    hkdf(salt, &ikm, &[b"Content-Encoding: nonce\0"], &mut nonce)?;
    Ok((cek, nonce))
}

fn decode_b64(s: &str, what: &str) -> Result<Vec<u8>, Error> {
    URL_SAFE_NO_PAD_INDIFFERENT
        .decode(s.trim())
        .or(err(format!("bad base64 in {what}")))
}

/// Encrypt a message for a subscription's `p256dh` and `auth` keys (both
/// base64url). Returns the `aes128gcm` body: header, then a single record.
///
/// # Errors
///
/// Returns an error if subscription keys are invalid, the payload is too large, or encryption
/// fails.
pub fn encrypt(p256dh_b64url: &str, auth_b64url: &str, plaintext: &[u8]) -> Result<Vec<u8>, Error> {
    let ua_public = decode_b64(p256dh_b64url, "p256dh")?;
    let auth = decode_b64(auth_b64url, "auth")?;
    if ua_public.len() != 65 || ua_public[0] != 4 {
        return err("p256dh is not an uncompressed P-256 point");
    }
    if auth.len() != 16 {
        return err("auth secret must be 16 bytes");
    }
    if plaintext.len() > MAX_PLAINTEXT {
        return err("message too long");
    }
    let rng = SystemRandom::new();
    let mut salt = [0u8; 16];
    rng.fill(&mut salt).or(err("random"))?;
    let private =
        agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).or(err("keygen"))?;
    let as_public = private.compute_public_key().or(err("keygen"))?;
    let as_public = as_public.as_ref();
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, &ua_public);
    let secret =
        agreement::agree_ephemeral(private, &peer, <[u8]>::to_vec).or(err("bad p256dh"))?;
    let (cek, nonce) = derive(&secret, &auth, &ua_public, as_public, &salt)?;

    let mut out = Vec::with_capacity(86 + plaintext.len() + 17);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&RECORD_SIZE.to_be_bytes());
    out.push(as_public.len() as u8);
    out.extend_from_slice(as_public);

    let mut record = Vec::with_capacity(plaintext.len() + 17);
    record.extend_from_slice(plaintext);
    record.push(2); // Last-record delimiter, no further padding.
    let key = aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_128_GCM, &cek).or(err("aead key"))?,
    );
    // Record sequence number 0: the nonce is used as derived.
    key.seal_in_place_append_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::empty(),
        &mut record,
    )
    .or(err("encrypt"))?;
    out.extend_from_slice(&record);
    Ok(out)
}

/// The origin (`scheme://host[:port]`) of an endpoint URL, the VAPID audience.
fn origin(endpoint: &http::Uri) -> Result<String, Error> {
    let scheme = endpoint
        .scheme_str()
        .ok_or(Error("endpoint has no scheme".into()))?;
    let authority = endpoint
        .authority()
        .ok_or(Error("endpoint has no host".into()))?;
    Ok(format!(
        "{scheme}://{}",
        authority.as_str().to_ascii_lowercase()
    ))
}

/// Build the push request for a subscription (the browser's
/// `PushSubscription` JSON: `{endpoint, keys: {p256dh, auth}}`). `subject`
/// is the VAPID contact; `now` is Unix seconds.
///
/// # Errors
///
/// Returns an error if a subscription field or endpoint is invalid, or signing or encryption fails.
pub fn request(
    subscription: &serde_json::Value,
    payload: &[u8],
    vapid: &Vapid,
    subject: &str,
    now: u64,
) -> Result<http::Request<Vec<u8>>, Error> {
    let field = |v: &serde_json::Value, name: &str| -> Result<String, Error> {
        v.get(name)
            .and_then(|x| x.as_str())
            .map(str::to_string)
            .ok_or(Error(format!("subscription lacks {name}")))
    };
    let endpoint = field(subscription, "endpoint")?;
    let keys = subscription
        .get("keys")
        .ok_or(Error("subscription lacks keys".into()))?;
    let uri: http::Uri = endpoint.parse().or(err("bad endpoint"))?;
    if uri.scheme() != Some(&http::uri::Scheme::HTTPS) {
        return err("endpoint must be https");
    }
    let body = encrypt(&field(keys, "p256dh")?, &field(keys, "auth")?, payload)?;
    let authorization = vapid.authorization(&origin(&uri)?, subject, now + VAPID_LIFETIME)?;
    http::Request::post(uri)
        .header("ttl", TTL.to_string())
        .header("content-encoding", "aes128gcm")
        .header("content-type", "application/octet-stream")
        .header("authorization", authorization)
        .body(body)
        .or(err("build request"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{ECDSA_P256_SHA256_FIXED, UnparsedPublicKey};

    /// The browser side of RFC 8291: decrypt with the receiver's key.
    fn decrypt(
        receiver: agreement::EphemeralPrivateKey,
        ua_public: &[u8],
        auth: &[u8],
        body: &[u8],
    ) -> Vec<u8> {
        let salt = &body[..16];
        let rs = u32::from_be_bytes(body[16..20].try_into().unwrap());
        assert_eq!(rs, 4096);
        let idlen = body[20] as usize;
        let as_public = &body[21..21 + idlen];
        let mut record = body[21 + idlen..].to_vec();
        assert!(record.len() <= rs as usize);
        let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, as_public);
        let secret = agreement::agree_ephemeral(receiver, &peer, <[u8]>::to_vec).unwrap();
        let (cek, nonce) = derive(&secret, auth, ua_public, as_public, salt).unwrap();
        let key = aead::LessSafeKey::new(aead::UnboundKey::new(&aead::AES_128_GCM, &cek).unwrap());
        let plain = key
            .open_in_place(
                aead::Nonce::assume_unique_for_key(nonce),
                aead::Aad::empty(),
                &mut record,
            )
            .unwrap();
        // Strip padding: trailing zeros, then the 0x02 delimiter.
        let end = plain.iter().rposition(|&b| b != 0).unwrap();
        assert_eq!(plain[end], 2);
        plain[..end].to_vec()
    }

    #[test]
    fn encrypt_round_trip() {
        let rng = SystemRandom::new();
        let receiver =
            agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let ua_public = receiver.compute_public_key().unwrap().as_ref().to_vec();
        let mut auth = [0u8; 16];
        rng.fill(&mut auth).unwrap();

        let message = b"You have a new message";
        let body = encrypt(&b64(&ua_public), &b64(auth), message).unwrap();
        assert_eq!(body.len(), 86 + message.len() + 1 + 16);
        assert_eq!(decrypt(receiver, &ua_public, &auth, &body), message);
    }

    #[test]
    fn rejects_bad_keys() {
        assert!(encrypt("AAAA", &b64([0; 16]), b"x").is_err());
        assert!(encrypt("!!", "!!", b"x").is_err());
        let mut point = vec![4u8];
        point.extend_from_slice(&[1; 64]); // Not on the curve.
        assert!(encrypt(&b64(&point), &b64([0; 16]), b"x").is_err());
    }

    fn check_jwt(vapid: &Vapid, header: &str) -> serde_json::Value {
        let rest = header.strip_prefix("vapid t=").unwrap();
        let (jwt, k) = rest.split_once(", k=").unwrap();
        assert_eq!(k, vapid.public_key());
        let (signing_input, sig) = jwt.rsplit_once('.').unwrap();
        let public = URL_SAFE_NO_PAD.decode(k).unwrap();
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, &public)
            .verify(
                signing_input.as_bytes(),
                &URL_SAFE_NO_PAD.decode(sig).unwrap(),
            )
            .unwrap();
        let (h, c) = signing_input.split_once('.').unwrap();
        let h: serde_json::Value =
            serde_json::from_slice(&URL_SAFE_NO_PAD.decode(h).unwrap()).unwrap();
        assert_eq!(h["alg"], "ES256");
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(c).unwrap()).unwrap()
    }

    #[test]
    fn vapid_persist_and_sign() {
        let vapid = Vapid::generate().unwrap();
        let again = Vapid::from_pkcs8(vapid.pkcs8()).unwrap();
        assert_eq!(vapid.public_key(), again.public_key());
        assert_eq!(
            URL_SAFE_NO_PAD.decode(vapid.public_key()).unwrap().len(),
            65
        );

        let auth = again
            .authorization("https://push.example.net", "mailto:me@example.org", 1234)
            .unwrap();
        let claims = check_jwt(&vapid, &auth);
        assert_eq!(claims["aud"], "https://push.example.net");
        assert_eq!(claims["sub"], "mailto:me@example.org");
        assert_eq!(claims["exp"], 1234);
    }

    #[test]
    fn build_request() {
        let rng = SystemRandom::new();
        let receiver =
            agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).unwrap();
        let ua_public = receiver.compute_public_key().unwrap().as_ref().to_vec();
        let auth = [7u8; 16];
        let sub = serde_json::json!({
            "endpoint": "https://Push.Example.net:8443/wpush/v2/abc",
            "keys": { "p256dh": b64(&ua_public), "auth": b64(auth) },
        });
        let vapid = Vapid::generate().unwrap();
        let req = request(&sub, b"hi", &vapid, "mailto:a@b.c", 1000).unwrap();
        assert_eq!(req.method(), http::Method::POST);
        assert_eq!(req.headers()["content-encoding"], "aes128gcm");
        assert_eq!(req.headers()["content-type"], "application/octet-stream");
        assert!(req.headers().contains_key("ttl"));
        let claims = check_jwt(&vapid, req.headers()["authorization"].to_str().unwrap());
        assert_eq!(claims["aud"], "https://push.example.net:8443");
        assert_eq!(claims["exp"], 1000 + VAPID_LIFETIME);
        assert_eq!(decrypt(receiver, &ua_public, &auth, req.body()), b"hi");

        assert!(request(&serde_json::json!({}), b"", &vapid, "x", 0).is_err());
        let mut plain = sub.clone();
        plain["endpoint"] = "http://push.example.net/x".into();
        assert!(request(&plain, b"", &vapid, "x", 0).is_err());
    }
}
