// SPDX-License-Identifier: AGPL-3.0-only

//! `DPoP` proofs (RFC 9449).
//!
//! A proof is a JWS signed by a key that the browser made and cannot give
//! out (a `WebCrypto` key that is not extractable). It names the method and
//! the URL of the request, and, with an access token, the hash of the token.
//! A token is bound to the thumbprint of the key (RFC 7638), so a stolen
//! token is of no use without the key.
//!
//! Keys may be P-256 (`ES256`) or Ed25519 (`EdDSA`).

use ring::signature::{ECDSA_P256_SHA256_FIXED, ED25519, UnparsedPublicKey};
use serde::Deserialize;

use crate::{Error, b64, sha256};

/// How far the time in a proof may be from the time now, in seconds.
pub const SKEW: u64 = 300;

#[derive(Deserialize)]
struct Header {
    typ: String,
    alg: String,
    jwk: Jwk,
}

/// A public key, as a JWK.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Jwk {
    pub kty: String,
    pub crv: String,
    pub x: String,
    #[serde(default)]
    pub y: Option<String>,
}

#[derive(Deserialize)]
struct Claims {
    htm: String,
    htu: String,
    iat: u64,
    jti: String,
    #[serde(default)]
    ath: Option<String>,
}

/// What a good proof says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proof {
    /// The thumbprint of the key (RFC 7638), base64url.
    pub jkt: String,
    /// The ID of the proof, so that it is not taken two times.
    pub jti: String,
    pub iat: u64,
}

impl Jwk {
    /// The thumbprint (RFC 7638): base64url of the SHA-256 of the members
    /// that are needed, in order, with no spaces.
    pub fn thumbprint(&self) -> String {
        let json = match &self.y {
            Some(y) => format!(
                r#"{{"crv":"{}","kty":"{}","x":"{}","y":"{}"}}"#,
                self.crv, self.kty, self.x, y
            ),
            None => format!(
                r#"{{"crv":"{}","kty":"{}","x":"{}"}}"#,
                self.crv, self.kty, self.x
            ),
        };
        b64::encode(&sha256(json.as_bytes()))
    }

    fn verify(&self, alg: &str, message: &[u8], signature: &[u8]) -> bool {
        let bytes = |s: &str| b64::decode(s).filter(|b| b.len() == 32);
        match (alg, self.kty.as_str(), self.crv.as_str()) {
            ("ES256", "EC", "P-256") => {
                let (Some(x), Some(y)) = (bytes(&self.x), self.y.as_deref().and_then(bytes)) else {
                    return false;
                };
                let mut point = vec![4];
                point.extend(x);
                point.extend(y);
                UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point)
                    .verify(message, signature)
                    .is_ok()
            }
            ("EdDSA", "OKP", "Ed25519") => bytes(&self.x).is_some_and(|x| {
                UnparsedPublicKey::new(&ED25519, x)
                    .verify(message, signature)
                    .is_ok()
            }),
            _ => false,
        }
    }
}

/// The URL that a proof names, with no query or fragment.
fn bare(url: &str) -> &str {
    url.split(['?', '#']).next().unwrap_or(url)
}

/// Checks a proof for a request of `method` to `url`, at `now` (seconds
/// since 1970). With `token`, the proof must name its hash.
///
/// # Errors
///
/// When the proof is not good. The caller must also keep the IDs of the
/// proofs it took, so that none is taken two times.
pub fn check(
    proof: &str,
    method: &str,
    url: &str,
    now: u64,
    token: Option<&str>,
) -> Result<Proof, Error> {
    let bad = |why: &str| Error::invalid(format!("a DPoP proof that is not good: {why}"));
    let mut parts = proof.split('.');
    let (Some(h), Some(c), Some(s), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad("not a JWS"));
    };
    let header: Header = b64::decode(h)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| bad("the header"))?;
    let claims: Claims = b64::decode(c)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .ok_or_else(|| bad("the claims"))?;
    let signature = b64::decode(s).ok_or_else(|| bad("the signature"))?;
    if header.typ != "dpop+jwt" {
        return Err(bad("the type"));
    }
    if !header
        .jwk
        .verify(&header.alg, format!("{h}.{c}").as_bytes(), &signature)
    {
        return Err(bad("the signature"));
    }
    if !claims.htm.eq_ignore_ascii_case(method) || bare(&claims.htu) != bare(url) {
        return Err(bad("the method or the URL"));
    }
    if claims.iat.abs_diff(now) > SKEW {
        return Err(bad("the time"));
    }
    if claims.jti.is_empty() || claims.jti.len() > 256 {
        return Err(bad("the ID"));
    }
    if let Some(token) = token
        && claims.ath.as_deref() != Some(b64::encode(&sha256(token.as_bytes())).as_str())
    {
        return Err(bad("the hash of the token"));
    }
    Ok(Proof {
        jkt: header.jwk.thumbprint(),
        jti: claims.jti,
        iat: claims.iat,
    })
}

/// Test keys that make proofs, as a browser does.
#[cfg(any(test, feature = "testing"))]
#[expect(
    missing_debug_implementations,
    clippy::new_without_default,
    clippy::missing_panics_doc,
    reason = "for tests only"
)]
pub mod testing {
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair as _};

    use crate::{b64, sha256};

    pub struct Key(EcdsaKeyPair);

    impl Key {
        pub fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
            Self(
                EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
                    .unwrap(),
            )
        }

        pub fn jwk(&self) -> String {
            let p = self.0.public_key().as_ref();
            format!(
                r#"{{"kty":"EC","crv":"P-256","x":"{}","y":"{}"}}"#,
                b64::encode(&p[1..33]),
                b64::encode(&p[33..65])
            )
        }

        pub fn jkt(&self) -> String {
            let jwk: super::Jwk = serde_json::from_str(&self.jwk()).unwrap();
            jwk.thumbprint()
        }

        pub fn proof(
            &self,
            method: &str,
            url: &str,
            iat: u64,
            jti: &str,
            token: Option<&str>,
        ) -> String {
            let header = format!(r#"{{"typ":"dpop+jwt","alg":"ES256","jwk":{}}}"#, self.jwk());
            let ath = token.map_or(String::new(), |t| {
                format!(r#","ath":"{}""#, b64::encode(&sha256(t.as_bytes())))
            });
            let claims =
                format!(r#"{{"htm":"{method}","htu":"{url}","iat":{iat},"jti":"{jti}"{ath}}}"#);
            let input = format!(
                "{}.{}",
                b64::encode(header.as_bytes()),
                b64::encode(claims.as_bytes())
            );
            let sig = self.0.sign(&SystemRandom::new(), input.as_bytes()).unwrap();
            format!("{input}.{}", b64::encode(sig.as_ref()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Key;
    use super::*;

    #[test]
    fn proofs() {
        let key = Key::new();
        let url = "http://fileos.localhost:7341/token";
        let p = key.proof("POST", url, 1000, "a", None);
        let proof = check(&p, "POST", url, 1010, None).unwrap();
        assert_eq!(proof.jkt, key.jkt());
        check(&p, "GET", url, 1010, None).unwrap_err();
        check(&p, "POST", "http://evil.localhost/token", 1010, None).unwrap_err();
        check(&p, "POST", url, 2000, None).unwrap_err();
        let with = key.proof("GET", url, 1000, "b", Some("tok"));
        check(&with, "GET", &format!("{url}?x=1"), 1000, Some("tok")).unwrap();
        check(&with, "GET", url, 1000, Some("other")).unwrap_err();
        // A proof signed by another key, with this key's JWK, is not good.
        let other = Key::new().proof("POST", url, 1000, "c", None);
        let mut parts: Vec<&str> = p.split('.').collect();
        parts[2] = other.rsplit('.').next().unwrap();
        check(&parts.join("."), "POST", url, 1000, None).unwrap_err();
    }

    #[test]
    fn the_thumbprint_of_rfc_7638() {
        // The example key of RFC 7638, section 3.1, is RSA; this is the
        // example EC key of RFC 9449, section 4.2.
        let jwk = Jwk {
            kty: "EC".into(),
            crv: "P-256".into(),
            x: "l8tFrhx-34tV3hRICRDY9zCkDlpBhF42UQUfWVAWBFs".into(),
            y: Some("9VE4jf_Ok_o64zbTTlcuNJajHmt6v9TDVrU0CdvGRDA".into()),
        };
        assert_eq!(
            jwk.thumbprint(),
            "0ZcOCORZNYy-DWpqq30jZyJGHTN0d2HglBV3uiguA4I"
        );
    }
}
