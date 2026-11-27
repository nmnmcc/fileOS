// SPDX-License-Identifier: AGPL-3.0-only

//! The formats at the bottom of fileOS: hashes, keys, signatures and CBOR.
//!
//! This crate is portable: it builds for `wasm32-unknown-unknown` and has no
//! threads, files, sockets or clocks.

use std::fmt;
use std::str::FromStr;

use ed25519_dalek::{Signer as _, Verifier as _};
use serde::de::{self, DeserializeOwned};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A BLAKE3 hash. Blobs, entries and folders are named by their hash.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Hash(pub [u8; 32]);

impl Hash {
    /// The hash of `data`.
    pub fn of(data: &[u8]) -> Self {
        Self(*blake3::hash(data).as_bytes())
    }

    /// The hash of the CBOR form of `value`.
    pub fn of_value<T: Serialize>(value: &T) -> Self {
        Self::of(&encode(value))
    }
}

/// An ed25519 public key. People, devices, nodes and apps are all keys.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PublicKey(pub [u8; 32]);

impl PublicKey {
    /// Says if `signature` is a signature of `message` made with this key.
    pub fn verify(&self, message: &[u8], signature: &Signature) -> bool {
        let Ok(key) = ed25519_dalek::VerifyingKey::from_bytes(&self.0) else {
            return false;
        };
        key.verify(message, &ed25519_dalek::Signature::from_bytes(&signature.0))
            .is_ok()
    }
}

/// An ed25519 secret key.
#[derive(Clone)]
pub struct SecretKey(ed25519_dalek::SigningKey);

impl SecretKey {
    /// The key for a seed of 32 random bytes. The caller gets the bytes; this
    /// crate has no source of randomness.
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self(ed25519_dalek::SigningKey::from_bytes(seed))
    }

    /// The seed that this key was made from.
    pub fn seed(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    /// A key for `purpose` that comes from this key and always gives the same
    /// result. A node uses it to give each app its own key.
    #[must_use]
    pub fn derive(&self, purpose: &str) -> Self {
        let seed = blake3::derive_key(
            "fileOS 2026 derived key",
            &[&self.seed()[..], purpose.as_bytes()].concat(),
        );
        Self::from_seed(&seed)
    }

    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.verifying_key().to_bytes())
    }

    pub fn sign(&self, message: &[u8]) -> Signature {
        Signature(self.0.sign(message).to_bytes())
    }
}

impl fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SecretKey({})", self.public())
    }
}

/// An ed25519 signature.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature(pub [u8; 64]);

/// Bytes that are kept as a CBOR byte string, not as a list of numbers.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Bytes(pub Vec<u8>);

impl fmt::Debug for Bytes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Bytes({})", self.0.len())
    }
}

impl Serialize for Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bytes(&self.0)
    }
}

impl<'de> Deserialize<'de> for Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl de::Visitor<'_> for Visit {
            type Value = Bytes;
            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a byte string")
            }
            fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<Bytes, E> {
                Ok(Bytes(v.to_vec()))
            }
            fn visit_byte_buf<E: de::Error>(self, v: Vec<u8>) -> Result<Bytes, E> {
                Ok(Bytes(v))
            }
        }
        deserializer.deserialize_byte_buf(Visit)
    }
}

/// The CBOR form of `value`.
///
/// # Panics
///
/// Never for the types in fileOS: writing to a `Vec` cannot fail, and every
/// type here has a CBOR form.
pub fn encode<T: Serialize>(value: &T) -> Vec<u8> {
    cbor4ii::serde::to_vec(Vec::new(), value).expect("every fileOS type has a CBOR form")
}

/// Reads a value from its CBOR form.
///
/// # Errors
///
/// When the bytes are not a CBOR form of `T`.
pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Error> {
    cbor4ii::serde::from_slice(bytes)
        .map_err(|e| Error::invalid(format!("bad CBOR: {e}")).with_source(e))
}

/// An operation that could not finish.
#[derive(Debug)]
pub struct Error {
    message: String,
    kind: Kind,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
    backtrace: std::backtrace::Backtrace,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Invalid,
    Denied,
    NotFound,
    Unavailable,
    Internal,
}

impl Error {
    fn new(kind: Kind, message: String) -> Self {
        Self {
            message,
            kind,
            source: None,
            backtrace: std::backtrace::Backtrace::capture(),
        }
    }

    /// Bad input or a request that cannot be carried out as given.
    pub fn invalid(message: String) -> Self {
        Self::new(Kind::Invalid, message)
    }

    /// The caller lacks the right to do this.
    pub fn denied(message: String) -> Self {
        Self::new(Kind::Denied, message)
    }

    /// The requested item does not exist.
    pub fn not_found(message: String) -> Self {
        Self::new(Kind::NotFound, message)
    }

    /// A service or some data is not ready now.
    pub fn unavailable(message: String) -> Self {
        Self::new(Kind::Unavailable, message)
    }

    /// A failure in the service that handles the request.
    pub fn internal(message: String) -> Self {
        Self::new(Kind::Internal, message)
    }

    /// Adds where the error happened, keeping its cause and kind.
    #[must_use]
    pub fn context(mut self, message: impl fmt::Display) -> Self {
        self.message = format!("{message}: {}", self.message);
        self
    }

    /// Keeps a lower-level cause for diagnostics.
    #[must_use]
    pub fn with_source(
        mut self,
        cause: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        self.source = Some(cause.into());
        self
    }

    pub fn is_invalid(&self) -> bool {
        self.kind == Kind::Invalid
    }

    pub fn is_denied(&self) -> bool {
        self.kind == Kind::Denied
    }

    pub fn is_not_found(&self) -> bool {
        self.kind == Kind::NotFound
    }

    pub fn is_unavailable(&self) -> bool {
        self.kind == Kind::Unavailable
    }

    pub fn backtrace(&self) -> &std::backtrace::Backtrace {
        &self.backtrace
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::internal(error.to_string()).with_source(error)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)?;
        if f.alternate() && self.backtrace.status() == std::backtrace::BacktraceStatus::Captured {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source.as_deref().map(|error| error as _)
    }
}

// Hex text and serde for the fixed-size byte types.

fn write_hex(f: &mut fmt::Formatter<'_>, bytes: &[u8]) -> fmt::Result {
    for b in bytes {
        write!(f, "{b:02x}")?;
    }
    Ok(())
}

fn read_hex<const N: usize>(s: &str) -> Result<[u8; N], Error> {
    let bad = || Error::invalid(format!("not {} hex characters: {s:?}", N * 2));
    if s.len() != N * 2 || !s.is_ascii() {
        return Err(bad());
    }
    let mut out = [0; N];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .ok()
            .ok_or_else(bad)?;
    }
    Ok(out)
}

macro_rules! byte_type {
    ($name:ident, $len:literal) => {
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write_hex(f, &self.0)
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), "("))?;
                write_hex(f, &self.0[..4])?;
                f.write_str("…)")
            }
        }

        impl FromStr for $name {
            type Err = Error;
            fn from_str(text: &str) -> Result<Self, Error> {
                read_hex::<$len>(text).map(Self)
            }
        }

        impl TryFrom<&[u8]> for $name {
            type Error = Error;
            fn try_from(bytes: &[u8]) -> Result<Self, Error> {
                bytes.try_into().map(Self).map_err(|_| {
                    Error::invalid(format!(
                        concat!("a ", stringify!($name), " has {} bytes, not {}"),
                        $len,
                        bytes.len()
                    ))
                })
            }
        }

        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.serialize_bytes(&self.0)
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                struct Visit;
                impl de::Visitor<'_> for Visit {
                    type Value = $name;
                    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                        write!(formatter, "{} bytes", $len)
                    }
                    fn visit_bytes<E: de::Error>(self, v: &[u8]) -> Result<$name, E> {
                        $name::try_from(v).map_err(E::custom)
                    }
                }
                deserializer.deserialize_bytes(Visit)
            }
        }
    };
}

byte_type!(Hash, 32);
byte_type!(PublicKey, 32);
byte_type!(Signature, 64);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sign_and_verify() {
        let key = SecretKey::from_seed(&[7; 32]);
        let sig = key.sign(b"hello");
        assert!(key.public().verify(b"hello", &sig));
        assert!(!key.public().verify(b"hullo", &sig));
        assert_ne!(key.derive("app").public(), key.public());
        assert_eq!(key.derive("app").public(), key.derive("app").public());
    }

    #[test]
    fn hex_and_cbor() {
        let h = Hash::of(b"x");
        assert_eq!(h.to_string().parse::<Hash>().unwrap(), h);
        let sig = SecretKey::from_seed(&[1; 32]).sign(b"m");
        let back: (Hash, Signature) = decode(&encode(&(h, sig))).unwrap();
        assert_eq!(back, (h, sig));
    }
}
