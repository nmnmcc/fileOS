// SPDX-License-Identifier: AGPL-3.0-only

//! PKCE (RFC 7636), with the `S256` method only.

use crate::{b64, sha256};

/// The challenge for a verifier: base64url of its SHA-256.
pub fn challenge(verifier: &str) -> String {
    b64::encode(&sha256(verifier.as_bytes()))
}

/// Says if `verifier` is good, and goes with `challenge`.
pub fn check(verifier: &str, challenge: &str) -> bool {
    let good = (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~".contains(&b));
    good && self::challenge(verifier) == challenge
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_example_of_rfc_7636() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert!(check(
            verifier,
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        ));
        assert!(!check("short", &challenge("short")));
    }
}
