// SPDX-License-Identifier: AGPL-3.0-only

//! Identity for homeserver.
//!
//! A person is a key pair; the identity ID is derived from the first public
//! key and never changes. A short signed, append-only identity log declares the
//! current personal key, node keys, addresses and domain aliases. Anyone who
//! holds the log can verify it without asking a third party.
//!
//! This crate is sans-IO: callers supply randomness (seeds) and time.

pub mod did;
mod log;
pub mod shamir;

use hs_core::cose::Sign1;
use hs_core::{PublicKey, base32};
use ring::signature::{self, Ed25519KeyPair, KeyPair};

#[doc(inline)]
pub use log::{IdentityLog, IdentityState, LogEntry, LogKind};

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// An Ed25519 key pair, kept as its 32-byte seed.
pub struct Keypair {
    seed: [u8; 32],
    pair: Ed25519KeyPair,
}

impl std::fmt::Debug for Keypair {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keypair")
            .field("public", &self.public())
            .finish_non_exhaustive()
    }
}

impl Keypair {
    /// Derive a key pair from a 32-byte seed. Callers get seeds from a secure
    /// random source (`hs-node` uses `ring::rand::SystemRandom`).
    ///
    /// # Panics
    ///
    /// Panics if the signing backend rejects a correctly sized Ed25519 seed.
    pub fn from_seed(seed: [u8; 32]) -> Keypair {
        // The seed is exactly 32 bytes, which is the only way this can fail.

        let pair = Ed25519KeyPair::from_seed_unchecked(&seed).expect("32-byte seed");
        Keypair { seed, pair }
    }

    /// The seed, for storing the key.
    pub fn seed(&self) -> &[u8; 32] {
        &self.seed
    }

    /// The public key.
    pub fn public(&self) -> PublicKey {
        let mut pk = [0u8; 32];
        pk.copy_from_slice(self.pair.public_key().as_ref());
        PublicKey(pk)
    }

    /// Sign a message.
    pub fn sign(&self, message: &[u8]) -> Vec<u8> {
        self.pair.sign(message).as_ref().to_vec()
    }

    /// Wrap a payload in a `COSE_Sign1` signed by this key.
    pub fn sign1(&self, payload: Vec<u8>) -> Sign1 {
        let signature = self.sign(&Sign1::to_be_signed(&payload));
        Sign1 {
            kid: self.public().0.to_vec(),
            payload,
            signature,
        }
    }
}

/// Verify an Ed25519 signature.
pub fn verify(key: &PublicKey, message: &[u8], sig: &[u8]) -> bool {
    signature::UnparsedPublicKey::new(&signature::ED25519, key.0)
        .verify(message, sig)
        .is_ok()
}

/// Verify a `COSE_Sign1` and return its signer.
pub fn verify1(msg: &Sign1) -> Option<PublicKey> {
    let key = PublicKey::from_slice(&msg.kid)?;
    verify(&key, &Sign1::to_be_signed(&msg.payload), &msg.signature).then_some(key)
}

/// Format a 32-byte secret as a recovery code: base32 in groups of four.
pub fn recovery_code(seed: &[u8; 32]) -> String {
    let text = base32::encode(seed);
    text.as_bytes()
        .chunks(4)
        .map(|c| std::str::from_utf8(c).unwrap_or(""))
        .collect::<Vec<_>>()
        .join("-")
}

/// Parse a recovery code back into the seed. Dashes and spaces are ignored.
pub fn parse_recovery_code(code: &str) -> Option<[u8; 32]> {
    let clean: String = code.chars().filter(char::is_ascii_alphanumeric).collect();
    base32::decode(&clean)?.try_into().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_the_key_seed() {
        let secret = [179; 32];
        let rendered = format!("{:?}", Keypair::from_seed(secret));
        assert!(rendered.contains("Keypair"));
        assert!(!rendered.contains("179"));
        assert!(!rendered.contains("seed"));
    }

    #[test]
    fn sign_and_verify() {
        let k = Keypair::from_seed([7; 32]);
        let msg = k.sign1(b"hello".to_vec());
        assert_eq!(verify1(&msg), Some(k.public()));
        let mut bad = msg.clone();
        bad.payload = b"hellp".to_vec();
        assert_eq!(verify1(&bad), None);
        let decoded = Sign1::decode(&msg.encode()).unwrap();
        assert_eq!(verify1(&decoded), Some(k.public()));
    }

    #[test]
    fn recovery_codes() {
        let code = recovery_code(&[9; 32]);
        assert_eq!(parse_recovery_code(&code), Some([9; 32]));
        assert_eq!(
            parse_recovery_code(&code.to_uppercase().replace('-', " ")),
            Some([9; 32])
        );
    }
}
