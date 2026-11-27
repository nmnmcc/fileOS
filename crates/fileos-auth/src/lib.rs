// SPDX-License-Identifier: AGPL-3.0-only

//! Sign-in for people, with no I/O and no clock: the caller gives the time.
//!
//! The kernel does all sign-in, with standard protocols, and apps never see
//! passwords, sessions or keys (see the docs page "HTTP and the browser"):
//!
//! - [`webauthn`]: passkeys (WebAuthn Level 2), to show that the person is
//!   there.
//! - [`pkce`]: OAuth 2.0 PKCE (RFC 7636), so that only the app that asked
//!   gets the token.
//! - [`dpop`]: `DPoP` proofs (RFC 9449), so that a token is good only with a
//!   key that the browser cannot give out.
//! - [`token`]: access tokens, signed by the node key.
//! - [`Auth`]: the state of the authorization server: passkeys, codes,
//!   refresh tokens that change at each use, and take-backs (RFC 7009).

pub mod b64;
pub mod dpop;
pub mod pkce;
pub mod token;
pub mod webauthn;

mod server;

pub use server::{Auth, AuthState, Authorize, Tokens};

/// An error, with a message for people. It never says more than the caller
/// should know: a failed sign-in does not say which check failed to the
/// browser, only to the log.
pub use fileos_core::Error;

/// SHA-256.
///
/// # Panics
///
/// Never: SHA-256 gives 32 bytes.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let d = ring::digest::digest(&ring::digest::SHA256, data);
    d.as_ref().try_into().expect("SHA-256 has 32 bytes")
}
