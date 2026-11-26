// SPDX-License-Identifier: AGPL-3.0-only

//! Identities: one person, many nodes.
//!
//! An identity is an ed25519 key. It is not a node: a node has its own key,
//! and an identity may have many nodes. The identity keeps one folder, made
//! with its key, and only the identity writes to it:
//!
//! - `/nodes/<node key>`: a [`NodeCert`], which says that the node is one of
//!   the nodes of the identity.
//!
//! A node certificate is signed by the identity, so it is a proof by itself:
//! anyone who has it can check it, with no log and no engine.
//!
//! A person gives their identity to others as a [`Link`].
//!
//! This crate is portable: it builds for `wasm32-unknown-unknown` and has no
//! threads, files, sockets or clocks.

use std::fmt;
use std::str::FromStr;

use fos_core::{Error, PublicKey, SecretKey, Signature, decode, encode};
use serde::{Deserialize, Serialize};

/// The version of the formats in this crate.
pub const VERSION: u64 = 1;

/// The name of the identity folder.
pub const FOLDER_NAME: &str = "identity";

/// The directory of node certificates in the identity folder.
pub const NODES: &str = "/nodes";

/// The path of the certificate of `node` in the identity folder.
pub fn node_path(node: PublicKey) -> String {
    format!("{NODES}/{node}")
}

/// A statement, signed by an identity, that a node is one of its nodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCert {
    pub version: u64,
    pub identity: PublicKey,
    pub node: PublicKey,
    pub sig: Signature,
}

/// What a node certificate signs. The first part keeps the signature from
/// being used as a signature of anything else.
fn cert_message(version: u64, identity: PublicKey, node: PublicKey) -> Vec<u8> {
    encode(&("fileOS node certificate", version, identity, node))
}

impl NodeCert {
    /// The certificate, signed by `identity`, that `node` is one of its nodes.
    pub fn sign(identity: &SecretKey, node: PublicKey) -> Self {
        let id = identity.public();
        Self {
            version: VERSION,
            identity: id,
            node,
            sig: identity.sign(&cert_message(VERSION, id, node)),
        }
    }

    /// Says if the identity signed this certificate.
    pub fn verify(&self) -> bool {
        self.version == VERSION
            && self.identity.verify(
                &cert_message(self.version, self.identity, self.node),
                &self.sig,
            )
    }

    /// The bytes of the certificate, as kept in the identity folder.
    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self)
    }

    /// Reads a certificate and checks it.
    ///
    /// # Errors
    ///
    /// When the bytes are not a certificate, or its signature is not good.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        let cert: Self = decode(bytes)?;
        if !cert.verify() {
            return Err(Error(
                "the node certificate is not signed by its identity".into(),
            ));
        }
        Ok(cert)
    }
}

/// An identity as people give it to each other:
/// `fos:<identity>?r=<multiaddr>&r=<multiaddr>`.
///
/// The `r` values are hints: addresses of nodes of the identity, or of
/// rendezvous nodes. They may be old. A link is good without them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub identity: PublicKey,
    pub hints: Vec<String>,
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fos:{}", self.identity)?;
        for (i, hint) in self.hints.iter().enumerate() {
            let sep = if i == 0 { '?' } else { '&' };
            write!(f, "{sep}r={hint}")?;
        }
        Ok(())
    }
}

impl FromStr for Link {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        let rest = text
            .strip_prefix("fos:")
            .ok_or_else(|| Error(format!("not a fileOS link: {text:?}")))?;
        let (key, query) = rest.split_once('?').unwrap_or((rest, ""));
        let hints = query
            .split('&')
            .filter(|part| !part.is_empty())
            .filter_map(|part| part.strip_prefix("r=").map(str::to_string))
            .filter(|hint| !hint.is_empty())
            .collect();
        Ok(Self {
            identity: key.parse()?,
            hints,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_certificate_is_a_proof_by_itself() {
        let identity = SecretKey::from_seed(&[1; 32]);
        let node = SecretKey::from_seed(&[2; 32]).public();
        let cert = NodeCert::sign(&identity, node);
        let back = NodeCert::from_bytes(&cert.to_bytes()).unwrap();
        assert_eq!(back, cert);
        assert_eq!(back.node, node);

        // A certificate for another node, or from another key, does not check.
        let mut other = cert.clone();
        other.node = SecretKey::from_seed(&[3; 32]).public();
        assert!(!other.verify());
        let mut forged = cert;
        forged.identity = node;
        NodeCert::from_bytes(&forged.to_bytes()).unwrap_err();
    }

    #[test]
    fn links_go_to_text_and_back() {
        let identity = SecretKey::from_seed(&[1; 32]).public();
        let bare = Link {
            identity,
            hints: Vec::new(),
        };
        assert_eq!(bare.to_string(), format!("fos:{identity}"));
        assert_eq!(bare.to_string().parse::<Link>().unwrap(), bare);

        let link = Link {
            identity,
            hints: vec![
                "/ip4/192.0.2.1/tcp/4001/p2p/12D3KooW".into(),
                "/dns4/example.org/tcp/4001".into(),
            ],
        };
        assert_eq!(link.to_string().parse::<Link>().unwrap(), link);
        "https://example.org".parse::<Link>().unwrap_err();
        "fos:abc".parse::<Link>().unwrap_err();
    }
}
