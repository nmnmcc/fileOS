// SPDX-License-Identifier: AGPL-3.0-only

//! Identities: one person, many nodes.
//!
//! An identity is a [W3C DID](https://www.w3.org/TR/did-core/) of the method
//! `did:fileos`. The method is the `did:plc` method of AT Protocol, with three
//! changes (see the page "Identity and finding nodes"):
//!
//! - Keys are ed25519 and the hash is BLAKE3, as in the rest of fileOS.
//! - The DID is the full hash, not the first 120 bits of it.
//! - There is no directory server. When the log forks, the branch whose
//!   first operation was signed by the rotation key with the higher priority
//!   wins.
//!
//! The DID is the hash of the first operation of a signed log. Each
//! operation gives the whole state: the rotation keys (which control the
//! identity, the first one with the highest priority), the verification
//! methods (keys for other work), other names, and services. Every operation
//! after the first is signed by a rotation key of the state before it.
//!
//! The verification method `fileos` is the signing key of the identity in
//! fileOS. It signs [`NodeCert`]s, which say that a node is one of the nodes
//! of the identity.
//!
//! This crate is portable: it builds for `wasm32-unknown-unknown` and has no
//! threads, files, sockets or clocks.

use std::collections::BTreeMap;
use std::fmt;
use std::str::FromStr;

use fileos_core::{Error, Hash, PublicKey, SecretKey, Signature, decode, encode};
use serde::{Deserialize, Serialize};

/// The version of the node certificate format.
pub const VERSION: u64 = 1;

/// The name of the identity folder.
pub const FOLDER_NAME: &str = "identity";

/// The directory of the operations of the DID log, in the identity folder.
pub const OPS: &str = "/did";

/// The directory of node certificates, in the identity folder.
pub const NODES: &str = "/nodes";

/// The verification method that signs node certificates.
pub const SIGNING_METHOD: &str = "fileos";

/// The service that names the identity folder.
pub const FOLDER_SERVICE: &str = "fileos_identity";

/// The type of the service that names the identity folder.
pub const FOLDER_SERVICE_TYPE: &str = "FileOSIdentityFolder";

/// The most rotation keys an operation may have, as in `did:plc`.
pub const MAX_ROTATION_KEYS: usize = 5;

/// The most verification methods an operation may have, as in `did:plc`.
pub const MAX_VERIFICATION_METHODS: usize = 10;

/// The most bytes an operation may have, as in `did:plc`.
pub const MAX_OP_BYTES: usize = 7500;

/// The path of the certificate of `node` in the identity folder.
pub fn node_path(node: PublicKey) -> String {
    format!("{NODES}/{node}")
}

/// The path of an operation in the identity folder, by its hash.
pub fn op_path(op: &Op) -> String {
    format!("{OPS}/{}", op.hash())
}

// did:key

/// The multicodec prefix of an ed25519 public key.
const ED25519_PUB: [u8; 2] = [0xed, 0x01];

/// The `did:key` form of an ed25519 public key.
pub fn did_key(key: PublicKey) -> String {
    let mut bytes = ED25519_PUB.to_vec();
    bytes.extend_from_slice(&key.0);
    format!("did:key:z{}", bs58::encode(bytes).into_string())
}

/// Reads an ed25519 public key from its `did:key` form.
///
/// # Errors
///
/// When the text is not the `did:key` of an ed25519 key.
pub fn parse_did_key(text: &str) -> Result<PublicKey, Error> {
    let bad = |why: &dyn fmt::Display| {
        Error(format!(
            "not the did:key of an ed25519 key: {text:?}: {why}"
        ))
    };
    let base58 = text
        .strip_prefix("did:key:z")
        .ok_or_else(|| bad(&"no did:key:z at the start"))?;
    let bytes = bs58::decode(base58).into_vec().map_err(|e| bad(&e))?;
    let key = bytes
        .strip_prefix(&ED25519_PUB[..])
        .ok_or_else(|| bad(&"not an ed25519 key"))?;
    PublicKey::try_from(key).map_err(|e| bad(&e))
}

// base32, lowercase, with no padding (RFC 4648), for the DID.

const BASE32: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";

fn base32(bytes: &[u8]) -> String {
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &byte in bytes {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            out.push(char::from(BASE32[((buffer >> bits) & 31) as usize]));
        }
    }
    if bits > 0 {
        out.push(char::from(BASE32[((buffer << (5 - bits)) & 31) as usize]));
    }
    out
}

fn unbase32(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = BASE32.iter().position(|b| *b == c)? as u32;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

/// The hash of the first operation of `did`: the part of the DID after
/// `did:fileos:`.
///
/// # Errors
///
/// When the text is not a `did:fileos` DID.
pub fn did_hash(did: &str) -> Result<Hash, Error> {
    let bad = || Error(format!("not a did:fileos DID: {did:?}"));
    let bytes = did
        .strip_prefix("did:fileos:")
        .and_then(unbase32)
        .ok_or_else(bad)?;
    let hash = Hash::try_from(bytes.as_slice()).ok().ok_or_else(bad)?;
    (did_of(hash) == did).then_some(hash).ok_or_else(bad)
}

/// The DID whose first operation has the hash `hash`.
pub fn did_of(hash: Hash) -> String {
    format!("did:fileos:{}", base32(&hash.0))
}

// Operations

/// A service of an identity, as in a DID document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    #[serde(rename = "type")]
    pub kind: String,
    pub endpoint: String,
}

/// The state of an identity, as an operation gives it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct State {
    /// The keys that control the identity, as `did:key`. The first one has
    /// the highest priority.
    pub rotation_keys: Vec<String>,
    /// Keys for other work, by name, as `did:key`.
    pub verification_methods: BTreeMap<String, String>,
    /// Other names of the identity, as URIs, for example `fileos://alice.example`.
    pub also_known_as: Vec<String>,
    pub services: BTreeMap<String, Service>,
}

impl State {
    /// The rotation keys, read from their `did:key` form.
    ///
    /// # Errors
    ///
    /// When one of them is not the `did:key` of an ed25519 key.
    pub fn rotation_keys(&self) -> Result<Vec<PublicKey>, Error> {
        self.rotation_keys
            .iter()
            .map(|k| parse_did_key(k))
            .collect()
    }

    /// The signing key of the identity in fileOS, if it has one.
    pub fn signing_key(&self) -> Option<PublicKey> {
        parse_did_key(self.verification_methods.get(SIGNING_METHOD)?).ok()
    }

    /// The ID of the identity folder, if the state names one.
    pub fn folder(&self) -> Option<Hash> {
        let service = self.services.get(FOLDER_SERVICE)?;
        if service.kind != FOLDER_SERVICE_TYPE {
            return None;
        }
        service.endpoint.parse().ok()
    }

    fn check(&self) -> Result<(), Error> {
        let keys = self.rotation_keys()?;
        if keys.is_empty() || keys.len() > MAX_ROTATION_KEYS {
            return Err(Error(format!(
                "an identity has 1 to {MAX_ROTATION_KEYS} rotation keys, not {}",
                keys.len()
            )));
        }
        if (1..keys.len()).any(|i| keys[..i].contains(&keys[i])) {
            return Err(Error("a rotation key is in the list two times".into()));
        }
        if self.verification_methods.len() > MAX_VERIFICATION_METHODS {
            return Err(Error(format!(
                "an identity has at most {MAX_VERIFICATION_METHODS} verification methods"
            )));
        }
        Ok(())
    }
}

/// What an operation does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    /// Gives the new state of the identity.
    Update(State),
    /// Ends the identity. No operation can come after it.
    Tombstone,
}

/// One operation of a DID log, signed by a rotation key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Op {
    pub change: Change,
    /// The hash of the operation before this one. `None` for the first one.
    pub prev: Option<Hash>,
    /// The rotation key that signed the operation.
    pub key: PublicKey,
    pub sig: Signature,
}

/// What an operation signs. The first part keeps the signature from being
/// used as a signature of anything else.
fn op_message(change: &Change, prev: Option<Hash>, key: PublicKey) -> Vec<u8> {
    encode(&("did:fileos operation", change, prev, key))
}

impl Op {
    /// The first operation of a new identity, signed by `rotation_key`,
    /// which must be one of the rotation keys of `state`.
    ///
    /// # Errors
    ///
    /// When the state is not valid, or `rotation_key` is not one of its
    /// rotation keys.
    pub fn genesis(state: State, rotation_key: &SecretKey) -> Result<Op, Error> {
        state.check()?;
        if !state.rotation_keys()?.contains(&rotation_key.public()) {
            return Err(Error(
                "the first operation must be signed by one of its own rotation keys".into(),
            ));
        }
        Ok(Self::sign(Change::Update(state), None, rotation_key))
    }

    /// An operation that comes after `prev`, signed by `rotation_key`.
    pub fn sign(change: Change, prev: Option<Hash>, rotation_key: &SecretKey) -> Op {
        let key = rotation_key.public();
        let sig = rotation_key.sign(&op_message(&change, prev, key));
        Op {
            change,
            prev,
            key,
            sig,
        }
    }

    pub fn hash(&self) -> Hash {
        Hash::of_value(self)
    }

    /// The DID that this operation makes, if it is a first operation.
    pub fn did(&self) -> String {
        did_of(self.hash())
    }

    fn verify_sig(&self) -> bool {
        self.key
            .verify(&op_message(&self.change, self.prev, self.key), &self.sig)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self)
    }

    /// # Errors
    ///
    /// When the bytes are too many, or are not an operation.
    pub fn from_bytes(bytes: &[u8]) -> Result<Op, Error> {
        if bytes.len() > MAX_OP_BYTES {
            return Err(Error(format!(
                "an operation has at most {MAX_OP_BYTES} bytes"
            )));
        }
        decode(bytes)
    }
}

/// The state of an identity after its log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    pub did: String,
    /// The state now. `None` when the identity has ended.
    pub state: Option<State>,
    /// The operations of the line that wins, first to last.
    pub line: Vec<Hash>,
}

/// Says if `op` may be the first operation of an identity.
fn valid_genesis(op: &Op) -> bool {
    op.prev.is_none()
        && op.verify_sig()
        && matches!(&op.change, Change::Update(s)
            if s.check().is_ok() && s.rotation_keys().is_ok_and(|keys| keys.contains(&op.key)))
}

/// The DIDs that the valid first operations in `ops` make.
pub fn dids(ops: &[Op]) -> Vec<String> {
    ops.iter()
        .filter(|op| valid_genesis(op))
        .map(Op::did)
        .collect()
}

/// Gives the state of the identity `did` from the operations of its log,
/// in any order. Operations that are not valid, or that are not in the line
/// that wins, are not used.
///
/// The line goes from the first operation. At each step, the next
/// operation is one that is signed by a rotation key of the state now. When
/// there are more than one (a fork), the one signed by the key with the
/// highest priority wins; when the same key signed more than one, the one
/// with the lowest hash wins.
///
/// # Errors
///
/// When no valid first operation makes `did`.
pub fn resolve(did: &str, ops: &[Op]) -> Result<Resolved, Error> {
    let genesis = ops
        .iter()
        .find(|op| op.did() == did && valid_genesis(op))
        .ok_or_else(|| Error(format!("no valid first operation for {did}")))?;
    let mut children: BTreeMap<Hash, Vec<&Op>> = BTreeMap::new();
    for op in ops {
        if let Some(prev) = op.prev {
            children.entry(prev).or_default().push(op);
        }
    }
    let Change::Update(first) = &genesis.change else {
        return Err(Error("the first operation is not an update".into()));
    };
    let mut state = first.clone();
    let mut head = genesis.hash();
    let mut line = vec![head];
    while let Ok(keys) = state.rotation_keys() {
        let next = children
            .get(&head)
            .into_iter()
            .flatten()
            .filter(|op| op.verify_sig())
            .filter(|op| match &op.change {
                Change::Update(s) => s.check().is_ok(),
                Change::Tombstone => true,
            })
            .filter_map(|op| Some((keys.iter().position(|k| *k == op.key)?, op.hash(), *op)))
            .min_by_key(|(priority, hash, _)| (*priority, *hash));
        let Some((_, hash, op)) = next else {
            break;
        };
        head = hash;
        line.push(hash);
        match &op.change {
            Change::Update(s) => state = s.clone(),
            Change::Tombstone => {
                return Ok(Resolved {
                    did: did.to_string(),
                    state: None,
                    line,
                });
            }
        }
    }
    Ok(Resolved {
        did: did.to_string(),
        state: Some(state),
        line,
    })
}

// Node certificates

/// A statement, signed by the signing key of an identity, that a node is one
/// of its nodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCert {
    pub version: u64,
    pub did: String,
    pub node: PublicKey,
    pub sig: Signature,
}

fn cert_message(version: u64, did: &str, node: PublicKey) -> Vec<u8> {
    encode(&("fileOS node certificate", version, did, node))
}

impl NodeCert {
    /// The certificate that `node` is one of the nodes of `did`, signed by
    /// the signing key of the identity.
    pub fn sign(did: &str, signing_key: &SecretKey, node: PublicKey) -> Self {
        Self {
            version: VERSION,
            did: did.to_string(),
            node,
            sig: signing_key.sign(&cert_message(VERSION, did, node)),
        }
    }

    /// Says if `signing_key` signed this certificate.
    pub fn verify(&self, signing_key: PublicKey) -> bool {
        self.version == VERSION
            && signing_key.verify(&cert_message(self.version, &self.did, self.node), &self.sig)
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self)
    }

    /// # Errors
    ///
    /// When the bytes are not a certificate.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        decode(bytes)
    }
}

/// A statement, signed by the signing key of an identity, that a node is no
/// longer one of its nodes.
///
/// It ends the certificate of that node for good; to take the node back, the
/// identity uses a new signing key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeRevocation {
    pub version: u64,
    pub did: String,
    pub node: PublicKey,
    pub sig: Signature,
}

fn revocation_message(version: u64, did: &str, node: PublicKey) -> Vec<u8> {
    encode(&("fileOS node revocation", version, did, node))
}

impl NodeRevocation {
    /// The revocation of `node` as a node of `did`, signed by the signing
    /// key of the identity.
    pub fn sign(did: &str, signing_key: &SecretKey, node: PublicKey) -> Self {
        Self {
            version: VERSION,
            did: did.to_string(),
            node,
            sig: signing_key.sign(&revocation_message(VERSION, did, node)),
        }
    }

    /// Says if `signing_key` signed this revocation.
    pub fn verify(&self, signing_key: PublicKey) -> bool {
        self.version == VERSION
            && signing_key.verify(
                &revocation_message(self.version, &self.did, self.node),
                &self.sig,
            )
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self)
    }

    /// # Errors
    ///
    /// When the bytes are not a revocation.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, Error> {
        decode(bytes)
    }
}

/// The directory of node revocations, in the identity space.
pub const REVOKED: &str = "/revoked";

/// The path of the revocation of `node` in the identity space.
pub fn revocation_path(node: PublicKey) -> String {
    format!("{REVOKED}/{node}")
}

// Links

/// An identity as people give it to each other: a DID URL with hints,
/// `did:fileos:…?r=<multiaddr>&r=<multiaddr>`.
///
/// The `r` values are hints: addresses of nodes of the identity, or of
/// rendezvous nodes. They may be old. A link is good without them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub did: String,
    pub hints: Vec<String>,
}

impl fmt::Display for Link {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.did)?;
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
        let (did, query) = text.split_once('?').unwrap_or((text, ""));
        let id = did
            .strip_prefix("did:fileos:")
            .ok_or_else(|| Error(format!("not a did:fileos link: {text:?}")))?;
        if id.is_empty() || !id.bytes().all(|b| BASE32.contains(&b)) {
            return Err(Error(format!("not a did:fileos link: {text:?}")));
        }
        let hints = query
            .split('&')
            .filter_map(|part| part.strip_prefix("r="))
            .filter(|hint| !hint.is_empty())
            .map(str::to_string)
            .collect();
        Ok(Self {
            did: did.to_string(),
            hints,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> SecretKey {
        SecretKey::from_seed(&[n; 32])
    }

    fn state(rotation: &[&SecretKey], signing: &SecretKey) -> State {
        State {
            rotation_keys: rotation.iter().map(|k| did_key(k.public())).collect(),
            verification_methods: [(SIGNING_METHOD.to_string(), did_key(signing.public()))].into(),
            also_known_as: Vec::new(),
            services: BTreeMap::new(),
        }
    }

    #[test]
    fn did_key_and_base32() {
        let k = key(1).public();
        let text = did_key(k);
        // Every ed25519 did:key starts with "z6Mk".
        assert!(text.starts_with("did:key:z6Mk"));
        assert_eq!(parse_did_key(&text).unwrap(), k);
        parse_did_key("did:key:zQ3shokFTS3brHcDQrn82RUDfCZESWL1ZdCEJwekUDPQiYBme").unwrap_err();
        // RFC 4648 test vectors, in lowercase and with no padding.
        assert_eq!(base32(b"foobar"), "mzxw6ytboi");
        assert_eq!(base32(b"f"), "my");
    }

    #[test]
    fn the_did_is_the_hash_of_the_first_operation() {
        let (r, s) = (key(1), key(2));
        let genesis = Op::genesis(state(&[&r], &s), &r).unwrap();
        let did = genesis.did();
        assert!(did.starts_with("did:fileos:"));
        assert_eq!(did.len(), "did:fileos:".len() + 52);
        assert_eq!(did_hash(&did).unwrap(), genesis.hash());
        did_hash("did:fileos:abc").unwrap_err();
        assert_eq!(
            dids(std::slice::from_ref(&genesis)),
            std::slice::from_ref(&did)
        );
        let resolved = resolve(&did, std::slice::from_ref(&genesis)).unwrap();
        assert_eq!(resolved.state.unwrap().signing_key(), Some(s.public()));
        // A first operation signed by a key that is not one of its rotation keys.
        Op::genesis(state(&[&r], &s), &s).unwrap_err();
        let forged = Op::sign(Change::Update(state(&[&r], &s)), None, &s);
        resolve(&forged.did(), &[forged]).unwrap_err();
        let back = Op::from_bytes(&genesis.to_bytes()).unwrap();
        assert_eq!(back, genesis);
    }

    #[test]
    fn rotation_keys_change_the_state() {
        let (r1, r2, s1, s2) = (key(1), key(2), key(3), key(4));
        let genesis = Op::genesis(state(&[&r1], &s1), &r1).unwrap();
        let did = genesis.did();
        // r1 adds r2 and changes the signing key; then r2 (now a rotation
        // key) changes it again.
        let a = Op::sign(
            Change::Update(state(&[&r1, &r2], &s2)),
            Some(genesis.hash()),
            &r1,
        );
        let b = Op::sign(Change::Update(state(&[&r1, &r2], &s1)), Some(a.hash()), &r2);
        // An operation by a key that is not a rotation key is not used.
        let bad = Op::sign(Change::Update(state(&[&s2], &s2)), Some(b.hash()), &s2);
        let ops = vec![bad, b.clone(), genesis, a];
        let resolved = resolve(&did, &ops).unwrap();
        assert_eq!(resolved.line.len(), 3);
        assert_eq!(resolved.state.unwrap().signing_key(), Some(s1.public()));

        let mut ops = ops;
        ops.push(Op::sign(Change::Tombstone, Some(b.hash()), &r1));
        assert_eq!(resolve(&did, &ops).unwrap().state, None);
    }

    #[test]
    fn at_a_fork_the_key_with_the_higher_priority_wins() {
        let (recovery, daily, s1, thief) = (key(1), key(2), key(3), key(9));
        let genesis = Op::genesis(state(&[&recovery, &daily], &s1), &recovery).unwrap();
        let did = genesis.did();
        // Someone who has the daily key puts their own key in.
        let stolen = Op::sign(
            Change::Update(state(&[&thief], &thief)),
            Some(genesis.hash()),
            &daily,
        );
        let more = Op::sign(
            Change::Update(state(&[&thief], &thief)),
            Some(stolen.hash()),
            &thief,
        );
        let ops = vec![genesis.clone(), stolen, more];
        let resolved = resolve(&did, &ops).unwrap();
        assert_eq!(resolved.state.unwrap().signing_key(), Some(thief.public()));
        // The recovery key, which has a higher priority, forks from before
        // the theft, and its line wins, though it is shorter.
        let fix = Op::sign(
            Change::Update(state(&[&recovery], &s1)),
            Some(genesis.hash()),
            &recovery,
        );
        let mut ops = ops;
        ops.push(fix.clone());
        let resolved = resolve(&did, &ops).unwrap();
        assert_eq!(resolved.line.last(), Some(&fix.hash()));
        assert_eq!(resolved.state.unwrap().signing_key(), Some(s1.public()));
    }

    #[test]
    fn a_node_certificate_is_signed_by_the_signing_key() {
        let (r, s) = (key(1), key(2));
        let did = Op::genesis(state(&[&r], &s), &r).unwrap().did();
        let node = key(5).public();
        let cert = NodeCert::sign(&did, &s, node);
        let back = NodeCert::from_bytes(&cert.to_bytes()).unwrap();
        assert!(back.verify(s.public()));
        assert!(!back.verify(r.public()));
    }

    #[test]
    fn links_go_to_text_and_back() {
        let did = Op::genesis(state(&[&key(1)], &key(2)), &key(1))
            .unwrap()
            .did();
        let bare = Link {
            did: did.clone(),
            hints: Vec::new(),
        };
        assert_eq!(bare.to_string(), did);
        assert_eq!(bare.to_string().parse::<Link>().unwrap(), bare);
        let link = Link {
            did,
            hints: vec!["/ip4/192.0.2.1/tcp/4001/p2p/12D3KooW".into()],
        };
        assert_eq!(link.to_string().parse::<Link>().unwrap(), link);
        "did:plc:abc".parse::<Link>().unwrap_err();
        "did:fileos:ABC".parse::<Link>().unwrap_err();
    }
}
