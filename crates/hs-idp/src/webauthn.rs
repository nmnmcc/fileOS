// SPDX-License-Identifier: AGPL-3.0-only

//! Passkeys (WebAuthn) without attestation.
//!
//! Registration gets the public key from the browser's
//! `AuthenticatorAttestationResponse.getPublicKey()` (SPKI DER) and
//! `getPublicKeyAlgorithm()`, so there is no need to parse CBOR. The supported
//! algorithms are ES256 (-7) and `EdDSA` with Ed25519 (-8).

use ring::signature;
use serde_json::{Value, json};

use crate::jose::{b64d, b64e};
use crate::{Error, Result, err, sha256};

/// COSE algorithm number for ES256.
pub const ALG_ES256: i64 = -7;
/// COSE algorithm number for `EdDSA` (Ed25519).
pub const ALG_EDDSA: i64 = -8;

const FLAG_UP: u8 = 0x01;
const FLAG_AT: u8 = 0x40;

const OID_EC_PUBLIC_KEY: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01];
const OID_P256: &[u8] = &[0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07];
const OID_ED25519: &[u8] = &[0x2b, 0x65, 0x70];

/// A new random 32-byte challenge.
pub fn challenge() -> [u8; 32] {
    crate::random()
}

/// A registered passkey.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    /// The credential ID the authenticator picked.
    pub id: Vec<u8>,
    /// COSE algorithm: [`ALG_ES256`] or [`ALG_EDDSA`].
    pub alg: i64,
    /// Uncompressed P-256 point (65 bytes) or Ed25519 key (32 bytes).
    pub public_key: Vec<u8>,
    /// The last signature counter seen.
    pub sign_count: u32,
}

impl Credential {
    /// Write as JSON, with byte fields in base64url.
    pub fn to_json(&self) -> Value {
        json!({
            "id": b64e(&self.id),
            "alg": self.alg,
            "public_key": b64e(&self.public_key),
            "sign_count": self.sign_count,
        })
    }

    /// Parse the output of [`Credential::to_json`].
    ///
    /// # Errors
    ///
    /// Returns an error if a needed credential field is missing or not valid.
    pub fn from_json(v: &Value) -> Result<Credential> {
        let s = |k: &str| {
            v.get(k)
                .and_then(Value::as_str)
                .ok_or(Error(format!("missing {k}")))
        };
        let alg = v
            .get("alg")
            .and_then(Value::as_i64)
            .ok_or(Error("missing alg".into()))?;
        let sign_count = v
            .get("sign_count")
            .and_then(Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or(Error("missing sign_count".into()))?;
        let public_key = b64d(s("public_key")?)?;
        check_key(alg, &public_key)?;
        Ok(Credential {
            id: b64d(s("id")?)?,
            alg,
            public_key,
            sign_count,
        })
    }
}

fn check_key(alg: i64, key: &[u8]) -> Result<()> {
    let ok = match alg {
        ALG_ES256 => key.len() == 65 && key[0] == 4,
        ALG_EDDSA => key.len() == 32,
        _ => return err("unsupported algorithm"),
    };
    if ok { Ok(()) } else { err("bad public key") }
}

/// Check the client data: type, challenge, origin.
fn check_client_data(json: &[u8], ty: &str, challenge: &[u8], origin: &str) -> Result<()> {
    let v: Value = serde_json::from_slice(json).map_err(|error| {
        Error(format!(
            "bad clientDataJSON at line {}, column {}",
            error.line(),
            error.column()
        ))
    })?;
    let s = |k: &str| v.get(k).and_then(Value::as_str);
    if s("type") != Some(ty) {
        return err("wrong client data type");
    }
    let got = b64d(s("challenge").ok_or(Error("missing challenge".into()))?)?;
    if !hs_core::ct_eq(&got, challenge) {
        return err("challenge mismatch");
    }
    if s("origin") != Some(origin) {
        return err("origin mismatch");
    }
    if v.get("crossOrigin").and_then(Value::as_bool) == Some(true) {
        return err("cross-origin request");
    }
    Ok(())
}

/// Check authenticator data and return its flags and sign count.
fn check_auth_data(data: &[u8], rp_id: &str) -> Result<(u8, u32)> {
    if data.len() < 37 {
        return err("authenticator data too short");
    }
    if !hs_core::ct_eq(&data[..32], &sha256(rp_id.as_bytes())) {
        return err("RP ID mismatch");
    }
    let flags = data[32];
    if flags & FLAG_UP == 0 {
        return err("user not present");
    }
    let count = u32::from_be_bytes([data[33], data[34], data[35], data[36]]);
    Ok((flags, count))
}

/// Read one DER element. Returns (tag, contents, rest).
fn der(input: &[u8]) -> Result<(u8, &[u8], &[u8])> {
    let bad = || Error("bad DER".into());
    let (&tag, rest) = input.split_first().ok_or_else(bad)?;
    let (&len0, rest) = rest.split_first().ok_or_else(bad)?;
    let (len, rest) = match len0 {
        n if n < 0x80 => (n as usize, rest),
        0x81 => {
            let (&n, rest) = rest.split_first().ok_or_else(bad)?;
            (n as usize, rest)
        }
        0x82 => {
            let b = rest.get(..2).ok_or_else(bad)?;
            (usize::from(b[0]) << 8 | usize::from(b[1]), &rest[2..])
        }
        _ => return Err(bad()),
    };
    if rest.len() < len {
        return Err(bad());
    }
    Ok((tag, &rest[..len], &rest[len..]))
}

/// Get the raw public key out of a `SubjectPublicKeyInfo` for `alg`.
///
/// # Errors
///
/// Returns an error if the key algorithm is not supported or the key encoding is badly formed.
pub fn spki_public_key(spki: &[u8], alg: i64) -> Result<Vec<u8>> {
    let (tag, body, rest) = der(spki)?;
    if tag != 0x30 || !rest.is_empty() {
        return err("SPKI is not a sequence");
    }
    let (tag, algid, rest) = der(body)?;
    if tag != 0x30 {
        return err("bad SPKI algorithm");
    }
    let (tag, bits, rest) = der(rest)?;
    if tag != 0x03 || !rest.is_empty() {
        return err("bad SPKI key");
    }
    let (tag, oid, params) = der(algid)?;
    if tag != 0x06 {
        return err("bad SPKI algorithm");
    }
    match alg {
        ALG_ES256 => {
            let (tag, curve, rest) = der(params)?;
            if oid != OID_EC_PUBLIC_KEY || tag != 0x06 || curve != OID_P256 || !rest.is_empty() {
                return err("SPKI is not a P-256 key");
            }
        }
        ALG_EDDSA => {
            if oid != OID_ED25519 || !params.is_empty() {
                return err("SPKI is not an Ed25519 key");
            }
        }
        _ => return err("unsupported algorithm"),
    }
    let Some((0, key)) = bits.split_first() else {
        return err("bad SPKI bit string");
    };
    check_key(alg, key)?;
    Ok(key.to_vec())
}

/// Check a registration (`navigator.credentials.create`) response.
///
/// # Errors
///
/// Returns an error when client or authenticator data is badly formed or does not agree with the
/// expected origin, relying party or challenge.
#[expect(
    clippy::too_many_arguments,
    reason = "the verifier takes the expected ceremony values explicitly"
)]
pub fn verify_registration(
    rp_id: &str,
    origin: &str,
    expected_challenge: &[u8],
    client_data_json: &[u8],
    authenticator_data: &[u8],
    spki_der: &[u8],
    alg: i64,
    credential_id: &[u8],
) -> Result<Credential> {
    check_client_data(
        client_data_json,
        "webauthn.create",
        expected_challenge,
        origin,
    )?;
    let (flags, sign_count) = check_auth_data(authenticator_data, rp_id)?;
    if flags & FLAG_AT == 0 {
        return err("no attested credential data");
    }
    // Attested credential data: AAGUID (16), ID length (2), ID.
    let id_len = authenticator_data
        .get(53..55)
        .map(|b| usize::from(b[0]) << 8 | usize::from(b[1]))
        .ok_or(Error("authenticator data too short".into()))?;
    if credential_id.is_empty() || authenticator_data.get(55..55 + id_len) != Some(credential_id) {
        return err("credential ID mismatch");
    }
    let public_key = spki_public_key(spki_der, alg)?;
    Ok(Credential {
        id: credential_id.to_vec(),
        alg,
        public_key,
        sign_count,
    })
}

/// Check an authentication (`navigator.credentials.get`) response and return
/// the new sign count to store.
///
/// # Errors
///
/// Returns an error for client data or a signature that is not valid, or for a sign counter that
/// goes against the credential's counter rules.
#[expect(
    clippy::map_err_ignore,
    reason = "ring reports authentication failure as Unspecified"
)]
pub fn verify_assertion(
    rp_id: &str,
    origin: &str,
    expected_challenge: &[u8],
    credential: &Credential,
    client_data_json: &[u8],
    authenticator_data: &[u8],
    signature: &[u8],
) -> Result<u32> {
    check_client_data(client_data_json, "webauthn.get", expected_challenge, origin)?;
    let (_, count) = check_auth_data(authenticator_data, rp_id)?;
    let alg: &dyn signature::VerificationAlgorithm = match credential.alg {
        ALG_ES256 => &signature::ECDSA_P256_SHA256_ASN1,
        ALG_EDDSA => &signature::ED25519,
        _ => return err("unsupported algorithm"),
    };
    let mut msg = authenticator_data.to_vec();
    msg.extend_from_slice(&sha256(client_data_json));
    signature::UnparsedPublicKey::new(alg, &credential.public_key)
        .verify(&msg, signature)
        .map_err(|_| Error("bad signature".into()))?;
    if (count != 0 || credential.sign_count != 0) && count <= credential.sign_count {
        return err("sign count did not go up; the authenticator may be a copy");
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::rand::SystemRandom;
    use ring::signature::{
        ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, Ed25519KeyPair, KeyPair as _,
    };

    const RP: &str = "example.com";
    const ORIGIN: &str = "https://example.com";

    enum Key {
        Ec(EcdsaKeyPair),
        Ed(Ed25519KeyPair),
    }

    impl Key {
        fn new(alg: i64) -> Key {
            let rng = SystemRandom::new();
            if alg == ALG_ES256 {
                let doc =
                    EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
                Key::Ec(
                    EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, doc.as_ref(), &rng)
                        .unwrap(),
                )
            } else {
                let doc = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
                Key::Ed(Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap())
            }
        }

        fn spki(&self) -> Vec<u8> {
            let (algid, key): (Vec<u8>, &[u8]) = match self {
                Key::Ec(k) => {
                    let mut a = vec![0x06, 7];
                    a.extend_from_slice(OID_EC_PUBLIC_KEY);
                    a.extend_from_slice(&[0x06, 8]);
                    a.extend_from_slice(OID_P256);
                    (a, k.public_key().as_ref())
                }
                Key::Ed(k) => {
                    let mut a = vec![0x06, 3];
                    a.extend_from_slice(OID_ED25519);
                    (a, k.public_key().as_ref())
                }
            };
            let mut body = vec![0x30, algid.len() as u8];
            body.extend_from_slice(&algid);
            body.extend_from_slice(&[0x03, key.len() as u8 + 1, 0]);
            body.extend_from_slice(key);
            let mut out = vec![0x30, body.len() as u8];
            out.extend_from_slice(&body);
            out
        }

        fn sign(&self, msg: &[u8]) -> Vec<u8> {
            match self {
                Key::Ec(k) => k.sign(&SystemRandom::new(), msg).unwrap().as_ref().to_vec(),
                Key::Ed(k) => k.sign(msg).as_ref().to_vec(),
            }
        }
    }

    fn client_data(ty: &str, challenge: &[u8], origin: &str) -> Vec<u8> {
        serde_json::to_vec(&json!({"type": ty, "challenge": b64e(challenge), "origin": origin}))
            .unwrap()
    }

    fn auth_data(rp: &str, flags: u8, count: u32, cred_id: Option<&[u8]>) -> Vec<u8> {
        let mut d = sha256(rp.as_bytes()).to_vec();
        d.push(flags);
        d.extend_from_slice(&count.to_be_bytes());
        if let Some(id) = cred_id {
            d.extend_from_slice(&[0u8; 16]);
            d.extend_from_slice(&(id.len() as u16).to_be_bytes());
            d.extend_from_slice(id);
            d.extend_from_slice(b"\xa0"); // stands in for the COSE key
        }
        d
    }

    fn register(alg: i64) -> (Key, Credential) {
        let key = Key::new(alg);
        let ch = challenge();
        let id = b"cred-1";
        let cred = verify_registration(
            RP,
            ORIGIN,
            &ch,
            &client_data("webauthn.create", &ch, ORIGIN),
            &auth_data(RP, FLAG_UP | FLAG_AT, 0, Some(id)),
            &key.spki(),
            alg,
            id,
        )
        .unwrap();
        assert_eq!(cred.id, id);
        assert_eq!(Credential::from_json(&cred.to_json()).unwrap(), cred);
        (key, cred)
    }

    fn assert_with(
        key: &Key,
        cred: &Credential,
        count: u32,
        origin: &str,
        ch_sent: &[u8],
    ) -> Result<u32> {
        let ch = [9u8; 32];
        let cd = client_data("webauthn.get", ch_sent, origin);
        let ad = auth_data(RP, FLAG_UP, count, None);
        let mut msg = ad.clone();
        msg.extend_from_slice(&sha256(&cd));
        verify_assertion(RP, ORIGIN, &ch, cred, &cd, &ad, &key.sign(&msg))
    }

    #[test]
    fn round_trip_both_algorithms() {
        for alg in [ALG_ES256, ALG_EDDSA] {
            let (key, mut cred) = register(alg);
            // Authenticators that never count keep zero.
            assert_eq!(assert_with(&key, &cred, 0, ORIGIN, &[9; 32]).unwrap(), 0);
            cred.sign_count = assert_with(&key, &cred, 5, ORIGIN, &[9; 32]).unwrap();
            assert_eq!(cred.sign_count, 5);
            // A counter that is used again, or is old.
            assert_with(&key, &cred, 5, ORIGIN, &[9; 32]).unwrap_err();
            assert_with(&key, &cred, 0, ORIGIN, &[9; 32]).unwrap_err();
            assert_eq!(assert_with(&key, &cred, 6, ORIGIN, &[9; 32]).unwrap(), 6);
        }
    }

    #[test]
    fn assertion_rejections() {
        let (key, cred) = register(ALG_ES256);
        assert_with(&key, &cred, 1, "https://evil.example", &[9; 32]).unwrap_err();
        assert_with(&key, &cred, 1, ORIGIN, &[8; 32]).unwrap_err();
        // Signed by another key.
        let other = Key::new(ALG_ES256);
        assert_with(&other, &cred, 1, ORIGIN, &[9; 32]).unwrap_err();
        // A signature that is not a signature, and the wrong type.
        let ch = [9u8; 32];
        let ad = auth_data(RP, FLAG_UP, 1, None);
        let cd = client_data("webauthn.get", &ch, ORIGIN);
        verify_assertion(RP, ORIGIN, &ch, &cred, &cd, &ad, b"nope").unwrap_err();
        let cd = client_data("webauthn.create", &ch, ORIGIN);
        let mut msg = ad.clone();
        msg.extend_from_slice(&sha256(&cd));
        verify_assertion(RP, ORIGIN, &ch, &cred, &cd, &ad, &key.sign(&msg)).unwrap_err();
        // User not present.
        let ad = auth_data(RP, 0, 1, None);
        let cd = client_data("webauthn.get", &ch, ORIGIN);
        let mut msg = ad.clone();
        msg.extend_from_slice(&sha256(&cd));
        verify_assertion(RP, ORIGIN, &ch, &cred, &cd, &ad, &key.sign(&msg)).unwrap_err();
    }

    #[test]
    fn registration_rejections() {
        let key = Key::new(ALG_EDDSA);
        let ch = challenge();
        let id: &[u8] = b"id";
        let good_cd = client_data("webauthn.create", &ch, ORIGIN);
        let good_ad = auth_data(RP, FLAG_UP | FLAG_AT, 0, Some(id));
        let reg = |cd: &[u8], ad: &[u8], spki: &[u8], alg: i64, cid: &[u8]| {
            verify_registration(RP, ORIGIN, &ch, cd, ad, spki, alg, cid)
        };
        reg(&good_cd, &good_ad, &key.spki(), ALG_EDDSA, id).unwrap();
        let wrong_origin = client_data("webauthn.create", &ch, "https://evil.example");
        reg(&wrong_origin, &good_ad, &key.spki(), ALG_EDDSA, id).unwrap_err();
        let wrong_ch = client_data("webauthn.create", &[0; 32], ORIGIN);
        reg(&wrong_ch, &good_ad, &key.spki(), ALG_EDDSA, id).unwrap_err();
        let wrong_rp = auth_data("evil.example", FLAG_UP | FLAG_AT, 0, Some(id));
        reg(&good_cd, &wrong_rp, &key.spki(), ALG_EDDSA, id).unwrap_err();
        reg(&good_cd, &good_ad, &key.spki(), ALG_ES256, id).unwrap_err();
        reg(&good_cd, &good_ad, &key.spki(), ALG_EDDSA, b"other").unwrap_err();
        reg(&good_cd, &good_ad[..40], &key.spki(), ALG_EDDSA, id).unwrap_err();
        reg(b"{", &good_ad, &key.spki(), ALG_EDDSA, id).unwrap_err();
        let spki = key.spki();
        for n in 0..spki.len() {
            reg(&good_cd, &good_ad, &spki[..n], ALG_EDDSA, id).unwrap_err();
        }
    }
}
