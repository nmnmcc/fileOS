// SPDX-License-Identifier: AGPL-3.0-only

//! `OpenID` Connect provider, passkeys (WebAuthn) and forward auth.
//!
//! Everything here is pure logic: no HTTP server and no I/O. Callers pass the
//! current time in as unix seconds.

pub mod forward;
pub mod jose;
pub mod oidc;
pub mod webauthn;

use std::fmt;

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// An error with a human-readable reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// The result type of this crate.
pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(Error(msg.into()))
}

/// Fill an array with bytes from the system random number generator.
///
/// # Panics
///
/// Panics if the operating system cannot supply random bytes.
pub fn random<const N: usize>() -> [u8; N] {
    ring::rand::generate(&ring::rand::SystemRandom::new())
        .map(ring::rand::Random::expose)
        .expect("system random number generator failed")
}

/// SHA-256 of `data`.
pub(crate) fn sha256(data: &[u8]) -> [u8; 32] {
    let d = ring::digest::digest(&ring::digest::SHA256, data);
    let mut out = [0u8; 32];
    out.copy_from_slice(d.as_ref());
    out
}

/// Lowercase hex encoding.
pub(crate) fn hex(data: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(data)
}

/// Everything but the RFC 3986 unreserved characters.
const COMPONENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Percent-encode everything except RFC 3986 unreserved characters.
pub fn percent_encode(s: &str) -> String {
    percent_encoding::utf8_percent_encode(s, COMPONENT).to_string()
}

/// Decode a form-encoded query component: `+` becomes a space and `%XX` a
/// byte. A `%` not followed by two hex digits is an error, as is a result
/// that is not UTF-8.
///
/// # Errors
///
/// Returns an error if an escape is malformed or the decoded bytes are not UTF-8.
pub fn percent_decode(s: &str) -> Result<String> {
    let b = s.as_bytes();
    let malformed = b.iter().enumerate().any(|(i, &c)| {
        c == b'%'
            && !b
                .get(i + 1..i + 3)
                .is_some_and(|h| h.iter().all(u8::is_ascii_hexdigit))
    });
    if malformed {
        return Err(Error("malformed percent escape".into()));
    }
    let s = s.replace('+', " ");
    percent_encoding::percent_decode_str(&s)
        .decode_utf8()
        .map(std::borrow::Cow::into_owned)
        .map_err(|error| Error(format!("query is not UTF-8: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_escapes() {
        for bad in ["%", "%z", "%zz", "a%4", "%%41"] {
            assert!(percent_decode(bad).is_err(), "{bad}");
        }
        assert_eq!(percent_decode("a%41+b").unwrap(), "aA b");
    }

    #[test]
    fn percent_round_trip() {
        let s = "https://a.example/x?y=1&z=é ~";
        assert_eq!(percent_decode(&percent_encode(s)).unwrap(), s);
        assert_eq!(percent_decode("a+b%20c").unwrap(), "a b c");
        percent_decode("%ff").unwrap_err();
    }

    #[test]
    fn hex_works() {
        assert_eq!(hex(&[0, 0xab, 0x10]), "00ab10");
    }
}
