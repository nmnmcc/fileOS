// SPDX-License-Identifier: AGPL-3.0-only

//! Core formats of homeserver.
//!
//! Everything a person keeps is a resource, and every change to a resource is
//! an event in the signed, append-only log of a space. This crate defines the
//! bytes: deterministic CBOR, `COSE_Sign1`, content hashes, and the proposal,
//! entry and head formats of a log. It knows nothing about keys or storage;
//! signing and verification live in `hs-identity`.
//!
//! The formats are meant to stay readable for twenty years with nothing but
//! this description and a CBOR library.

pub mod base32;
pub mod cbor;
pub mod cose;
pub mod event;

use std::fmt;

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// A BLAKE3 hash: the name of every immutable thing.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Default)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// The all-zero hash, used where there is no previous value.
    pub const ZERO: Hash = Hash([0; 32]);

    /// Hash some bytes.
    pub fn of(bytes: &[u8]) -> Hash {
        Hash(*blake3::hash(bytes).as_bytes())
    }

    /// Lowercase hexadecimal, as used in URLs and file names.
    pub fn to_hex(&self) -> String {
        data_encoding::HEXLOWER.encode(&self.0)
    }

    /// Parse hexadecimal.
    pub fn from_hex(s: &str) -> Option<Hash> {
        let b = data_encoding::HEXLOWER_PERMISSIVE
            .decode(s.as_bytes())
            .ok()?;
        Hash::from_slice(&b)
    }

    /// Take a hash from a 32-byte slice.
    pub fn from_slice(b: &[u8]) -> Option<Hash> {
        Some(Hash(b.try_into().ok()?))
    }
}

impl fmt::Display for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

impl fmt::Debug for Hash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Hash({})", &self.to_hex()[..12])
    }
}

/// A public key (Ed25519), 32 bytes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PublicKey(pub [u8; 32]);

impl PublicKey {
    /// The text form used in URLs: lowercase base32, 52 characters.
    pub fn to_id(&self) -> String {
        base32::encode(&self.0)
    }

    /// Parse the text form.
    pub fn from_id(s: &str) -> Option<PublicKey> {
        Some(PublicKey(base32::decode(s)?.try_into().ok()?))
    }

    /// Take a key from a 32-byte slice.
    pub fn from_slice(b: &[u8]) -> Option<PublicKey> {
        Some(PublicKey(b.try_into().ok()?))
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_id())
    }
}

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", &self.to_id()[..10])
    }
}

/// An error in a format.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FormatError(pub String);

impl fmt::Display for FormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for FormatError {}

impl From<cbor::Error> for FormatError {
    fn from(e: cbor::Error) -> Self {
        FormatError(e.to_string())
    }
}

/// Compare two byte strings in time that depends only on their lengths.
pub fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A resource path inside a space: `type/id`, both made of URL-safe characters.
pub fn valid_target(target: &str) -> bool {
    let mut parts = target.split('/');
    let ok = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.len() <= 128
                && p.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.~".contains(&c))
                && p != "."
                && p != ".."
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trip() {
        let h = Hash::of(b"hello");
        assert_eq!(Hash::from_hex(&h.to_hex()), Some(h));
        assert_eq!(
            PublicKey::from_id(&PublicKey(h.0).to_id()),
            Some(PublicKey(h.0))
        );
    }

    #[test]
    fn targets() {
        assert!(valid_target("note/abc-1"));
        assert!(!valid_target("note"));
        assert!(!valid_target("note/../x"));
        assert!(!valid_target("note/a/b"));
        assert!(!valid_target("note/.."));
    }
}
