// SPDX-License-Identifier: AGPL-3.0-only

//! The identity log: a short, append-only log signed by the personal key.

use hs_core::cbor::{Value, map};
use hs_core::cose::Sign1;
use hs_core::{FormatError, Hash, PublicKey};

use crate::{Keypair, verify1};

/// What an identity log entry declares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LogKind {
    /// The first entry: the identity exists, with this personal key.
    Create,
    /// A node serves this identity.
    AddNode {
        /// The node's key.
        key: PublicKey,
    },
    /// A node no longer serves this identity.
    RemoveNode {
        /// The node's key.
        key: PublicKey,
    },
    /// An address (URL) where the identity's nodes can be reached.
    Address {
        /// The URL, such as `https://home.example.org`.
        url: String,
    },
    /// A domain alias.
    Alias {
        /// The domain.
        domain: String,
    },
    /// From now on, entries are signed by a new personal key.
    Rotate {
        /// The new personal key.
        key: PublicKey,
    },
}

/// One identity log entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogEntry {
    /// Position, starting at 0.
    pub seq: u64,
    /// Hash of the previous signed entry, or zero.
    pub prev: Hash,
    /// Milliseconds since the Unix epoch.
    pub time: u64,
    /// The declaration.
    pub kind: LogKind,
}

impl LogEntry {
    fn encode(&self) -> Vec<u8> {
        let (kind, value) = match &self.kind {
            LogKind::Create => ("create", Value::Null),
            LogKind::AddNode { key } => ("add-node", Value::Bytes(key.0.to_vec())),
            LogKind::RemoveNode { key } => ("remove-node", Value::Bytes(key.0.to_vec())),
            LogKind::Address { url } => ("address", Value::Text(url.clone())),
            LogKind::Alias { domain } => ("alias", Value::Text(domain.clone())),
            LogKind::Rotate { key } => ("rotate", Value::Bytes(key.0.to_vec())),
        };
        map([
            (1, Value::Uint(self.seq)),
            (2, Value::Bytes(self.prev.0.to_vec())),
            (3, Value::Uint(self.time)),
            (4, Value::Text(kind.into())),
            (5, value),
        ])
        .encode()
    }

    fn decode(bytes: &[u8]) -> Result<LogEntry, FormatError> {
        let v = Value::decode(bytes)?;
        let err = || FormatError("identity log: bad entry".into());
        let key = || {
            v.get(5)
                .and_then(Value::as_bytes)
                .and_then(PublicKey::from_slice)
                .ok_or_else(err)
        };
        let text = || {
            v.get(5)
                .and_then(Value::as_text)
                .map(String::from)
                .ok_or_else(err)
        };
        let kind = match v.get(4).and_then(Value::as_text).ok_or_else(err)? {
            "create" => LogKind::Create,
            "add-node" => LogKind::AddNode { key: key()? },
            "remove-node" => LogKind::RemoveNode { key: key()? },
            "address" => LogKind::Address { url: text()? },
            "alias" => LogKind::Alias { domain: text()? },
            "rotate" => LogKind::Rotate { key: key()? },
            _ => return Err(err()),
        };
        Ok(LogEntry {
            seq: v.get(1).and_then(Value::as_u64).ok_or_else(err)?,
            prev: v
                .get(2)
                .and_then(Value::as_bytes)
                .and_then(Hash::from_slice)
                .ok_or_else(err)?,
            time: v.get(3).and_then(Value::as_u64).ok_or_else(err)?,
            kind,
        })
    }
}

/// The current state declared by an identity log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IdentityState {
    /// The identity ID: the first personal key. Never changes.
    pub id: PublicKey,
    /// The current personal key.
    pub personal: PublicKey,
    /// Nodes serving this identity.
    pub nodes: Vec<PublicKey>,
    /// Addresses.
    pub addresses: Vec<String>,
    /// Domain aliases.
    pub aliases: Vec<String>,
    /// Sequence number of the last entry.
    pub seq: u64,
    /// Hash of the last signed entry.
    pub head: Hash,
}

/// A verified identity log.
#[derive(Clone, Debug, Default)]
pub struct IdentityLog {
    /// The signed entries (`COSE_Sign1` bytes), in order.
    pub entries: Vec<Vec<u8>>,
}

impl IdentityLog {
    /// Start a new identity.
    pub fn create(personal: &Keypair, time: u64) -> IdentityLog {
        let entry = LogEntry {
            seq: 0,
            prev: Hash::ZERO,
            time,
            kind: LogKind::Create,
        };
        IdentityLog {
            entries: vec![personal.sign1(entry.encode()).encode()],
        }
    }

    /// Verify every entry and compute the current state.
    ///
    /// # Errors
    ///
    /// Returns an error if the log is empty or its signatures, sequence or transitions are invalid.
    pub fn state(&self) -> Result<IdentityState, FormatError> {
        let mut state: Option<IdentityState> = None;
        for (i, raw) in self.entries.iter().enumerate() {
            let msg = Sign1::decode(raw)?;
            let signer =
                verify1(&msg).ok_or_else(|| FormatError("identity log: bad signature".into()))?;
            let entry = LogEntry::decode(&msg.payload)?;
            let prev = state.as_ref().map_or(Hash::ZERO, |s| s.head);
            if entry.seq != i as u64 || entry.prev != prev {
                return Err(FormatError("identity log: broken chain".into()));
            }
            let s = match (&mut state, &entry.kind) {
                (None, LogKind::Create) => state.insert(IdentityState {
                    id: signer,
                    personal: signer,
                    nodes: vec![],
                    addresses: vec![],
                    aliases: vec![],
                    seq: 0,
                    head: Hash::ZERO,
                }),
                (Some(s), _) if s.personal == signer && entry.kind != LogKind::Create => s,
                _ => return Err(FormatError("identity log: entry not allowed".into())),
            };
            match entry.kind {
                LogKind::Create => {}
                LogKind::AddNode { key } => {
                    if !s.nodes.contains(&key) {
                        s.nodes.push(key);
                    }
                }
                LogKind::RemoveNode { key } => s.nodes.retain(|k| *k != key),
                LogKind::Address { url } => {
                    if !s.addresses.contains(&url) {
                        s.addresses.push(url);
                    }
                }
                LogKind::Alias { domain } => {
                    if !s.aliases.contains(&domain) {
                        s.aliases.push(domain);
                    }
                }
                LogKind::Rotate { key } => s.personal = key,
            }
            s.seq = entry.seq;
            s.head = Hash::of(raw);
        }
        state.ok_or_else(|| FormatError("identity log: empty".into()))
    }

    /// Append a declaration, signed by the current personal key.
    ///
    /// # Errors
    ///
    /// Returns an error if the existing log, signing key or new declaration is invalid.
    pub fn append(
        &mut self,
        personal: &Keypair,
        time: u64,
        kind: LogKind,
    ) -> Result<(), FormatError> {
        let state = self.state()?;
        if state.personal != personal.public() {
            return Err(FormatError(
                "identity log: not the current personal key".into(),
            ));
        }
        let entry = LogEntry {
            seq: state.seq + 1,
            prev: state.head,
            time,
            kind,
        };
        self.entries.push(personal.sign1(entry.encode()).encode());
        Ok(())
    }

    /// Encode the whole log as a CBOR array of signed entries.
    pub fn encode(&self) -> Vec<u8> {
        Value::Array(self.entries.iter().cloned().map(Value::Bytes).collect()).encode()
    }

    /// Decode and verify.
    ///
    /// # Errors
    ///
    /// Returns an error if the encoded log is malformed or fails verification.
    pub fn decode(bytes: &[u8]) -> Result<IdentityLog, FormatError> {
        let v = Value::decode(bytes)?;
        let entries = v
            .as_array()
            .ok_or_else(|| FormatError("identity log: not an array".into()))?
            .iter()
            .map(|e| {
                e.as_bytes()
                    .map(<[u8]>::to_vec)
                    .ok_or_else(|| FormatError("identity log".into()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let log = IdentityLog { entries };
        log.state()?;
        Ok(log)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_lifecycle() {
        let personal = Keypair::from_seed([1; 32]);
        let node = Keypair::from_seed([2; 32]);
        let mut log = IdentityLog::create(&personal, 1);
        log.append(&personal, 2, LogKind::AddNode { key: node.public() })
            .unwrap();
        log.append(
            &personal,
            3,
            LogKind::Address {
                url: "https://a.example".into(),
            },
        )
        .unwrap();
        let next = Keypair::from_seed([3; 32]);
        log.append(&personal, 4, LogKind::Rotate { key: next.public() })
            .unwrap();
        assert!(
            log.append(&personal, 5, LogKind::Alias { domain: "x".into() })
                .is_err()
        );
        log.append(
            &next,
            5,
            LogKind::Alias {
                domain: "a.example".into(),
            },
        )
        .unwrap();

        let back = IdentityLog::decode(&log.encode()).unwrap();
        let s = back.state().unwrap();
        assert_eq!(s.id, personal.public());
        assert_eq!(s.personal, next.public());
        assert_eq!(s.nodes, vec![node.public()]);
        assert_eq!(s.aliases, vec!["a.example".to_string()]);

        // A node key cannot sign entries.
        let mut forged = back.clone();
        let entry = LogEntry {
            seq: 5,
            prev: s.head,
            time: 6,
            kind: LogKind::AddNode { key: node.public() },
        };
        forged.entries.push(node.sign1(entry.encode()).encode());
        assert!(forged.state().is_err());
    }
}
