// SPDX-License-Identifier: AGPL-3.0-only

//! Lowercase base32 (RFC 4648 alphabet, no padding), used in URLs and file names.

use std::sync::LazyLock;

use data_encoding::{Encoding, Specification};

static LOWER: LazyLock<Encoding> = LazyLock::new(|| {
    let mut spec = Specification::new();
    spec.symbols.push_str("abcdefghijklmnopqrstuvwxyz234567");
    // Accept uppercase too; leftover bits must be zero, so each value has
    // one encoding.
    spec.translate.from.push_str("ABCDEFGHIJKLMNOPQRSTUVWXYZ");
    spec.translate.to.push_str("abcdefghijklmnopqrstuvwxyz");
    spec.check_trailing_bits = true;

    spec.encoding().expect("a valid base32 specification")
});

/// Encode bytes as lowercase base32 without padding.
pub fn encode(bytes: &[u8]) -> String {
    LOWER.encode(bytes)
}

/// Decode lowercase or uppercase base32 without padding.
pub fn decode(text: &str) -> Option<Vec<u8>> {
    LOWER.decode(text.as_bytes()).ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn rfc4648_vectors() {
        for (raw, enc) in [
            ("", ""),
            ("f", "my"),
            ("fo", "mzxq"),
            ("foo", "mzxw6"),
            ("foob", "mzxw6yq"),
            ("fooba", "mzxw6ytb"),
            ("foobar", "mzxw6ytboi"),
        ] {
            assert_eq!(super::encode(raw.as_bytes()), enc);
            assert_eq!(super::decode(enc).unwrap(), raw.as_bytes());
            assert_eq!(super::decode(&enc.to_uppercase()).unwrap(), raw.as_bytes());
        }
        assert!(super::decode("mz").is_none());
        assert!(super::decode("mz======").is_none());
    }
}
