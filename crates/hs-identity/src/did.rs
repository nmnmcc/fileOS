// SPDX-License-Identifier: AGPL-3.0-only

//! The identity log as a W3C DID document, so other software can resolve it
//! with standard tools. The method name may still change (it is an open
//! question on the architecture page).

use serde_json::{Value, json};

use crate::IdentityState;

/// The DID of an identity.
pub fn did(state: &IdentityState) -> String {
    format!("did:hs:{}", state.id.to_id())
}

/// Encode an Ed25519 public key as a Multikey (`z` + base58btc of 0xed01 + key).
pub fn multikey(key: &[u8; 32]) -> String {
    let mut bytes = vec![0xed, 0x01];
    bytes.extend_from_slice(key);
    format!("z{}", base58(&bytes))
}

fn base58(input: &[u8]) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut digits: Vec<u8> = Vec::new();
    for &byte in input {
        let mut carry = u32::from(byte);
        for d in &mut digits {
            carry += u32::from(*d) << 8;
            *d = (carry % 58) as u8;
            carry /= 58;
        }
        while carry > 0 {
            digits.push((carry % 58) as u8);
            carry /= 58;
        }
    }
    let zeros = input.iter().take_while(|&&b| b == 0).count();
    let mut out = "1".repeat(zeros);
    out.extend(digits.iter().rev().map(|&d| ALPHABET[d as usize] as char));
    out
}

/// Build the DID document.
pub fn document(state: &IdentityState) -> Value {
    let id = did(state);
    let mut methods = vec![json!({
        "id": format!("{id}#personal"),
        "type": "Multikey",
        "controller": id,
        "publicKeyMultibase": multikey(&state.personal.0),
    })];
    for (i, node) in state.nodes.iter().enumerate() {
        methods.push(json!({
            "id": format!("{id}#node-{i}"),
            "type": "Multikey",
            "controller": id,
            "publicKeyMultibase": multikey(&node.0),
        }));
    }
    let services: Vec<Value> = state
        .addresses
        .iter()
        .enumerate()
        .map(|(i, url)| json!({"id": format!("{id}#hs-{i}"), "type": "Homeserver", "serviceEndpoint": url}))
        .collect();
    json!({
        "@context": ["https://www.w3.org/ns/did/v1", "https://w3id.org/security/multikey/v1"],
        "id": id,
        "alsoKnownAs": state.aliases.iter().map(|d| format!("https://{d}")).collect::<Vec<_>>(),
        "verificationMethod": methods,
        "authentication": [format!("{id}#personal")],
        "assertionMethod": [format!("{id}#personal")],
        "service": services,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn base58_vectors() {
        assert_eq!(super::base58(b"Hello World!"), "2NEpo7TZRRrLZSi2U");
        assert_eq!(super::base58(&[0, 0, 1]), "112");
    }
}
