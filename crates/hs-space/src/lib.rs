// SPDX-License-Identifier: AGPL-3.0-only

//! Spaces: the unit of sharing and sync.
//!
//! A space has members, a primary and a log. The primary orders signed
//! proposals into signed entries ([`Space::order`]); everyone else verifies
//! and applies the same entries ([`Space::apply`]). This crate is sans-IO:
//! payload bytes, keys and time come from the caller, and nothing here
//! touches files, sockets or clocks, so the same code runs in production and
//! in deterministic simulation.

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use hs_core::cbor::{Value, map};
use hs_core::cose::Sign1;
use hs_core::event::{Entry, Head, Kind, Proposal};
use hs_core::{Hash, PublicKey};
use hs_identity::{Keypair, verify1};
use serde_json::Value as Json;

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// Why a proposal or entry was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refused(pub String);

impl fmt::Display for Refused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Refused {}

fn refused<T>(why: impl Into<String>) -> Result<T, Refused> {
    Err(Refused(why.into()))
}

/// A member's role.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Can read.
    Reader,
    /// Can read and change resources.
    Writer,
    /// Can do everything, including changing members and grants.
    Owner,
}

impl Role {
    /// Parse the text form.
    pub fn parse(s: &str) -> Option<Role> {
        Some(match s {
            "reader" => Role::Reader,
            "writer" => Role::Writer,
            "owner" => Role::Owner,
            _ => return None,
        })
    }

    /// The text form.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Reader => "reader",
            Role::Writer => "writer",
            Role::Owner => "owner",
        }
    }
}

/// A member: a person's homeserver, identified by its node key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The person's identity ID.
    pub identity: PublicKey,
    /// The role.
    pub role: Role,
    /// Where the member's homeserver can be reached, if known.
    pub home: Option<String>,
}

/// The current state of one resource.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resource {
    /// The current payload, or `None` if deleted.
    pub payload: Option<Hash>,
    /// Media type of the payload.
    pub media_type: String,
    /// Sequence number of the entry that last changed it.
    pub seq: u64,
    /// Time of that entry (primary's clock).
    pub time: u64,
    /// Every payload it has had, for erasing.
    pub history: Vec<Hash>,
}

/// A grant recorded in the space.
#[derive(Clone, Debug, PartialEq)]
pub struct Grant {
    /// The grant as recorded (JSON).
    pub body: Json,
    /// Whether it has been revoked.
    pub revoked: bool,
}

/// What applying an entry changed, for the caller to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Applied {
    /// Sequence number.
    pub seq: u64,
    /// Hash of the signed entry.
    pub hash: Hash,
    /// The proposal it carried.
    pub proposal: Proposal,
    /// Who wrote it.
    pub author: PublicKey,
    /// Primary's time.
    pub time: u64,
    /// Payloads that every follower should now delete.
    pub erase: Vec<Hash>,
}

/// The result of ordering a proposal.
#[derive(Clone, Debug, PartialEq, Eq)]
#[expect(clippy::large_enum_variant, reason = "short-lived; almost always New")]
pub enum Ordered {
    /// A new entry: its signed bytes and what it changed.
    New(Vec<u8>, Applied),
    /// The same author already sent this nonce; this is its sequence number.
    Duplicate(u64),
}

/// A space's state, built from its log.
#[derive(Clone, Debug)]
pub struct Space {
    /// The space ID: the hash of the signed genesis proposal.
    pub id: Hash,
    /// Name and other settings from the genesis (JSON).
    pub info: Json,
    /// The node that orders entries.
    pub primary: PublicKey,
    /// Members by node key.
    pub members: BTreeMap<PublicKey, Member>,
    /// Resources by target.
    pub resources: BTreeMap<String, Resource>,
    /// Grants by ID.
    pub grants: BTreeMap<String, Grant>,
    /// Sequence number of the last entry.
    pub seq: u64,
    /// Hash of the last signed entry.
    pub head: Hash,
    nonces: HashMap<(PublicKey, Vec<u8>), u64>,
}

enum Update {
    Genesis(Json, PublicKey, BTreeMap<PublicKey, Member>),
    Resource,
    Erase,
    Member(PublicKey, Option<Member>),
    Grant(String, Json),
    Revoke,
    Handover(PublicKey),
    None,
}

/// Fetch a payload by hash, or `None` if it is not available (or erased).
pub trait Payloads {
    /// Get the bytes of a payload.
    fn payload(&self, hash: &Hash) -> Option<Vec<u8>>;
}

impl<F: Fn(&Hash) -> Option<Vec<u8>>> Payloads for F {
    fn payload(&self, hash: &Hash) -> Option<Vec<u8>> {
        self(hash)
    }
}

fn payload_json(proposal: &Proposal, payloads: &dyn Payloads) -> Option<Json> {
    proposal
        .payload
        .and_then(|h| payloads.payload(&h).filter(|b| Hash::of(b) == h))
        .and_then(|b| serde_json::from_slice(&b).ok())
}

fn parse_member(j: &Json) -> Option<(PublicKey, Member)> {
    let node = PublicKey::from_id(j.get("node")?.as_str()?)?;
    let identity = PublicKey::from_id(j.get("identity")?.as_str()?)?;
    let role = j
        .get("role")
        .and_then(Json::as_str)
        .map_or(Some(Role::Writer), Role::parse)?;
    let home = j.get("home").and_then(Json::as_str).map(String::from);
    Some((
        node,
        Member {
            identity,
            role,
            home,
        },
    ))
}

/// Build the genesis of a new space. Returns the signed genesis proposal and
/// its payload; pass both to [`Space::genesis`].
///
/// `info` must hold `name`; the creator becomes the only owner.
pub fn genesis_proposal(
    author: &Keypair,
    identity: PublicKey,
    mut info: Json,
    time: u64,
    nonce: Vec<u8>,
) -> (Vec<u8>, Vec<u8>) {
    if let Some(obj) = info.as_object_mut() {
        obj.insert("primary".into(), Json::String(author.public().to_id()));
        obj.insert(
            "members".into(),
            serde_json::json!([{ "node": author.public().to_id(), "identity": identity.to_id(), "role": "owner" }]),
        );
        obj.insert("created".into(), Json::from(time));
    }
    let payload = serde_json::to_vec(&info).unwrap_or_default();
    let proposal = Proposal {
        space: Hash::ZERO,
        kind: Kind::Genesis,
        target: "space/info".into(),
        payload: Some(Hash::of(&payload)),
        media_type: "application/json".into(),
        time,
        nonce,
        grant: None,
    };
    (author.sign1(proposal.encode()).encode(), payload)
}

impl Space {
    /// The ID a genesis proposal gives its space.
    pub fn id_of(genesis_proposal: &[u8]) -> Hash {
        Hash::of(genesis_proposal)
    }

    /// Start a space from its genesis. The primary signs entry 0.
    ///
    /// # Errors
    ///
    /// Returns a refusal if genesis, its payload or any required signature is invalid.
    pub fn genesis(
        primary: &Keypair,
        signed_genesis: &[u8],
        payload: &[u8],
        time: u64,
    ) -> Result<(Space, Vec<u8>, Applied), Refused> {
        let entry = Entry {
            seq: 0,
            prev: Hash::ZERO,
            time,
            proposal: signed_genesis.to_vec(),
        };
        let signed = primary.sign1(entry.encode()).encode();
        let mut space = Space::empty(Space::id_of(signed_genesis));
        let lookup = |h: &Hash| (*h == Hash::of(payload)).then(|| payload.to_vec());
        let applied = space.apply(&signed, &lookup)?;
        Ok((space, signed, applied))
    }

    fn empty(id: Hash) -> Space {
        Space {
            id,
            info: Json::Null,
            primary: PublicKey([0; 32]),
            members: BTreeMap::new(),
            resources: BTreeMap::new(),
            grants: BTreeMap::new(),
            seq: 0,
            head: Hash::ZERO,
            nonces: HashMap::new(),
        }
    }

    /// Rebuild a space by applying signed entries in order.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the log is empty or an entry fails verification or application.
    pub fn replay<'a>(
        id: Hash,
        entries: impl IntoIterator<Item = &'a [u8]>,
        payloads: &dyn Payloads,
    ) -> Result<(Space, Vec<Applied>), Refused> {
        let mut space = Space::empty(id);
        let mut applied = Vec::new();
        for e in entries {
            applied.push(space.apply(e, payloads)?);
        }
        if applied.is_empty() {
            return refused("empty log");
        }
        Ok((space, applied))
    }

    /// The space's name.
    pub fn name(&self) -> &str {
        self.info.get("name").and_then(Json::as_str).unwrap_or("")
    }

    /// The role of a node, if it is a member.
    pub fn role(&self, node: &PublicKey) -> Option<Role> {
        self.members.get(node).map(|m| m.role)
    }

    /// Check a signed proposal against the current state without applying it.
    ///
    /// # Errors
    ///
    /// Returns a refusal for an invalid signature, space, author, role or missing required payload.
    pub fn check(&self, signed_proposal: &[u8]) -> Result<(PublicKey, Proposal), Refused> {
        let msg = Sign1::decode(signed_proposal).map_err(|e| Refused(e.to_string()))?;
        let author = verify1(&msg).ok_or_else(|| Refused("bad proposal signature".into()))?;
        let p = Proposal::decode(&msg.payload).map_err(|e| Refused(e.to_string()))?;
        if p.kind == Kind::Genesis {
            return refused("genesis only starts a space");
        }
        if p.space != self.id {
            return refused("proposal is for another space");
        }
        let role = self
            .role(&author)
            .ok_or_else(|| Refused("author is not a member".into()))?;
        let needed = match p.kind {
            Kind::Put | Kind::Delete | Kind::Erase => Role::Writer,
            Kind::Mls => Role::Reader,
            Kind::Genesis | Kind::Member | Kind::Grant | Kind::Revoke | Kind::Handover => {
                Role::Owner
            }
        };
        if role < needed {
            return refused(format!("a {} cannot {}", role.as_str(), p.kind.as_str()));
        }
        if matches!(
            p.kind,
            Kind::Put | Kind::Member | Kind::Grant | Kind::Handover | Kind::Mls
        ) && p.payload.is_none()
        {
            return refused("missing payload");
        }
        Ok((author, p))
    }

    /// As the primary: order a signed proposal into the log.
    ///
    /// # Errors
    ///
    /// Returns a refusal when the primary, proposal or state transition is invalid.
    pub fn order(
        &mut self,
        primary: &Keypair,
        signed_proposal: &[u8],
        payloads: &dyn Payloads,
        time: u64,
    ) -> Result<Ordered, Refused> {
        self.order_with(primary, signed_proposal, payloads, time, |_| Ok(()))
    }

    /// Order a proposal, calling `persist` before publishing the change.
    ///
    /// # Errors
    ///
    /// Returns the validation or persistence error. In either case the space
    /// remains unchanged. Duplicate proposals skip persistence.
    pub fn order_with<E: From<Refused>>(
        &mut self,
        primary: &Keypair,
        signed_proposal: &[u8],
        payloads: &dyn Payloads,
        time: u64,
        persist: impl FnOnce(&[u8]) -> Result<(), E>,
    ) -> Result<Ordered, E> {
        self.order_checked(
            primary,
            signed_proposal,
            payloads,
            time,
            |_| Ok(()),
            persist,
        )
    }

    /// Check the resource's current payload before ordering a new proposal.
    ///
    /// Duplicate proposals reuse their original sequence without running the check.
    ///
    /// # Errors
    ///
    /// Returns an error if the proposal, check or persistence fails.
    pub fn order_checked<E: From<Refused>>(
        &mut self,
        primary: &Keypair,
        signed_proposal: &[u8],
        payloads: &dyn Payloads,
        time: u64,
        check: impl FnOnce(Option<Hash>) -> Result<(), E>,
        persist: impl FnOnce(&[u8]) -> Result<(), E>,
    ) -> Result<Ordered, E> {
        if primary.public() != self.primary {
            return Err(Refused("not the primary of this space".into()).into());
        }
        let (author, p) = self.check(signed_proposal)?;
        if let Some(seq) = self.nonces.get(&(author, p.nonce.clone())) {
            return Ok(Ordered::Duplicate(*seq));
        }
        check(
            self.resources
                .get(&p.target)
                .and_then(|resource| resource.payload),
        )?;
        let entry = Entry {
            seq: self
                .seq
                .checked_add(1)
                .ok_or_else(|| Refused("space sequence exhausted".into()))?,
            prev: self.head,
            time,
            proposal: signed_proposal.to_vec(),
        };
        let signed = primary.sign1(entry.encode()).encode();
        let applied = self.apply_with(&signed, payloads, persist)?;
        Ok(Ordered::New(signed, applied))
    }

    /// Verify a signed entry from the primary and apply it.
    ///
    /// # Errors
    ///
    /// Returns a refusal if the entry or its state transition is invalid.
    pub fn apply(
        &mut self,
        signed_entry: &[u8],
        payloads: &dyn Payloads,
    ) -> Result<Applied, Refused> {
        self.apply_with(signed_entry, payloads, |_| Ok(()))
    }

    /// Apply a verified entry only after `persist` succeeds.
    ///
    /// # Errors
    ///
    /// Returns the validation or persistence error and leaves the space unchanged.
    pub fn apply_with<E: From<Refused>>(
        &mut self,
        signed_entry: &[u8],
        payloads: &dyn Payloads,
        persist: impl FnOnce(&[u8]) -> Result<(), E>,
    ) -> Result<Applied, E> {
        let msg = Sign1::decode(signed_entry).map_err(|e| Refused(e.to_string()))?;
        let signer = verify1(&msg).ok_or_else(|| Refused("bad entry signature".into()))?;
        let entry = Entry::decode(&msg.payload).map_err(|e| Refused(e.to_string()))?;
        let (author, p, update) = self.prepare(&entry, signer, payloads)?;
        let hash = Hash::of(signed_entry);
        persist(signed_entry)?;

        let mut erase = Vec::new();
        match update {
            Update::Genesis(info, primary, members) => {
                self.info = info;
                self.primary = primary;
                self.members = members;
            }
            Update::Resource => {
                let r = self
                    .resources
                    .entry(p.target.clone())
                    .or_insert_with(|| Resource {
                        payload: None,
                        media_type: String::new(),
                        seq: 0,
                        time: 0,
                        history: Vec::new(),
                    });
                r.payload = if p.kind == Kind::Put { p.payload } else { None };
                r.media_type.clone_from(&p.media_type);
                r.seq = entry.seq;
                r.time = entry.time;
                r.history.extend(p.payload);
            }
            Update::Erase => {
                if let Some(r) = self.resources.get_mut(&p.target) {
                    erase = std::mem::take(&mut r.history);
                    r.payload = None;
                }
            }
            Update::Member(node, Some(member)) => {
                self.members.insert(node, member);
            }
            Update::Member(node, None) => {
                self.members.remove(&node);
            }
            Update::Grant(id, body) => {
                self.grants.insert(
                    id,
                    Grant {
                        body,
                        revoked: false,
                    },
                );
            }
            Update::Revoke => {
                let id = p.target.strip_prefix("grant/").unwrap_or(&p.target);
                if let Some(g) = self.grants.get_mut(id) {
                    g.revoked = true;
                }
            }
            Update::Handover(primary) => self.primary = primary,
            Update::None => {}
        }
        self.nonces.insert((author, p.nonce.clone()), entry.seq);
        self.seq = entry.seq;
        self.head = hash;
        Ok(Applied {
            seq: entry.seq,
            hash,
            proposal: p,
            author,
            time: entry.time,
            erase,
        })
    }

    fn prepare(
        &self,
        entry: &Entry,
        signer: PublicKey,
        payloads: &dyn Payloads,
    ) -> Result<(PublicKey, Proposal, Update), Refused> {
        if entry.seq == 0 {
            return self.prepare_genesis(entry, signer, payloads);
        }
        if signer != self.primary {
            return refused("entry not signed by the primary");
        }
        if self.seq.checked_add(1) != Some(entry.seq) || entry.prev != self.head {
            return refused(format!("entry {} does not follow {}", entry.seq, self.seq));
        }
        let (author, p) = self.check(&entry.proposal)?;
        let update = match p.kind {
            Kind::Genesis | Kind::Mls => Update::None,
            Kind::Put | Kind::Delete => Update::Resource,
            Kind::Erase => Update::Erase,
            Kind::Member => {
                let body = payload_json(&p, payloads)
                    .ok_or_else(|| Refused("member payload missing".into()))?;
                let (node, member) =
                    parse_member(&body).ok_or_else(|| Refused("bad member".into()))?;
                let member =
                    (body.get("remove").and_then(Json::as_bool) != Some(true)).then_some(member);
                let keeps_owner = member.as_ref().is_some_and(|m| m.role == Role::Owner)
                    || self
                        .members
                        .iter()
                        .any(|(key, m)| *key != node && m.role == Role::Owner);
                if !keeps_owner {
                    return refused("a space needs an owner");
                }
                Update::Member(node, member)
            }
            Kind::Grant => {
                let body = payload_json(&p, payloads)
                    .ok_or_else(|| Refused("grant payload missing".into()))?;
                let id = body
                    .get("id")
                    .and_then(Json::as_str)
                    .ok_or_else(|| Refused("grant has no id".into()))?
                    .to_string();
                Update::Grant(id, body)
            }
            Kind::Revoke => Update::Revoke,
            Kind::Handover => {
                let body = payload_json(&p, payloads)
                    .ok_or_else(|| Refused("handover payload missing".into()))?;
                let primary = body
                    .get("primary")
                    .and_then(Json::as_str)
                    .and_then(PublicKey::from_id)
                    .ok_or_else(|| Refused("handover names no primary".into()))?;
                if !self.members.contains_key(&primary) {
                    return refused("the new primary must be a member");
                }
                Update::Handover(primary)
            }
        };
        Ok((author, p, update))
    }

    fn prepare_genesis(
        &self,
        entry: &Entry,
        signer: PublicKey,
        payloads: &dyn Payloads,
    ) -> Result<(PublicKey, Proposal, Update), Refused> {
        if !self.members.is_empty() {
            return refused("genesis on a started space");
        }
        if Space::id_of(&entry.proposal) != self.id || entry.prev != Hash::ZERO {
            return refused("genesis does not match the space ID");
        }
        let msg = Sign1::decode(&entry.proposal).map_err(|e| Refused(e.to_string()))?;
        let author = verify1(&msg).ok_or_else(|| Refused("bad genesis signature".into()))?;
        let p = Proposal::decode(&msg.payload).map_err(|e| Refused(e.to_string()))?;
        if p.kind != Kind::Genesis {
            return refused("entry 0 must be a genesis");
        }
        let info =
            payload_json(&p, payloads).ok_or_else(|| Refused("genesis payload missing".into()))?;
        let primary = info
            .get("primary")
            .and_then(Json::as_str)
            .and_then(PublicKey::from_id)
            .ok_or_else(|| Refused("genesis has no primary".into()))?;
        if primary != signer || primary != author {
            return refused("genesis must be signed by its primary");
        }
        let members = info
            .get("members")
            .and_then(Json::as_array)
            .into_iter()
            .flatten()
            .map(|m| parse_member(m).ok_or_else(|| Refused("bad genesis member".into())))
            .collect::<Result<_, _>>()?;
        Ok((author, p, Update::Genesis(info, primary, members)))
    }

    /// The signed head for the current state.
    pub fn head(&self, primary: &Keypair, segments: Vec<Hash>) -> Vec<u8> {
        let head = Head {
            space: self.id,
            seq: self.seq,
            hash: self.head,
            segments,
            primary: self.primary,
        };
        primary.sign1(head.encode()).encode()
    }

    /// A signed checkpoint: the state at the current sequence number, so a
    /// node can start from it instead of replaying the whole log.
    pub fn checkpoint(&self, primary: &Keypair) -> Vec<u8> {
        let members = self
            .members
            .iter()
            .map(|(k, m)| {
                Value::Array(vec![
                    Value::Bytes(k.0.to_vec()),
                    Value::Bytes(m.identity.0.to_vec()),
                    Value::Text(m.role.as_str().into()),
                    m.home.clone().map_or(Value::Null, Value::Text),
                ])
            })
            .collect();
        let resources = self
            .resources
            .iter()
            .map(|(t, r)| {
                Value::Array(vec![
                    Value::Text(t.clone()),
                    r.payload
                        .map_or(Value::Null, |h| Value::Bytes(h.0.to_vec())),
                    Value::Text(r.media_type.clone()),
                    Value::Uint(r.seq),
                    Value::Uint(r.time),
                    Value::Array(
                        r.history
                            .iter()
                            .map(|h| Value::Bytes(h.0.to_vec()))
                            .collect(),
                    ),
                ])
            })
            .collect();
        let grants = self
            .grants
            .iter()
            .map(|(id, g)| {
                Value::Array(vec![
                    Value::Text(id.clone()),
                    Value::Text(g.body.to_string()),
                    Value::Bool(g.revoked),
                ])
            })
            .collect();
        let body = map([
            (1, Value::Bytes(self.id.0.to_vec())),
            (2, Value::Uint(self.seq)),
            (3, Value::Bytes(self.head.0.to_vec())),
            (4, Value::Bytes(self.primary.0.to_vec())),
            (5, Value::Text(self.info.to_string())),
            (6, Value::Array(members)),
            (7, Value::Array(resources)),
            (8, Value::Array(grants)),
        ]);
        primary.sign1(body.encode()).encode()
    }

    /// Load a checkpoint signed by `trusted_primary`.
    ///
    /// # Errors
    ///
    /// Returns a refusal if the checkpoint is malformed or its signature does not match the trusted
    /// primary.
    pub fn from_checkpoint(bytes: &[u8], trusted_primary: &PublicKey) -> Result<Space, Refused> {
        let bad = || Refused("bad checkpoint".into());
        let msg = Sign1::decode(bytes).map_err(|_| bad())?;
        if verify1(&msg) != Some(*trusted_primary) {
            return refused("checkpoint not signed by the primary");
        }
        let v = Value::decode(&msg.payload).map_err(|_| bad())?;
        let hash = |v: Option<&Value>| v.and_then(Value::as_bytes).and_then(Hash::from_slice);
        let key = |v: Option<&Value>| v.and_then(Value::as_bytes).and_then(PublicKey::from_slice);
        let mut space = Space::empty(hash(v.get(1)).ok_or_else(bad)?);
        space.seq = v.get(2).and_then(Value::as_u64).ok_or_else(bad)?;
        space.head = hash(v.get(3)).ok_or_else(bad)?;
        space.primary = key(v.get(4)).ok_or_else(bad)?;
        space.info = serde_json::from_str(v.get(5).and_then(Value::as_text).ok_or_else(bad)?)
            .map_err(|_| bad())?;
        for m in v.get(6).and_then(Value::as_array).ok_or_else(bad)? {
            let a = m.as_array().ok_or_else(bad)?;
            space.members.insert(
                key(a.first()).ok_or_else(bad)?,
                Member {
                    identity: key(a.get(1)).ok_or_else(bad)?,
                    role: a
                        .get(2)
                        .and_then(Value::as_text)
                        .and_then(Role::parse)
                        .ok_or_else(bad)?,
                    home: a.get(3).and_then(Value::as_text).map(String::from),
                },
            );
        }
        for r in v.get(7).and_then(Value::as_array).ok_or_else(bad)? {
            let a = r.as_array().ok_or_else(bad)?;
            space.resources.insert(
                a.first()
                    .and_then(Value::as_text)
                    .ok_or_else(bad)?
                    .to_string(),
                Resource {
                    payload: hash(a.get(1)),
                    media_type: a
                        .get(2)
                        .and_then(Value::as_text)
                        .ok_or_else(bad)?
                        .to_string(),
                    seq: a.get(3).and_then(Value::as_u64).ok_or_else(bad)?,
                    time: a.get(4).and_then(Value::as_u64).ok_or_else(bad)?,
                    history: a
                        .get(5)
                        .and_then(Value::as_array)
                        .ok_or_else(bad)?
                        .iter()
                        .filter_map(|h| hash(Some(h)))
                        .collect(),
                },
            );
        }
        for g in v.get(8).and_then(Value::as_array).ok_or_else(bad)? {
            let a = g.as_array().ok_or_else(bad)?;
            space.grants.insert(
                a.first()
                    .and_then(Value::as_text)
                    .ok_or_else(bad)?
                    .to_string(),
                Grant {
                    body: serde_json::from_str(a.get(1).and_then(Value::as_text).ok_or_else(bad)?)
                        .map_err(|_| bad())?,
                    revoked: a.get(2) == Some(&Value::Bool(true)),
                },
            );
        }
        Ok(space)
    }
}

/// Build and sign a proposal.
#[expect(
    clippy::too_many_arguments,
    reason = "the signature carries the signed proposal fields"
)]
pub fn propose(
    author: &Keypair,
    space: Hash,
    kind: Kind,
    target: &str,
    payload: Option<&[u8]>,
    media_type: &str,
    time: u64,
    nonce: Vec<u8>,
    grant: Option<String>,
) -> Vec<u8> {
    let p = Proposal {
        space,
        kind,
        target: target.into(),
        payload: payload.map(Hash::of),
        media_type: media_type.into(),
        time,
        nonce,
        grant,
    };
    author.sign1(p.encode()).encode()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Store(RefCell<HashMap<Hash, Vec<u8>>>);
    impl Store {
        fn put(&self, b: &[u8]) -> Hash {
            let h = Hash::of(b);
            self.0.borrow_mut().insert(h, b.to_vec());
            h
        }
    }
    impl Payloads for Store {
        fn payload(&self, h: &Hash) -> Option<Vec<u8>> {
            self.0.borrow().get(h).cloned()
        }
    }

    #[test]
    fn order_apply_replay_checkpoint() {
        let alice_node = Keypair::from_seed([1; 32]);
        let alice = PublicKey([11; 32]);
        let bob_node = Keypair::from_seed([2; 32]);
        let bob = PublicKey([22; 32]);
        let store = Store(RefCell::new(HashMap::new()));

        let (g, gp) = genesis_proposal(
            &alice_node,
            alice,
            serde_json::json!({"name": "family"}),
            1,
            vec![0],
        );
        store.put(&gp);
        let (mut space, e0, _) = Space::genesis(&alice_node, &g, &gp, 1).unwrap();
        let mut log = vec![e0];
        assert_eq!(space.name(), "family");

        // Bob is not a member yet.
        let note = br#"{"text":"hi"}"#;
        store.put(note);
        let p = propose(
            &bob_node,
            space.id,
            Kind::Put,
            "note/1",
            Some(note),
            "application/json",
            2,
            vec![1],
            None,
        );
        assert!(space.order(&alice_node, &p, &store, 2).is_err());

        let m = serde_json::to_vec(&serde_json::json!({"node": bob_node.public().to_id(), "identity": bob.to_id(), "role": "writer"})).unwrap();
        store.put(&m);
        let pm = propose(
            &alice_node,
            space.id,
            Kind::Member,
            "member/bob",
            Some(&m),
            "application/json",
            3,
            vec![2],
            None,
        );
        let Ordered::New(e1, _) = space.order(&alice_node, &pm, &store, 3).unwrap() else {
            panic!()
        };
        log.push(e1);

        let Ordered::New(e2, a) = space.order(&alice_node, &p, &store, 4).unwrap() else {
            panic!()
        };
        assert_eq!(a.author, bob_node.public());
        log.push(e2);
        // Retrying the same proposal is idempotent.
        assert_eq!(
            space.order(&alice_node, &p, &store, 5).unwrap(),
            Ordered::Duplicate(2)
        );

        // A writer cannot add members.
        let pm2 = propose(
            &bob_node,
            space.id,
            Kind::Member,
            "member/x",
            Some(&m),
            "application/json",
            6,
            vec![3],
            None,
        );
        assert!(space.order(&alice_node, &pm2, &store, 6).is_err());

        // Erase drops the payload but the chain still verifies without it.
        let pe = propose(
            &alice_node,
            space.id,
            Kind::Erase,
            "note/1",
            None,
            "",
            7,
            vec![4],
            None,
        );
        let Ordered::New(e3, a) = space.order(&alice_node, &pe, &store, 7).unwrap() else {
            panic!()
        };
        assert_eq!(a.erase, vec![Hash::of(note)]);
        store.0.borrow_mut().remove(&Hash::of(note));
        log.push(e3);

        let (replayed, applied) =
            Space::replay(space.id, log.iter().map(Vec::as_slice), &store).unwrap();
        assert_eq!(applied.len(), 4);
        assert_eq!(replayed.head, space.head);
        assert_eq!(replayed.resources, space.resources);

        // Tampering breaks the chain.
        let mut bad = log.clone();
        bad.swap(1, 2);
        assert!(Space::replay(space.id, bad.iter().map(Vec::as_slice), &store).is_err());

        let cp = space.checkpoint(&alice_node);
        let restored = Space::from_checkpoint(&cp, &alice_node.public()).unwrap();
        assert_eq!(restored.resources, space.resources);
        assert_eq!(restored.members, space.members);
        assert!(Space::from_checkpoint(&cp, &bob_node.public()).is_err());
    }

    #[test]
    fn handover() {
        let a = Keypair::from_seed([1; 32]);
        let b = Keypair::from_seed([2; 32]);
        let store = Store(RefCell::new(HashMap::new()));
        let (g, gp) = genesis_proposal(
            &a,
            PublicKey([9; 32]),
            serde_json::json!({"name": "s"}),
            1,
            vec![],
        );
        store.put(&gp);
        let (mut space, _, _) = Space::genesis(&a, &g, &gp, 1).unwrap();
        let m = serde_json::to_vec(&serde_json::json!({"node": b.public().to_id(), "identity": PublicKey([9; 32]).to_id(), "role": "owner"})).unwrap();
        store.put(&m);
        let pm = propose(
            &a,
            space.id,
            Kind::Member,
            "member/b",
            Some(&m),
            "application/json",
            2,
            vec![1],
            None,
        );
        space.order(&a, &pm, &store, 2).unwrap();
        let h = serde_json::to_vec(&serde_json::json!({"primary": b.public().to_id()})).unwrap();
        store.put(&h);
        let ph = propose(
            &a,
            space.id,
            Kind::Handover,
            "space/primary",
            Some(&h),
            "application/json",
            3,
            vec![2],
            None,
        );
        space.order(&a, &ph, &store, 3).unwrap();
        assert_eq!(space.primary, b.public());
        let p = propose(
            &a,
            space.id,
            Kind::Delete,
            "note/x",
            None,
            "",
            4,
            vec![3],
            None,
        );
        assert!(space.order(&a, &p, &store, 4).is_err());
        assert!(space.order(&b, &p, &store, 4).is_ok());
    }
}
