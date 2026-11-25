// SPDX-License-Identifier: AGPL-3.0-only

//! The log formats: proposals, entries and heads.
//!
//! An author (a node acting for a person) signs a [`Proposal`]: an operation
//! on one resource, naming the payload only by its hash. The primary of the
//! space orders proposals into [`Entry`] values, each carrying the sequence
//! number and the hash of the previous entry, and signs them. Because the
//! chain covers the payload's hash and not the payload, payloads can later be
//! erased while the chain still verifies.

use crate::cbor::{Value, map};
use crate::{FormatError, Hash, PublicKey};

/// What a proposal does.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    /// Creates the space. The payload describes it (JSON).
    Genesis,
    /// Creates or replaces a resource with the payload.
    Put,
    /// Deletes a resource.
    Delete,
    /// Adds, changes or removes a member (JSON payload).
    Member,
    /// Records a grant (JSON payload).
    Grant,
    /// Revokes a grant; the target is `grant/<id>`.
    Revoke,
    /// Asks every follower to erase the payloads of a resource's history.
    Erase,
    /// An opaque MLS message (commit or proposal) for the space's group.
    Mls,
    /// Hands the space over to a new primary (JSON payload).
    Handover,
}

impl Kind {
    /// The text form stored in the log.
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Genesis => "genesis",
            Kind::Put => "put",
            Kind::Delete => "delete",
            Kind::Member => "member",
            Kind::Grant => "grant",
            Kind::Revoke => "revoke",
            Kind::Erase => "erase",
            Kind::Mls => "mls",
            Kind::Handover => "handover",
        }
    }

    /// Parse the text form.
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "genesis" => Kind::Genesis,
            "put" => Kind::Put,
            "delete" => Kind::Delete,
            "member" => Kind::Member,
            "grant" => Kind::Grant,
            "revoke" => Kind::Revoke,
            "erase" => Kind::Erase,
            "mls" => Kind::Mls,
            "handover" => Kind::Handover,
            _ => return None,
        })
    }
}

/// An operation on one resource, signed by its author.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Proposal {
    /// The space. All zero for a genesis proposal.
    pub space: Hash,
    /// What it does.
    pub kind: Kind,
    /// The resource, `type/id`.
    pub target: String,
    /// The hash of the payload, if there is one.
    pub payload: Option<Hash>,
    /// The payload's media type, such as `application/json`.
    pub media_type: String,
    /// Milliseconds since the Unix epoch, by the author's clock.
    pub time: u64,
    /// A random value chosen by the author; retries reuse it (idempotency).
    pub nonce: Vec<u8>,
    /// The grant under which the author wrote this, if any.
    pub grant: Option<String>,
}

impl Proposal {
    /// Encode as deterministic CBOR.
    pub fn encode(&self) -> Vec<u8> {
        map([
            (1, Value::Bytes(self.space.0.to_vec())),
            (2, Value::Text(self.kind.as_str().into())),
            (3, Value::Text(self.target.clone())),
            (
                4,
                self.payload
                    .map_or(Value::Null, |h| Value::Bytes(h.0.to_vec())),
            ),
            (5, Value::Text(self.media_type.clone())),
            (6, Value::Uint(self.time)),
            (7, Value::Bytes(self.nonce.clone())),
            (8, self.grant.clone().map_or(Value::Null, Value::Text)),
        ])
        .encode()
    }

    /// Decode from CBOR.
    ///
    /// # Errors
    ///
    /// Returns an error if a required proposal field is absent, malformed or inconsistent with its
    /// kind.
    pub fn decode(bytes: &[u8]) -> Result<Proposal, FormatError> {
        let v = Value::decode(bytes)?;
        let err = |f: &str| FormatError(format!("proposal: bad field {f}"));
        let hash = |v: Option<&Value>| v.and_then(Value::as_bytes).and_then(Hash::from_slice);
        let target = v
            .get(3)
            .and_then(Value::as_text)
            .ok_or_else(|| err("target"))?;
        let kind = v
            .get(2)
            .and_then(Value::as_text)
            .and_then(Kind::parse)
            .ok_or_else(|| err("kind"))?;
        if !crate::valid_target(target) {
            return Err(err("target"));
        }
        Ok(Proposal {
            space: hash(v.get(1)).ok_or_else(|| err("space"))?,
            kind,
            target: target.into(),
            payload: match v.get(4) {
                Some(Value::Null) => None,
                other => Some(hash(other).ok_or_else(|| err("payload"))?),
            },
            media_type: v
                .get(5)
                .and_then(Value::as_text)
                .ok_or_else(|| err("media type"))?
                .into(),
            time: v
                .get(6)
                .and_then(Value::as_u64)
                .ok_or_else(|| err("time"))?,
            nonce: v
                .get(7)
                .and_then(Value::as_bytes)
                .ok_or_else(|| err("nonce"))?
                .to_vec(),
            grant: match v.get(8) {
                Some(Value::Null) => None,
                other => Some(
                    other
                        .and_then(Value::as_text)
                        .ok_or_else(|| err("grant"))?
                        .into(),
                ),
            },
        })
    }
}

/// A proposal placed in the log by the primary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Position in the log, starting at 0 with the genesis.
    pub seq: u64,
    /// Hash of the previous signed entry; [`Hash::ZERO`] for the genesis.
    pub prev: Hash,
    /// Milliseconds since the Unix epoch, by the primary's clock.
    pub time: u64,
    /// The author's signed proposal (`COSE_Sign1` bytes).
    pub proposal: Vec<u8>,
}

impl Entry {
    /// Encode as deterministic CBOR.
    pub fn encode(&self) -> Vec<u8> {
        map([
            (1, Value::Uint(self.seq)),
            (2, Value::Bytes(self.prev.0.to_vec())),
            (3, Value::Uint(self.time)),
            (4, Value::Bytes(self.proposal.clone())),
        ])
        .encode()
    }

    /// Decode from CBOR.
    ///
    /// # Errors
    ///
    /// Returns an error if an entry field is absent or malformed.
    pub fn decode(bytes: &[u8]) -> Result<Entry, FormatError> {
        let v = Value::decode(bytes)?;
        let err = |f: &str| FormatError(format!("entry: bad field {f}"));
        Ok(Entry {
            seq: v.get(1).and_then(Value::as_u64).ok_or_else(|| err("seq"))?,
            prev: v
                .get(2)
                .and_then(Value::as_bytes)
                .and_then(Hash::from_slice)
                .ok_or_else(|| err("prev"))?,
            time: v
                .get(3)
                .and_then(Value::as_u64)
                .ok_or_else(|| err("time"))?,
            proposal: v
                .get(4)
                .and_then(Value::as_bytes)
                .ok_or_else(|| err("proposal"))?
                .to_vec(),
        })
    }
}

/// The mutable pointer to the end of a log, signed by the primary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    /// The space.
    pub space: Hash,
    /// Sequence number of the last entry.
    pub seq: u64,
    /// Hash of the last signed entry.
    pub hash: Hash,
    /// Hashes of the sealed segments, oldest first.
    pub segments: Vec<Hash>,
    /// The primary that signed this head.
    pub primary: PublicKey,
}

impl Head {
    /// Encode as deterministic CBOR.
    pub fn encode(&self) -> Vec<u8> {
        map([
            (1, Value::Bytes(self.space.0.to_vec())),
            (2, Value::Uint(self.seq)),
            (3, Value::Bytes(self.hash.0.to_vec())),
            (
                4,
                Value::Array(
                    self.segments
                        .iter()
                        .map(|h| Value::Bytes(h.0.to_vec()))
                        .collect(),
                ),
            ),
            (5, Value::Bytes(self.primary.0.to_vec())),
        ])
        .encode()
    }

    /// Decode from CBOR.
    ///
    /// # Errors
    ///
    /// Returns an error if the head fields or segment hashes are malformed.
    pub fn decode(bytes: &[u8]) -> Result<Head, FormatError> {
        let v = Value::decode(bytes)?;
        let err = |f: &str| FormatError(format!("head: bad field {f}"));
        let hash = |v: Option<&Value>| v.and_then(Value::as_bytes).and_then(Hash::from_slice);
        Ok(Head {
            space: hash(v.get(1)).ok_or_else(|| err("space"))?,
            seq: v.get(2).and_then(Value::as_u64).ok_or_else(|| err("seq"))?,
            hash: hash(v.get(3)).ok_or_else(|| err("hash"))?,
            segments: v
                .get(4)
                .and_then(Value::as_array)
                .ok_or_else(|| err("segments"))?
                .iter()
                .map(|s| {
                    s.as_bytes()
                        .and_then(Hash::from_slice)
                        .ok_or_else(|| err("segment"))
                })
                .collect::<Result<_, _>>()?,
            primary: v
                .get(5)
                .and_then(Value::as_bytes)
                .and_then(PublicKey::from_slice)
                .ok_or_else(|| err("primary"))?,
        })
    }
}

/// A sealed segment: a run of consecutive signed entries, each prefixed by
/// its length as a CBOR unsigned integer. Its name is the hash of its bytes.
pub fn encode_segment(entries: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for e in entries {
        out.extend(Value::Uint(e.len() as u64).encode());
        out.extend_from_slice(e);
    }
    out
}

/// Split a segment back into signed entries.
///
/// # Errors
///
/// Returns an error if the segment is malformed or contains a non-byte-string item.
pub fn decode_segment(mut bytes: &[u8]) -> Result<Vec<Vec<u8>>, FormatError> {
    let mut out = Vec::new();
    while !bytes.is_empty() {
        let (len, used) = Value::decode_prefix(bytes)?;
        let len = len
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| FormatError("segment: bad length".into()))?;
        let rest = &bytes[used..];
        if rest.len() < len {
            return Err(FormatError("segment: truncated".into()));
        }
        out.push(rest[..len].to_vec());
        bytes = &rest[len..];
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proposal_round_trip() {
        let p = Proposal {
            space: Hash::of(b"s"),
            kind: Kind::Put,
            target: "note/1".into(),
            payload: Some(Hash::of(b"p")),
            media_type: "application/json".into(),
            time: 1_700_000_000_000,
            nonce: vec![1, 2, 3],
            grant: Some("g1".into()),
        };
        assert_eq!(Proposal::decode(&p.encode()).unwrap(), p);
        let e = Entry {
            seq: 7,
            prev: Hash::of(b"x"),
            time: 5,
            proposal: p.encode(),
        };
        assert_eq!(Entry::decode(&e.encode()).unwrap(), e);
        let seg = encode_segment(&[vec![1; 30], vec![2; 300]]);
        assert_eq!(
            decode_segment(&seg).unwrap(),
            vec![vec![1; 30], vec![2; 300]]
        );
    }
}
