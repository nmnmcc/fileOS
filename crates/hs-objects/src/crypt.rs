// SPDX-License-Identifier: AGPL-3.0-only

//! Encrypt replica objects.
//!
//! Every object that leaves a person's nodes is encrypted with AES-256-GCM
//! under a key derived from the storage key and the object's name, and stored
//! under a keyed hash of its name, so the holder sees neither the contents nor
//! which objects are the same.

use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};

const MAGIC: &[u8; 4] = b"HSE1";

/// The secret that protects a person's replicas.
pub struct StorageKey([u8; 32]);

impl std::fmt::Debug for StorageKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageKey").finish_non_exhaustive()
    }
}

impl StorageKey {
    /// Wrap 32 secret bytes.
    pub fn new(bytes: [u8; 32]) -> StorageKey {
        StorageKey(bytes)
    }

    fn derive(&self, context: &str, name: &str) -> [u8; 32] {
        let mut h = blake3::Hasher::new_keyed(&self.0);
        h.update(context.as_bytes());
        h.update(&[0]);
        h.update(name.as_bytes());
        *h.finalize().as_bytes()
    }

    /// The name an object is stored under on an untrusted replica.
    pub fn opaque_name(&self, name: &str) -> String {
        let h = self.derive("hs name v1", name);
        let hex = hs_core::Hash(h).to_hex();
        format!("{}/{}", &hex[..2], &hex[2..])
    }

    fn key(&self, name: &str) -> LessSafeKey {
        let k = self.derive("hs key v1", name);
        // A 32-byte key always fits AES-256.

        LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &k).expect("32-byte key"))
    }

    /// Encrypt. `nonce` must be random (12 bytes); the name is authenticated,
    /// so an object cannot be swapped for another.
    pub fn seal(&self, name: &str, plaintext: &[u8], nonce: [u8; 12]) -> Vec<u8> {
        let mut buf = plaintext.to_vec();
        let tag = self.key(name).seal_in_place_separate_tag(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(name.as_bytes()),
            &mut buf,
        );
        let mut out = Vec::with_capacity(4 + 12 + buf.len() + 16);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&buf);
        if let Ok(tag) = tag {
            out.extend_from_slice(tag.as_ref());
        }
        out
    }

    /// Decrypt, or `None` if the object was changed or is not ours.
    pub fn open(&self, name: &str, sealed: &[u8]) -> Option<Vec<u8>> {
        if sealed.len() < 4 + 12 + 16 || &sealed[..4] != MAGIC {
            return None;
        }
        let nonce: [u8; 12] = sealed[4..16].try_into().ok()?;
        let mut buf = sealed[16..].to_vec();
        let plain = self
            .key(name)
            .open_in_place(
                Nonce::assume_unique_for_key(nonce),
                Aad::from(name.as_bytes()),
                &mut buf,
            )
            .ok()?;
        Some(plain.to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_storage_key() {
        let key = StorageKey::new([179; 32]);
        let rendered = format!("{key:?}");
        assert!(rendered.contains("StorageKey"));
        assert!(!rendered.contains("179"));
    }

    #[test]
    fn seal_open() {
        let k = StorageKey::new([5; 32]);
        let sealed = k.seal("blobs/ab/cd", b"secret photo", [1; 12]);
        assert_eq!(k.open("blobs/ab/cd", &sealed).unwrap(), b"secret photo");
        assert!(k.open("blobs/ab/ce", &sealed).is_none());
        let mut bad = sealed.clone();
        bad[20] ^= 1;
        assert!(k.open("blobs/ab/cd", &bad).is_none());
        assert!(
            StorageKey::new([6; 32])
                .open("blobs/ab/cd", &sealed)
                .is_none()
        );
        assert_ne!(k.opaque_name("a"), k.opaque_name("b"));
        assert_ne!(
            k.opaque_name("a"),
            StorageKey::new([6; 32]).opaque_name("a")
        );
    }
}
