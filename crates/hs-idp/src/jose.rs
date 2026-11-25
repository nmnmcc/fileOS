// SPDX-License-Identifier: AGPL-3.0-only

//! JOSE: base64url, signing keys, compact JWS, JWK and JWKS.
//!
//! Supports ES256 (ECDSA P-256 with SHA-256) and `EdDSA` (Ed25519).

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use ring::rand::SystemRandom;
use ring::signature::{self, EcdsaKeyPair, Ed25519KeyPair, KeyPair as _};
use serde_json::{Map, Value, json};

use crate::{Error, Result, err, sha256};

/// Encode bytes as base64url without padding.
pub fn b64e(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

/// Decode base64url without padding.
///
/// # Errors
///
/// Returns an error for an invalid base64url string.
pub fn b64d(s: &str) -> Result<Vec<u8>> {
    URL_SAFE_NO_PAD
        .decode(s)
        .map_err(|_| Error("bad base64url".into()))
}

/// A JWS signature algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alg {
    /// ECDSA on P-256 with SHA-256.
    Es256,
    /// Ed25519.
    EdDsa,
}

impl Alg {
    /// The JOSE name: `ES256` or `EdDSA`.
    pub fn name(self) -> &'static str {
        match self {
            Alg::Es256 => "ES256",
            Alg::EdDsa => "EdDSA",
        }
    }

    /// Parse a JOSE algorithm name.
    pub fn from_name(name: &str) -> Option<Alg> {
        match name {
            "ES256" => Some(Alg::Es256),
            "EdDSA" => Some(Alg::EdDsa),
            _ => None,
        }
    }
}

/// PKCS#8 v1 prefix for an Ed25519 private key; the 32-byte seed follows.
const ED25519_PKCS8_PREFIX: [u8; 16] = [
    0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20,
];

/// A private signing key. Persist it with [`SigningKey::to_bytes`].
pub enum SigningKey {
    /// An ES256 key and its PKCS#8 document.
    Es256 {
        /// PKCS#8 encoding of the key.
        pkcs8: Vec<u8>,
        /// The parsed key pair.
        pair: EcdsaKeyPair,
    },
    /// An Ed25519 key and its PKCS#8 document.
    EdDsa {
        /// PKCS#8 encoding of the key.
        pkcs8: Vec<u8>,
        /// The parsed key pair.
        pair: Ed25519KeyPair,
    },
}

impl std::fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SigningKey({}, kid={})", self.alg().name(), self.kid())
    }
}

impl SigningKey {
    /// Generate a new ES256 key.
    ///
    /// # Errors
    ///
    /// Returns an error if the key cannot be generated or parsed by the signing backend.
    pub fn generate_es256() -> Result<SigningKey> {
        let rng = SystemRandom::new();
        let doc = EcdsaKeyPair::generate_pkcs8(&signature::ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
            .map_err(|_| Error("key generation failed".into()))?;
        SigningKey::es256_from_pkcs8(doc.as_ref())
    }

    /// Generate a new Ed25519 key.
    ///
    /// # Errors
    ///
    /// Returns an error if the generated seed cannot be loaded by the signing backend.
    pub fn generate_eddsa() -> Result<SigningKey> {
        SigningKey::eddsa_from_seed(&crate::random::<32>())
    }

    /// Load an ES256 key from PKCS#8.
    ///
    /// # Errors
    ///
    /// Returns an error if the PKCS #8 data is malformed or describes an incompatible key.
    pub fn es256_from_pkcs8(pkcs8: &[u8]) -> Result<SigningKey> {
        let pair = EcdsaKeyPair::from_pkcs8(
            &signature::ECDSA_P256_SHA256_FIXED_SIGNING,
            pkcs8,
            &SystemRandom::new(),
        )
        .map_err(|e| Error(format!("bad ES256 key: {e}")))?;
        Ok(SigningKey::Es256 {
            pkcs8: pkcs8.to_vec(),
            pair,
        })
    }

    /// Load an Ed25519 key from its 32-byte seed.
    ///
    /// # Errors
    ///
    /// Returns an error if the signing backend rejects the Ed25519 seed.
    pub fn eddsa_from_seed(seed: &[u8; 32]) -> Result<SigningKey> {
        let mut pkcs8 = ED25519_PKCS8_PREFIX.to_vec();
        pkcs8.extend_from_slice(seed);
        SigningKey::eddsa_from_pkcs8(&pkcs8)
    }

    /// Load an Ed25519 key from PKCS#8 (v1 or v2).
    ///
    /// # Errors
    ///
    /// Returns an error if the PKCS #8 data is malformed or describes an incompatible key.
    pub fn eddsa_from_pkcs8(pkcs8: &[u8]) -> Result<SigningKey> {
        let pair = Ed25519KeyPair::from_pkcs8_maybe_unchecked(pkcs8)
            .map_err(|e| Error(format!("bad Ed25519 key: {e}")))?;
        Ok(SigningKey::EdDsa {
            pkcs8: pkcs8.to_vec(),
            pair,
        })
    }

    /// The PKCS#8 encoding of the key, for storage.
    pub fn to_bytes(&self) -> Vec<u8> {
        match self {
            SigningKey::Es256 { pkcs8, .. } | SigningKey::EdDsa { pkcs8, .. } => pkcs8.clone(),
        }
    }

    /// Load a key stored with [`SigningKey::to_bytes`], whatever its algorithm.
    ///
    /// # Errors
    ///
    /// Returns an error if the stored algorithm or key bytes are invalid.
    pub fn from_bytes(pkcs8: &[u8]) -> Result<SigningKey> {
        SigningKey::es256_from_pkcs8(pkcs8)
            .or_else(|_| SigningKey::eddsa_from_pkcs8(pkcs8))
            .map_err(|_| Error("unrecognized signing key".into()))
    }

    /// The key's algorithm.
    pub fn alg(&self) -> Alg {
        match self {
            SigningKey::Es256 { .. } => Alg::Es256,
            SigningKey::EdDsa { .. } => Alg::EdDsa,
        }
    }

    /// The public half of the key.
    pub fn public_key(&self) -> PublicKey {
        let raw = match self {
            SigningKey::Es256 { pair, .. } => pair.public_key().as_ref().to_vec(),
            SigningKey::EdDsa { pair, .. } => pair.public_key().as_ref().to_vec(),
        };
        PublicKey {
            alg: self.alg(),
            raw,
        }
    }

    /// The key ID: the RFC 7638 thumbprint of the public JWK.
    pub fn kid(&self) -> String {
        self.public_key().thumbprint()
    }

    /// Sign raw bytes. ES256 signatures are the fixed 64-byte `r || s` form.
    ///
    /// # Errors
    ///
    /// Returns an error if the signing backend cannot produce a signature.
    pub fn sign_bytes(&self, msg: &[u8]) -> Result<Vec<u8>> {
        match self {
            SigningKey::Es256 { pair, .. } => pair
                .sign(&SystemRandom::new(), msg)
                .map(|s| s.as_ref().to_vec())
                .map_err(|_| Error("signing failed".into())),
            SigningKey::EdDsa { pair, .. } => Ok(pair.sign(msg).as_ref().to_vec()),
        }
    }

    /// Create a compact JWS. The header gets `alg` and `kid`, plus `typ: JWT`
    /// unless `header_extra` (an object, or null) sets other fields.
    ///
    /// # Errors
    ///
    /// Returns an error if the payload cannot be serialized or the signing operation fails.
    pub fn sign(&self, header_extra: &Value, claims: &Value) -> Result<String> {
        let mut header = Map::new();
        header.insert("typ".into(), "JWT".into());
        if let Some(extra) = header_extra.as_object() {
            for (k, v) in extra {
                header.insert(k.clone(), v.clone());
            }
        } else if !header_extra.is_null() {
            return err("header must be an object");
        }
        header.insert("alg".into(), self.alg().name().into());
        header.insert("kid".into(), self.kid().into());
        let header =
            serde_json::to_vec(&Value::Object(header)).map_err(|e| Error(e.to_string()))?;
        let claims = serde_json::to_vec(claims).map_err(|e| Error(e.to_string()))?;
        let input = format!("{}.{}", b64e(&header), b64e(&claims));
        let sig = self.sign_bytes(input.as_bytes())?;
        Ok(format!("{input}.{}", b64e(&sig)))
    }
}

/// A public verification key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKey {
    /// The algorithm.
    pub alg: Alg,
    /// Uncompressed P-256 point (65 bytes) or Ed25519 key (32 bytes).
    pub raw: Vec<u8>,
}

impl PublicKey {
    /// Build a key from raw bytes, checking the length for the algorithm.
    ///
    /// # Errors
    ///
    /// Returns an error if the key has the wrong encoding or length for the algorithm.
    pub fn new(alg: Alg, raw: &[u8]) -> Result<PublicKey> {
        let ok = match alg {
            Alg::Es256 => raw.len() == 65 && raw[0] == 4,
            Alg::EdDsa => raw.len() == 32,
        };
        if !ok {
            return err("bad public key length");
        }
        Ok(PublicKey {
            alg,
            raw: raw.to_vec(),
        })
    }

    fn members(&self) -> Vec<(&'static str, String)> {
        match self.alg {
            Alg::Es256 => vec![
                ("crv", "P-256".into()),
                ("kty", "EC".into()),
                ("x", b64e(&self.raw[1..33])),
                ("y", b64e(&self.raw[33..65])),
            ],
            Alg::EdDsa => vec![
                ("crv", "Ed25519".into()),
                ("kty", "OKP".into()),
                ("x", b64e(&self.raw)),
            ],
        }
    }

    /// The RFC 7638 JWK thumbprint (SHA-256, base64url).
    pub fn thumbprint(&self) -> String {
        // Required members in lexicographic order, no whitespace. Values are
        // fixed strings or base64url, so no JSON escaping is needed.
        let body: Vec<String> = self
            .members()
            .iter()
            .map(|(k, v)| format!("\"{k}\":\"{v}\""))
            .collect();
        b64e(&sha256(format!("{{{}}}", body.join(",")).as_bytes()))
    }

    /// The public JWK, with `kid`, `alg` and `use: sig`.
    pub fn to_jwk(&self) -> Value {
        let mut m = Map::new();
        for (k, v) in self.members() {
            m.insert(k.into(), v.into());
        }
        m.insert("kid".into(), self.thumbprint().into());
        m.insert("alg".into(), self.alg.name().into());
        m.insert("use".into(), "sig".into());
        Value::Object(m)
    }

    /// Parse a public JWK (EC P-256 or OKP Ed25519).
    ///
    /// # Errors
    ///
    /// Returns an error if a required JWK field is invalid or the key type is unsupported.
    pub fn from_jwk(jwk: &Value) -> Result<PublicKey> {
        let s = |k: &str| jwk.get(k).and_then(Value::as_str);
        match (s("kty"), s("crv")) {
            (Some("EC"), Some("P-256")) => {
                let x = b64d(s("x").ok_or(Error("missing x".into()))?)?;
                let y = b64d(s("y").ok_or(Error("missing y".into()))?)?;
                if x.len() != 32 || y.len() != 32 {
                    return err("bad EC coordinates");
                }
                let mut raw = vec![4];
                raw.extend_from_slice(&x);
                raw.extend_from_slice(&y);
                PublicKey::new(Alg::Es256, &raw)
            }
            (Some("OKP"), Some("Ed25519")) => {
                PublicKey::new(Alg::EdDsa, &b64d(s("x").ok_or(Error("missing x".into()))?)?)
            }
            _ => err("unsupported JWK"),
        }
    }

    /// Verify a JWS signature (fixed `r || s` for ES256) over `msg`.
    pub fn verify_bytes(&self, msg: &[u8], sig: &[u8]) -> bool {
        let alg: &dyn signature::VerificationAlgorithm = match self.alg {
            Alg::Es256 => &signature::ECDSA_P256_SHA256_FIXED,
            Alg::EdDsa => &signature::ED25519,
        };
        signature::UnparsedPublicKey::new(alg, &self.raw)
            .verify(msg, sig)
            .is_ok()
    }
}

/// Build a JWKS document from public keys.
pub fn jwks(keys: &[PublicKey]) -> Value {
    json!({ "keys": keys.iter().map(PublicKey::to_jwk).collect::<Vec<_>>() })
}

/// Verify a compact JWS and return its header and claims.
///
/// The header's `alg` must match the key, and a `kid`, if present, must be the
/// key's thumbprint. Claims are not checked; see [`crate::oidc`] for that.
///
/// # Errors
///
/// Returns an error if the JWS is malformed, conflicts with the expected key, or has an invalid
/// signature.
pub fn verify_full(token: &str, key: &PublicKey) -> Result<(Value, Value)> {
    let mut parts = token.split('.');
    let (Some(h), Some(c), Some(s), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return err("malformed JWS");
    };
    let header: Value =
        serde_json::from_slice(&b64d(h)?).map_err(|_| Error("bad JWS header".into()))?;
    if header.get("alg").and_then(Value::as_str) != Some(key.alg.name()) {
        return err("JWS algorithm mismatch");
    }
    if let Some(kid) = header.get("kid")
        && kid.as_str() != Some(key.thumbprint().as_str())
    {
        return err("JWS key ID mismatch");
    }
    if header.get("crit").is_some() {
        return err("unsupported critical JWS header");
    }
    let sig = b64d(s)?;
    let input_len = h.len() + 1 + c.len();
    if !key.verify_bytes(&token.as_bytes()[..input_len], &sig) {
        return err("bad JWS signature");
    }
    let claims: Value =
        serde_json::from_slice(&b64d(c)?).map_err(|_| Error("bad JWS payload".into()))?;
    if !claims.is_object() {
        return err("JWS payload is not an object");
    }
    Ok((header, claims))
}

/// Verify a compact JWS and return its claims.
///
/// # Errors
///
/// Returns an error if the JWS is malformed or its signature cannot be verified with the key.
pub fn verify(token: &str, key: &PublicKey) -> Result<Value> {
    verify_full(token, key).map(|(_, claims)| claims)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(key: SigningKey) {
        let claims = json!({"sub": "alice", "n": 1});
        let token = key.sign(&json!({"typ": "at+jwt"}), &claims).unwrap();
        let pk = key.public_key();
        let (header, got) = verify_full(&token, &pk).unwrap();
        assert_eq!(got, claims);
        assert_eq!(header["typ"], "at+jwt");
        assert_eq!(header["kid"], key.kid());

        // Through JWK and persisted bytes.
        let jwk = pk.to_jwk();
        assert_eq!(PublicKey::from_jwk(&jwk).unwrap(), pk);
        assert_eq!(jwks(std::slice::from_ref(&pk))["keys"][0], jwk);
        let again = SigningKey::from_bytes(&key.to_bytes()).unwrap();
        assert_eq!(again.public_key(), pk);

        // Tampering fails.
        let mut parts: Vec<&str> = token.split('.').collect();
        let forged = b64e(br#"{"sub":"mallory","n":1}"#);
        parts[1] = &forged;
        assert!(verify(&parts.join("."), &pk).is_err());
        assert!(verify("a.b", &pk).is_err());
        assert!(verify("a.b.c.d", &pk).is_err());
        assert!(verify("!!.!!.!!", &pk).is_err());
    }

    #[test]
    fn es256_round_trip() {
        round_trip(SigningKey::generate_es256().unwrap());
    }

    #[test]
    fn eddsa_round_trip() {
        round_trip(SigningKey::generate_eddsa().unwrap());
    }

    #[test]
    fn wrong_key_or_alg_fails() {
        let a = SigningKey::generate_es256().unwrap();
        let b = SigningKey::generate_es256().unwrap();
        let e = SigningKey::generate_eddsa().unwrap();
        let t = a.sign(&Value::Null, &json!({})).unwrap();
        assert!(verify(&t, &b.public_key()).is_err());
        assert!(verify(&t, &e.public_key()).is_err());
    }

    #[test]
    fn rfc8037_ed25519_thumbprint() {
        // RFC 8037 appendix A.3.
        let x = b64d("11qYAYKxCrfVS_7TyWQHOg7hcvPapiMlrwIaaPcHURo").unwrap();
        let pk = PublicKey::new(Alg::EdDsa, &x).unwrap();
        assert_eq!(
            pk.thumbprint(),
            "kPrK_qmxVWaYVA9wwBF6Iuo3vVzz7TxHCTwXBygrS4k"
        );
    }

    #[test]
    fn seed_is_stable() {
        let seed = [7u8; 32];
        let a = SigningKey::eddsa_from_seed(&seed).unwrap();
        let b = SigningKey::eddsa_from_seed(&seed).unwrap();
        assert_eq!(a.public_key(), b.public_key());
        assert!(SigningKey::from_bytes(b"junk").is_err());
    }
}
