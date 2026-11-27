// SPDX-License-Identifier: AGPL-3.0-only

//! Passkeys (WebAuthn Level 2): registering one, and checking a sign-in.
//!
//! The relying party is the kernel at `fileos.localhost`. Keys may be P-256
//! (`ES256`, COSE algorithm -7) or Ed25519 (`EdDSA`, -8). Attestation is not
//! asked for: a passkey is registered only with a one-time setup code that
//! the owner of the node made, so who made the key does not matter.

use cbor4ii::core::Value;
use ring::signature::{ECDSA_P256_SHA256_ASN1, ED25519, UnparsedPublicKey};
use serde::{Deserialize, Serialize};

use crate::{Error, b64, sha256};

/// A passkey that was registered.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Passkey {
    /// The credential ID, as the browser gives it.
    pub id: fileos_core::Bytes,
    /// `-7` for P-256, `-8` for Ed25519.
    pub alg: i32,
    /// The public key: an uncompressed P-256 point, or 32 Ed25519 bytes.
    pub public: fileos_core::Bytes,
    /// The highest signature counter seen, to see cloned keys.
    pub count: u32,
}

#[derive(Deserialize)]
struct ClientData {
    #[serde(rename = "type")]
    kind: String,
    challenge: String,
    origin: String,
    #[serde(rename = "crossOrigin", default)]
    cross_origin: bool,
}

/// User present.
const UP: u8 = 0x01;
/// User verified (with a PIN, a face or a finger).
const UV: u8 = 0x04;
/// Attested credential data is there.
const AT: u8 = 0x40;

fn bad(why: &str) -> Error {
    Error::invalid(format!("a passkey answer that is not good: {why}"))
}

fn check_client(json: &[u8], kind: &str, challenge: &[u8], origin: &str) -> Result<(), Error> {
    let c: ClientData = serde_json::from_slice(json).map_err(|_e| bad("the client data"))?;
    if c.kind != kind || c.cross_origin {
        return Err(bad("the type"));
    }
    if b64::decode(&c.challenge).as_deref() != Some(challenge) {
        return Err(bad("the challenge"));
    }
    if c.origin != origin {
        return Err(bad("the origin"));
    }
    Ok(())
}

/// Checks the first 37 bytes of authenticator data: the hash of the
/// relying party, the flags (present and verified), and the counter.
fn check_auth_data(data: &[u8], rp_id: &str) -> Result<(u8, u32), Error> {
    if data.len() < 37 {
        return Err(bad("the authenticator data"));
    }
    if data[..32] != sha256(rp_id.as_bytes()) {
        return Err(bad("the relying party"));
    }
    let flags = data[32];
    if flags & UP == 0 || flags & UV == 0 {
        return Err(bad("the person was not there or not verified"));
    }
    let count = u32::from_be_bytes(data[33..37].try_into().expect("4 bytes"));
    Ok((flags, count))
}

fn map_get(map: &[(Value, Value)], key: i128) -> Option<&Value> {
    map.iter()
        .find(|(k, _)| matches!(k, Value::Integer(i) if *i == key))
        .map(|(_, v)| v)
}

fn bytes(v: Option<&Value>) -> Option<&[u8]> {
    match v? {
        Value::Bytes(b) => Some(b),
        _ => None,
    }
}

/// Reads a COSE key into an algorithm and public key bytes.
fn cose_key(data: &[u8]) -> Result<(i32, Vec<u8>), Error> {
    let mut reader = cbor4ii::core::utils::SliceReader::new(data);
    let key =
        <Value as cbor4ii::core::dec::Decode>::decode(&mut reader).map_err(|_e| bad("the key"))?;
    let Value::Map(map) = key else {
        return Err(bad("the key"));
    };
    let int = |k| match map_get(&map, k) {
        Some(Value::Integer(i)) => Some(*i),
        _ => None,
    };
    match (int(1), int(3), int(-1)) {
        // EC2, ES256, P-256.
        (Some(2), Some(-7), Some(1)) => {
            let (x, y) = (bytes(map_get(&map, -2)), bytes(map_get(&map, -3)));
            match (x, y) {
                (Some(x), Some(y)) if x.len() == 32 && y.len() == 32 => {
                    let mut point = vec![4];
                    point.extend_from_slice(x);
                    point.extend_from_slice(y);
                    Ok((-7, point))
                }
                _ => Err(bad("the P-256 key")),
            }
        }
        // OKP, EdDSA, Ed25519.
        (Some(1), Some(-8), Some(6)) => match bytes(map_get(&map, -2)) {
            Some(x) if x.len() == 32 => Ok((-8, x.to_vec())),
            _ => Err(bad("the Ed25519 key")),
        },
        _ => Err(bad("a kind of key that is not taken")),
    }
}

/// Registers a passkey from what `navigator.credentials.create` gave:
/// `clientDataJSON` and `attestationObject`.
///
/// # Errors
///
/// When the answer is not for this challenge, origin and relying party, or
/// the key is not P-256 or Ed25519.
pub fn register(
    client_data: &[u8],
    attestation: &[u8],
    challenge: &[u8],
    origin: &str,
    rp_id: &str,
) -> Result<Passkey, Error> {
    check_client(client_data, "webauthn.create", challenge, origin)?;
    let object: Value = fileos_core::decode(attestation).map_err(|_e| bad("the attestation"))?;
    let Value::Map(object) = object else {
        return Err(bad("the attestation"));
    };
    let data = object
        .iter()
        .find(|(k, _)| matches!(k, Value::Text(t) if t == "authData"))
        .and_then(|(_, v)| match v {
            Value::Bytes(b) => Some(b.as_slice()),
            _ => None,
        })
        .ok_or_else(|| bad("the authenticator data"))?;
    let (flags, count) = check_auth_data(data, rp_id)?;
    if flags & AT == 0 || data.len() < 55 {
        return Err(bad("no key in the answer"));
    }
    // 16 bytes of AAGUID, then the length of the credential ID.
    let id_len = usize::from(u16::from_be_bytes([data[53], data[54]]));
    let id = data
        .get(55..55 + id_len)
        .ok_or_else(|| bad("the credential ID"))?;
    let (alg, public) = cose_key(&data[55 + id_len..])?;
    Ok(Passkey {
        id: fileos_core::Bytes(id.to_vec()),
        alg,
        public: fileos_core::Bytes(public),
        count,
    })
}

/// Checks a sign-in from what `navigator.credentials.get` gave:
/// `clientDataJSON`, `authenticatorData` and `signature`. Gives back the new
/// counter, to keep.
///
/// # Errors
///
/// When the answer is not good, or the counter went back (a cloned key).
pub fn verify(
    key: &Passkey,
    client_data: &[u8],
    auth_data: &[u8],
    signature: &[u8],
    challenge: &[u8],
    origin: &str,
    rp_id: &str,
) -> Result<u32, Error> {
    check_client(client_data, "webauthn.get", challenge, origin)?;
    let (_, count) = check_auth_data(auth_data, rp_id)?;
    let mut message = auth_data.to_vec();
    message.extend_from_slice(&sha256(client_data));
    let alg: &dyn ring::signature::VerificationAlgorithm = match key.alg {
        -7 => &ECDSA_P256_SHA256_ASN1,
        -8 => &ED25519,
        _ => return Err(bad("a kind of key that is not taken")),
    };
    UnparsedPublicKey::new(alg, &key.public.0)
        .verify(&message, signature)
        .map_err(|_e| bad("the signature"))?;
    // Keys that keep no counter always give 0.
    if (count != 0 || key.count != 0) && count <= key.count {
        return Err(bad("the counter went back: the key may be a copy"));
    }
    Ok(count)
}

/// A test authenticator, as a browser and a passkey do it.
#[cfg(any(test, feature = "testing"))]
#[expect(
    missing_debug_implementations,
    clippy::new_without_default,
    clippy::missing_panics_doc,
    reason = "for tests only"
)]
pub mod testing {
    use cbor4ii::core::Value;
    use ring::rand::SystemRandom;
    use ring::signature::{ECDSA_P256_SHA256_ASN1_SIGNING, EcdsaKeyPair, KeyPair as _};

    use crate::{b64, sha256};

    pub struct Authenticator {
        key: EcdsaKeyPair,
        pub id: Vec<u8>,
        pub count: u32,
    }

    pub struct Answer {
        pub client_data: Vec<u8>,
        pub auth_data: Vec<u8>,
        pub signature: Vec<u8>,
    }

    impl Authenticator {
        pub fn new() -> Self {
            let rng = SystemRandom::new();
            let pkcs8 =
                EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
            Self {
                key: EcdsaKeyPair::from_pkcs8(
                    &ECDSA_P256_SHA256_ASN1_SIGNING,
                    pkcs8.as_ref(),
                    &rng,
                )
                .unwrap(),
                id: b"credential-1".to_vec(),
                count: 0,
            }
        }

        fn client(kind: &str, challenge: &[u8], origin: &str) -> Vec<u8> {
            format!(
                r#"{{"type":"{kind}","challenge":"{}","origin":"{origin}","crossOrigin":false}}"#,
                b64::encode(challenge)
            )
            .into_bytes()
        }

        fn auth(&mut self, rp_id: &str, flags: u8) -> Vec<u8> {
            self.count += 1;
            let mut d = sha256(rp_id.as_bytes()).to_vec();
            d.push(flags);
            d.extend(self.count.to_be_bytes());
            d
        }

        /// What `navigator.credentials.create` gives.
        pub fn create(
            &mut self,
            challenge: &[u8],
            origin: &str,
            rp_id: &str,
        ) -> (Vec<u8>, Vec<u8>) {
            let mut d = self.auth(rp_id, 0x45);
            d.extend([0; 16]);
            d.extend((self.id.len() as u16).to_be_bytes());
            d.extend(&self.id);
            let p = self.key.public_key().as_ref();
            let cose = Value::Map(vec![
                (Value::Integer(1), Value::Integer(2)),
                (Value::Integer(3), Value::Integer(-7)),
                (Value::Integer(-1), Value::Integer(1)),
                (Value::Integer(-2), Value::Bytes(p[1..33].to_vec())),
                (Value::Integer(-3), Value::Bytes(p[33..65].to_vec())),
            ]);
            d.extend(fileos_core::encode(&cose));
            let object = Value::Map(vec![
                (Value::Text("fmt".into()), Value::Text("none".into())),
                (Value::Text("attStmt".into()), Value::Map(Vec::new())),
                (Value::Text("authData".into()), Value::Bytes(d)),
            ]);
            (
                Self::client("webauthn.create", challenge, origin),
                fileos_core::encode(&object),
            )
        }

        /// What `navigator.credentials.get` gives.
        pub fn get(&mut self, challenge: &[u8], origin: &str, rp_id: &str) -> Answer {
            let client_data = Self::client("webauthn.get", challenge, origin);
            let auth_data = self.auth(rp_id, 0x05);
            let mut message = auth_data.clone();
            message.extend(sha256(&client_data));
            let signature = self
                .key
                .sign(&SystemRandom::new(), &message)
                .unwrap()
                .as_ref()
                .to_vec();
            Answer {
                client_data,
                auth_data,
                signature,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::Authenticator;
    use super::*;

    const ORIGIN: &str = "http://fileos.localhost:7341";
    const RP: &str = "fileos.localhost";

    #[test]
    fn register_and_sign_in() {
        let mut a = Authenticator::new();
        let (client, object) = a.create(b"c1", ORIGIN, RP);
        register(&client, &object, b"c2", ORIGIN, RP).unwrap_err();
        register(&client, &object, b"c1", "http://evil.localhost", RP).unwrap_err();
        let mut key = register(&client, &object, b"c1", ORIGIN, RP).unwrap();
        assert_eq!(key.alg, -7);

        let ans = a.get(b"c3", ORIGIN, RP);
        let count = verify(
            &key,
            &ans.client_data,
            &ans.auth_data,
            &ans.signature,
            b"c3",
            ORIGIN,
            RP,
        )
        .unwrap();
        key.count = count;
        // The same answer again: the counter did not go up.
        verify(
            &key,
            &ans.client_data,
            &ans.auth_data,
            &ans.signature,
            b"c3",
            ORIGIN,
            RP,
        )
        .unwrap_err();
        // Another challenge, origin or a changed signature.
        let ans = a.get(b"c4", ORIGIN, RP);
        verify(
            &key,
            &ans.client_data,
            &ans.auth_data,
            &ans.signature,
            b"c5",
            ORIGIN,
            RP,
        )
        .unwrap_err();
        let mut sig = ans.signature.clone();
        let last = sig.len() - 1;
        sig[last] ^= 1;
        verify(
            &key,
            &ans.client_data,
            &ans.auth_data,
            &sig,
            b"c4",
            ORIGIN,
            RP,
        )
        .unwrap_err();
        verify(
            &key,
            &ans.client_data,
            &ans.auth_data,
            &ans.signature,
            b"c4",
            ORIGIN,
            RP,
        )
        .unwrap();
    }
}
