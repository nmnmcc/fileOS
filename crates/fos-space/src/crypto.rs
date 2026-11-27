// SPDX-License-Identifier: AGPL-3.0-only

//! Keys that follow prefixes, and the ciphers under them.
//!
//! - Each **epoch** of a space has a random root key. The key of a prefix is
//!   worked out from the key of the prefix above it, one part of the name at
//!   a time, with BLAKE3 `derive_key`: whoever has the key of `2026/` has the
//!   keys of everything under it, but not of `private/` or of the whole space
//!   (as in Cryptree and Tahoe-LAFS).
//! - Each file has a random **data key**. Its chunks are encrypted with it
//!   (XChaCha20-Poly1305), and it is kept **wrapped** with the key of the
//!   prefix the file is in.
//! - Keys go to each device with **HPKE** (RFC 9180: DHKEM X25519 with
//!   HKDF-SHA256, and ChaCha20-Poly1305), to the X25519 form of its ed25519
//!   key.
//!
//! Nothing here has a source of randomness of its own: [`Random`] is made
//! from a seed that the caller gives, so the simulation is the same every
//! time.

use std::convert::Infallible;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use fos_core::{Bytes, PublicKey, SecretKey};
use hpke::kem::X25519HkdfSha256;
use hpke::{Deserializable as _, OpModeR, OpModeS, Serializable as _};
use serde::{Deserialize, Serialize};

/// A secret key of 32 bytes.
pub type Key = [u8; 32];

/// The HPKE suite: ChaCha20-Poly1305 and HKDF-SHA256, with DHKEM X25519.
type HpkeAead = hpke::aead::ChaCha20Poly1305;
type Kdf = hpke::kdf::HkdfSha256;

/// Random bytes from a secret seed: BLAKE3 in keyed mode over a counter.
#[derive(Clone)]
pub struct Random {
    seed: Key,
    counter: u64,
}

impl std::fmt::Debug for Random {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Random")
    }
}

impl Random {
    pub fn new(seed: Key) -> Self {
        Self { seed, counter: 0 }
    }

    pub fn fill(&mut self, out: &mut [u8]) {
        let mut reader = blake3::Hasher::new_keyed(&self.seed)
            .update(&self.counter.to_le_bytes())
            .finalize_xof();
        reader.fill(out);
        self.counter += 1;
    }

    pub fn key(&mut self) -> Key {
        let mut key = [0; 32];
        self.fill(&mut key);
        key
    }
}

impl hpke::rand_core::TryRng for Random {
    type Error = Infallible;

    fn try_next_u32(&mut self) -> Result<u32, Infallible> {
        let mut b = [0; 4];
        self.fill(&mut b);
        Ok(u32::from_le_bytes(b))
    }

    fn try_next_u64(&mut self) -> Result<u64, Infallible> {
        let mut b = [0; 8];
        self.fill(&mut b);
        Ok(u64::from_le_bytes(b))
    }

    fn try_fill_bytes(&mut self, dst: &mut [u8]) -> Result<(), Infallible> {
        self.fill(dst);
        Ok(())
    }
}

impl hpke::rand_core::TryCryptoRng for Random {}

/// The prefix that a file name is in: `a/b/c.jpg` is in `a/b/`, and `c.jpg`
/// is in `""`.
pub fn parent(name: &str) -> String {
    match name.rfind('/') {
        Some(i) => name[..=i].to_string(),
        None => String::new(),
    }
}

/// The prefix of a directory that covers `prefix`: `2026/` stays, `2026/a`
/// becomes `2026/`.
pub fn dir_of(prefix: &str) -> String {
    if prefix.is_empty() || prefix.ends_with('/') {
        prefix.to_string()
    } else {
        parent(prefix)
    }
}

/// The key of the whole space in an epoch, from its root key.
fn top(root: &Key) -> Key {
    blake3::derive_key("fileOS 2026 space prefix key", root)
}

/// The key of `below`, from the key of `above`, when `below` is under it.
/// Both are directory prefixes (empty, or ending in `/`).
pub fn derive(above_key: &Key, above: &str, below: &str) -> Option<Key> {
    let rest = below.strip_prefix(above)?;
    let mut key = *above_key;
    for part in rest.split_terminator('/') {
        key = blake3::derive_key(
            "fileOS 2026 prefix key",
            &[&key[..], part.as_bytes()].concat(),
        );
    }
    Some(key)
}

/// The key of a directory prefix in an epoch, from the epoch's root key.
///
/// # Panics
///
/// Never: every prefix is under the whole space.
pub fn prefix_key(root: &Key, prefix: &str) -> Key {
    derive(&top(root), "", prefix).expect("every prefix is under the whole space")
}

/// Encrypts with XChaCha20-Poly1305.
///
/// # Panics
///
/// Never: encryption with a key of the right size does not fail.
pub fn seal(key: &Key, nonce: &[u8; 24], plaintext: &[u8], aad: &[u8]) -> Vec<u8> {
    let cipher = XChaCha20Poly1305::new(&(*key).into());
    cipher
        .encrypt(
            &XNonce::from(*nonce),
            Payload {
                msg: plaintext,
                aad,
            },
        )
        .expect("encryption does not fail")
}

/// Decrypts with XChaCha20-Poly1305. `None` when the key is wrong or the
/// bytes were changed.
pub fn open(key: &Key, nonce: &[u8; 24], ciphertext: &[u8], aad: &[u8]) -> Option<Vec<u8>> {
    let cipher = XChaCha20Poly1305::new(&(*key).into());
    cipher
        .decrypt(
            &XNonce::from(*nonce),
            Payload {
                msg: ciphertext,
                aad,
            },
        )
        .ok()
}

/// A data key, wrapped with the key of a prefix in an epoch.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Wrapped {
    pub epoch: fos_core::Hash,
    /// The directory prefix whose key wraps it.
    pub prefix: String,
    pub nonce: Bytes,
    pub sealed: Bytes,
}

fn wrap_aad(epoch: &fos_core::Hash, prefix: &str) -> Vec<u8> {
    fos_core::encode(&("fileOS wrapped key", epoch, prefix))
}

/// Wraps `data_key` with `kek`, the key of `prefix` in `epoch`.
pub fn wrap(
    kek: &Key,
    epoch: fos_core::Hash,
    prefix: &str,
    data_key: &Key,
    random: &mut Random,
) -> Wrapped {
    let mut nonce = [0; 24];
    random.fill(&mut nonce);
    Wrapped {
        epoch,
        prefix: prefix.to_string(),
        nonce: Bytes(nonce.to_vec()),
        sealed: Bytes(seal(kek, &nonce, data_key, &wrap_aad(&epoch, prefix))),
    }
}

/// Unwraps a data key with `kek`, the key of its prefix in its epoch.
pub fn unwrap(kek: &Key, wrapped: &Wrapped) -> Option<Key> {
    let nonce: [u8; 24] = wrapped.nonce.0.as_slice().try_into().ok()?;
    let key = open(
        kek,
        &nonce,
        &wrapped.sealed.0,
        &wrap_aad(&wrapped.epoch, &wrapped.prefix),
    )?;
    key.try_into().ok()
}

/// Seals `plaintext` to the device key `to` with HPKE. Gives back the
/// encapsulated key and the ciphertext. `None` when `to` is not a key.
pub fn seal_to(
    to: PublicKey,
    info: &[u8],
    plaintext: &[u8],
    random: &mut Random,
) -> Option<(Bytes, Bytes)> {
    let pk = <X25519HkdfSha256 as hpke::Kem>::PublicKey::from_bytes(&to.x25519()?).ok()?;
    let (enc, ct) = hpke::single_shot_seal_with_rng::<HpkeAead, Kdf, X25519HkdfSha256>(
        &OpModeS::Base,
        &pk,
        info,
        plaintext,
        b"",
        random,
    )
    .ok()?;
    Some((Bytes(enc.to_bytes().to_vec()), Bytes(ct)))
}

/// Opens what [`seal_to`] sealed to `me`.
pub fn open_with(me: &SecretKey, info: &[u8], enc: &[u8], ct: &[u8]) -> Option<Vec<u8>> {
    let sk = <X25519HkdfSha256 as hpke::Kem>::PrivateKey::from_bytes(&me.x25519()).ok()?;
    let enc = <X25519HkdfSha256 as hpke::Kem>::EncappedKey::from_bytes(enc).ok()?;
    hpke::single_shot_open::<HpkeAead, Kdf, X25519HkdfSha256>(
        &OpModeR::Base,
        &sk,
        &enc,
        info,
        ct,
        b"",
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_prefixes_one_way() {
        let root = [7; 32];
        let k2026 = prefix_key(&root, "2026/");
        assert_eq!(
            derive(&k2026, "2026/", "2026/05/"),
            Some(prefix_key(&root, "2026/05/"))
        );
        assert_eq!(derive(&k2026, "2026/", "private/"), None);
        assert_ne!(prefix_key(&root, ""), k2026);
        assert_eq!(parent("a/b/c.jpg"), "a/b/");
        assert_eq!(parent("c.jpg"), "");
        assert_eq!(dir_of("2026/a"), "2026/");
    }

    #[test]
    fn wrapping_and_hpke() {
        let mut random = Random::new([1; 32]);
        let kek = random.key();
        let dk = random.key();
        let epoch = fos_core::Hash::of(b"e");
        let w = wrap(&kek, epoch, "2026/", &dk, &mut random);
        assert_eq!(unwrap(&kek, &w), Some(dk));
        assert_eq!(unwrap(&random.key(), &w), None);
        let mut moved = w.clone();
        moved.prefix = "private/".into();
        assert_eq!(
            unwrap(&kek, &moved),
            None,
            "the prefix is bound to the wrap"
        );

        let me = SecretKey::from_seed(&[3; 32]);
        let (enc, ct) = seal_to(me.public(), b"info", b"secret", &mut random).unwrap();
        assert_eq!(
            open_with(&me, b"info", &enc.0, &ct.0).as_deref(),
            Some(&b"secret"[..])
        );
        let other = SecretKey::from_seed(&[4; 32]);
        assert_eq!(open_with(&other, b"info", &enc.0, &ct.0), None);
        assert_eq!(open_with(&me, b"other", &enc.0, &ct.0), None);
    }
}
