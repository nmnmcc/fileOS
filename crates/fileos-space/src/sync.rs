// SPDX-License-Identifier: AGPL-3.0-only

//! A node with many spaces, and the messages between nodes, with no I/O.
//!
//! A [`Node`] takes messages from peers and gives back messages to send and
//! events for the caller. It never touches the network or the disk itself.
//!
//! The rules:
//!
//! - When two nodes connect, each says which spaces it has and how long each
//!   log is (`Have`), but only for spaces that the other may keep. Each sends
//!   the other the entries it does not have. A node that sees that the other
//!   has more says what it has, so that the other sends the rest.
//! - A node writes at any time, also when offline. New entries go to every
//!   connected node that may keep the space, and every node that gets new
//!   entries sends them on in the same way.
//! - A node may keep a space when its key has `data.keep` or `files.read` on
//!   the whole space. (Reading part of a space, by prefix or by file, is done
//!   through a node that has it all, until keys follow prefixes.)
//! - Content is asked for from the node that sent the entry, and is given
//!   only to nodes that may keep a space that has it.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use fileos_core::{Bytes, Error, Hash, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};

use crate::content;
use crate::crypto::{self, Key, Random, Recipient, Wrapped};
use crate::entry::{
    Act, Content, Entry, Id, KeyRef, Permission, Principal, Resource, Retention, Sealed,
};
use crate::space::{Keyring, Snapshot, Space, Version};

/// What [`Node::seal`] gives: the content, the wrapped data key, and the
/// pieces for the content store.
pub type Sealing = (Content, Option<Wrapped>, Vec<(Hash, Vec<u8>)>);

/// A message between two nodes.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Msg {
    /// The spaces I have, with the length of each log.
    Have(Vec<(Hash, BTreeMap<PublicKey, u64>)>),
    /// Send me this space.
    Want(Hash),
    Entries {
        space: Hash,
        entries: Vec<Entry>,
    },
    /// The state at a checkpoint, for a node that lacks entries before it.
    Snapshot {
        space: Hash,
        snapshot: Box<Snapshot>,
    },
    WantContent(Hash),
    Content(Bytes),
    /// I am a node of an identity: its DID operations and my certificate.
    Iam {
        ops: Vec<Bytes>,
        cert: Bytes,
    },
    /// Nodes that are no longer nodes of an identity: revocations signed by
    /// its signing key.
    Revoked(Vec<Bytes>),
}

/// Something the caller acts on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    /// The state of a space changed.
    Changed(Hash),
    /// Content came. The caller keeps it.
    Content { from: PublicKey, data: Vec<u8> },
    /// Acts of this node made at the same time as a checkpoint had no
    /// effect, and were written again on top. The person is told.
    Again { space: Hash, entries: usize },
}

/// The output of one step.
#[derive(Debug, Default)]
pub struct Step {
    pub out: Vec<(PublicKey, Msg)>,
    pub events: Vec<Event>,
}

/// What the node needs to know about the content store.
pub trait Contents {
    fn has(&self, hash: Hash) -> bool;
    fn get(&self, hash: Hash) -> Option<Vec<u8>>;
}

/// What a node shows about its identity: the identity, the operations of
/// its DID log, and the node's certificate.
type Proof = (Hash, Vec<Bytes>, Bytes);

/// The most content requests in flight on one node. Leave room in the
/// transport for entries and for replies to other nodes.
const CONTENT_IN_FLIGHT: usize = 16;
/// A request can take three retry ticks. The host gives a tick every five seconds.
const REQUEST_TICKS: u64 = 3;

#[derive(Debug)]
struct Request {
    from: PublicKey,
    deadline: u64,
    list: Option<Content>,
}

/// The spaces on one node, and its peers.
#[derive(Debug)]
pub struct Node {
    key: SecretKey,
    spaces: BTreeMap<Hash, Space>,
    /// Spaces to get, whose first entry this node does not have yet.
    wanted: BTreeSet<Hash>,
    peers: BTreeSet<PublicKey>,
    /// Peers that have offered entries for a space.
    sources: BTreeMap<PublicKey, BTreeSet<Hash>>,
    asked: BTreeMap<Hash, Request>,
    ticks: u64,
    nonce: u64,
    /// The identity this node is a node of: its DID operations and the
    /// certificate of this node.
    identity: Option<Proof>,
    /// What connected peers showed about their identities.
    proofs: BTreeMap<PublicKey, Proof>,
    /// The vouches and unvouches this node has tried to write, by (space,
    /// node, hash of the certificate or revocation), so each is tried once
    /// and writing one never starts writing it again.
    tried: BTreeSet<(Hash, PublicKey, Hash)>,
    /// Node revocations this node knows: (the identity, the node, the bytes).
    revocations: Vec<(Hash, PublicKey, Bytes)>,
    /// The newest time this node has seen, for what it writes when a
    /// message comes (this crate has no clock).
    time: u64,
    /// Other keys this node signs with, such as the signing key of an
    /// identity that it holds.
    keys: Vec<SecretKey>,
    /// Keys that other nodes hold too, such as app keys.
    shared: Vec<SecretKey>,
    random: Random,
    /// The opened keys of each space, with the heads they were opened at.
    rings: RefCell<BTreeMap<Hash, (Vec<Hash>, Keyring)>>,
    /// The recipient keys of this node, with the time each was made, the
    /// newest last. Keys are sealed to the newest; the one before is kept
    /// until keys sealed to it are sealed again to the newest.
    recipients: Vec<(u64, Recipient)>,
}

impl Node {
    /// `nonce` should be random, so that spaces made after a restart are
    /// different.
    pub fn new(key: SecretKey, nonce: u64) -> Self {
        let random = Random::new(blake3::derive_key(
            "fileOS 2026 node random",
            &[&key.seed()[..], &nonce.to_le_bytes()].concat(),
        ));
        Self {
            keys: Vec::new(),
            shared: Vec::new(),
            random,
            rings: RefCell::new(BTreeMap::new()),
            recipients: Vec::new(),
            key,
            spaces: BTreeMap::new(),
            wanted: BTreeSet::new(),
            peers: BTreeSet::new(),
            sources: BTreeMap::new(),
            asked: BTreeMap::new(),
            ticks: 0,
            nonce,
            identity: None,
            proofs: BTreeMap::new(),
            revocations: Vec::new(),
            tried: BTreeSet::new(),
            time: 0,
        }
    }

    pub fn me(&self) -> PublicKey {
        self.key.public()
    }

    /// Lets this node also sign with `key`, and open keys sealed to it.
    pub fn add_key(&mut self, key: SecretKey) {
        let known = self
            .keys
            .iter()
            .chain(&self.shared)
            .any(|k| k.public() == key.public());
        if key.public() != self.me() && !known {
            self.keys.push(key);
        }
    }

    fn mine(&self) -> Vec<SecretKey> {
        std::iter::once(self.key.clone())
            .chain(self.keys.iter().cloned())
            .chain(self.shared.iter().cloned())
            .collect()
    }

    /// Lets this node sign with `key`, a key that other nodes hold too, such
    /// as the key of an app that runs on every node of a person. Keys for it
    /// are sealed to a recipient key made from the key itself, so every
    /// node that holds it can open them.
    pub fn add_shared_key(&mut self, key: SecretKey) {
        if !self.shared.iter().any(|k| k.public() == key.public()) {
            self.shared.push(key);
            self.rings.borrow_mut().clear();
        }
    }

    /// The recipient key of a shared key: the same on every node.
    fn shared_recipient(key: &SecretKey) -> Recipient {
        Recipient::from_secret(blake3::derive_key(
            "fileOS 2026 shared key recipient",
            &key.seed(),
        ))
    }

    /// The recipient key this node shows for `key`.
    fn recipient_for(&self, key: &SecretKey) -> Option<Key> {
        if self.shared.iter().any(|k| k.public() == key.public()) {
            Some(Self::shared_recipient(key).public)
        } else {
            self.recipients.last().map(|(_, r)| r.public)
        }
    }

    /// The keys this node has opened in a space.
    pub fn keyring(&self, space: Hash) -> Keyring {
        let Some(s) = self.spaces.get(&space) else {
            return Keyring::default();
        };
        let heads = s.heads();
        if let Some((at, ring)) = self.rings.borrow().get(&space)
            && *at == heads
        {
            return ring.clone();
        }
        let mine = self.mine();
        let recipients: Vec<Recipient> = self
            .recipients
            .iter()
            .map(|(_, r)| r.clone())
            .chain(self.shared.iter().map(Self::shared_recipient))
            .collect();
        let ring = s.keyring_with(&mine.iter().collect::<Vec<_>>(), &recipients);
        self.rings.borrow_mut().insert(space, (heads, ring.clone()));
        ring
    }

    /// The recipient keys this node holds, as (time made, secret), to keep
    /// on disk.
    pub fn recipients(&self) -> Vec<(u64, Key)> {
        self.recipients
            .iter()
            .map(|(t, r)| (*t, r.secret()))
            .collect()
    }

    /// Puts the recipient keys kept on disk back in place.
    pub fn set_recipients(&mut self, kept: Vec<(u64, Key)>) {
        self.recipients = kept
            .into_iter()
            .map(|(t, k)| (t, Recipient::from_secret(k)))
            .collect();
        self.rings.borrow_mut().clear();
    }

    /// The time the newest recipient key was made, if there is one.
    pub fn recipient_made(&self) -> Option<u64> {
        self.recipients.last().map(|(t, _)| *t)
    }

    /// Makes a new recipient key, and destroys all but the one before it.
    /// When to do it is the caller's policy (for example every 30 days). The
    /// new key goes into every space this node may read, and other nodes
    /// then seal keys to it.
    pub fn rotate_recipient(&mut self, time: u64) -> Step {
        let new = Recipient::new(&mut self.random);
        self.recipients.push((time, new));
        if self.recipients.len() > 2 {
            self.recipients.drain(..self.recipients.len() - 2);
        }
        self.rings.borrow_mut().clear();
        let mut step = Step::default();
        let ids: Vec<Hash> = self.spaces.keys().copied().collect();
        for id in ids {
            self.upkeep_at(id, time, &mut step);
        }
        step
    }

    /// Seals `keys` for the key `to`, to its newest recipient key in the
    /// space. Never to the key itself: it cannot be changed, so whoever
    /// steals it later could open every key ever sealed to it.
    fn seal_for(
        &mut self,
        space: Hash,
        to: PublicKey,
        refs: Vec<KeyRef>,
        keys: &[u8],
    ) -> Option<Sealed> {
        let (recipient, _) = self.spaces.get(&space)?.recipient_of(to)?;
        let (enc, sealed) =
            crypto::seal_to_recipient(&recipient, &space.0, keys, &mut self.random)?;
        Some(Sealed {
            to,
            refs,
            enc,
            keys: sealed,
            recipient: Some(Bytes(recipient.to_vec())),
        })
    }

    /// Makes content for a file named `name` (the file `file`, when it is
    /// there): encrypted under the newest epoch of the space, with the data
    /// key wrapped by the key of the prefix it is in. Gives back the content,
    /// the wrapped key, and the pieces for the content store. With no epoch
    /// in the space, the content is not encrypted.
    ///
    /// # Errors
    ///
    /// When the space is not here, or this node has no key for that prefix.
    pub fn seal(
        &mut self,
        space: Hash,
        name: &str,
        file: Option<Id>,
        data: &[u8],
    ) -> Result<Sealing, Error> {
        let s = self
            .spaces
            .get(&space)
            .ok_or_else(|| Error::unavailable(format!("space {space} is not here")))?;
        let Some(epoch) = s.epoch() else {
            let (content, pieces) = content::split(data);
            return Ok((content, None, pieces));
        };
        let ring = self.keyring(space);
        let s = &self.spaces[&space];
        let prefix = crypto::parent(name);
        let kek = ring.prefix(epoch, &prefix).ok_or_else(|| {
            Error::unavailable(format!("this node has no key for {prefix:?} yet"))
        })?;
        // The same data key while the epoch is the same, so chunks that do
        // not change are kept once.
        let same = file
            .and_then(|id| s.file(id))
            .and_then(|f| {
                let v = f.versions.iter().max_by_key(|v| (v.time, v.id))?.clone();
                (v.key.as_ref()?.epoch == epoch).then(|| s.data_keys(&ring, f.id, &v))
            })
            .and_then(|keys| keys.into_iter().next());
        let data_key = same.unwrap_or_else(|| self.random.key());
        let (content, pieces) = content::seal(data, &data_key);
        let wrapped = crypto::wrap(&kek, epoch, &prefix, &data_key, &mut self.random);
        Ok((content, Some(wrapped), pieces))
    }

    /// The bytes of `version` of `file`, if this node can open them and all
    /// of the content is here.
    pub fn open(
        &self,
        space: Hash,
        file: Id,
        version: &Version,
        contents: &dyn Contents,
    ) -> Option<Vec<u8>> {
        self.reader(space, file, version)?.join(contents)
    }

    /// Prepares a read of one file version with the keys this node holds.
    pub fn reader(&self, space: Hash, file: Id, version: &Version) -> Option<content::Reader> {
        let content = version.content?;
        if version.key.is_none() {
            return Some(content::Reader::plain(content));
        }
        let ring = self.keyring(space);
        let space = self.spaces.get(&space)?;
        content::Reader::sealed(content, space.data_keys(&ring, file, version))
    }

    /// Keeps the keys of a space in order, with the keys this node holds:
    /// starts a new epoch when access was taken back (or there is none),
    /// and sends readers the keys they do not have yet.
    fn upkeep(&mut self, space: Hash, step: &mut Step) {
        self.upkeep_at(space, 0, step);
    }

    /// [`upkeep`](Self::upkeep), when the time now is `now` (or the newest
    /// time in the space, if later).
    fn upkeep_at(&mut self, space: Hash, now: u64, step: &mut Step) {
        let Some(s) = self.spaces.get(&space) else {
            return;
        };
        let time = s.latest_time().max(now);
        // Show that this node is a node of its identity, in a space that
        // names the identity and does not show it yet.
        if let Some((did, ops, cert)) = self.identity.clone()
            && s.identities().contains(&did)
            && !s.is_node_of(self.me(), did)
        {
            self.vouch(did, self.me(), &ops, &cert, time, step);
        }
        // Show this node's newest recipient key for each key it signs with,
        // so keys for them are sealed to it.
        if self.recipients.last().is_some() {
            for key in self.mine() {
                let Some(public) = self.recipient_for(&key) else {
                    continue;
                };
                let s = &self.spaces[&space];
                if s.recipient_of(key.public()).map(|(k, _)| k) == Some(public) {
                    continue;
                }
                let act = Act::Recipient {
                    key: Bytes(public.to_vec()),
                };
                if let Ok(more) = self.write_inner(space, &key, time, vec![act]) {
                    step.out.extend(more.out);
                    step.events.extend(more.events);
                }
            }
        }
        let Some(s) = self.spaces.get(&space) else {
            return;
        };
        let whole = Resource::Prefix(String::new());
        let mine = self.mine();
        if s.needs_new_epoch(time)
            && let Some(author) = mine.iter().find(|k| {
                s.may(
                    Principal::Key(k.public()),
                    Permission::SpaceCheckpoint,
                    &whole,
                )
            })
        {
            let ring = self.keyring(space);
            let s = &self.spaces[&space];
            let epoch = Hash(self.random.key());
            let root = self.random.key();
            let mut rewrap = Vec::new();
            for f in s.files() {
                let Some(v) = f.versions.iter().max_by_key(|v| (v.time, v.id)) else {
                    continue;
                };
                if v.key.is_none() {
                    continue;
                }
                if let Some(dk) = s.data_keys(&ring, f.id, v).into_iter().next() {
                    let prefix = crypto::parent(&v.name);
                    let kek = crypto::prefix_key(&root, &prefix);
                    rewrap.push((
                        f.id,
                        crypto::wrap(&kek, epoch, &prefix, &dk, &mut self.random),
                    ));
                }
            }
            if let Some(sealed) =
                self.seal_for(space, author.public(), vec![KeyRef::Root(epoch)], &root)
            {
                let act = Act::Keys {
                    epoch: Some(epoch),
                    sealed: vec![sealed],
                    rewrap,
                };
                let author = author.clone();
                if let Ok(more) = self.write_inner(space, &author, time, vec![act]) {
                    step.out.extend(more.out);
                    step.events.extend(more.events);
                }
            }
        }
        // Send the keys that readers lack, with each key this node holds
        // that may give them.
        let gaps = self.spaces[&space].key_gaps();
        if gaps.is_empty() {
            return;
        }
        let ring = self.keyring(space);
        for author in &mine {
            let s = &self.spaces[&space];
            let who = Principal::Key(author.public());
            // Who may give the access may give the key; a reader may give it
            // to a device that may read it too (a gap means it may).
            let may = |r: &KeyRef| {
                let on = match r {
                    KeyRef::Root(_) => return s.may(who, Permission::SpaceCheckpoint, &whole),
                    KeyRef::Prefix(_, p) => Resource::Prefix(p.clone()),
                    KeyRef::File(id, _) => Resource::File(*id),
                };
                s.may(who, Permission::AccessGive, &on) || s.may(who, Permission::FilesRead, &on)
            };
            let mut by_device: BTreeMap<PublicKey, (Vec<KeyRef>, Vec<u8>)> = BTreeMap::new();
            // The data key of a file is unwrapped from its newest version.
            let get = |r: &KeyRef| match r {
                KeyRef::File(id, epoch) => s.data_key_in(&ring, *id, *epoch),
                other => ring.get(other),
            };
            for (to, r) in &gaps {
                // Keys sealed to this node's own keys give it nothing new.
                if mine.iter().any(|k| k.public() == *to) {
                    continue;
                }
                if let Some(key) = get(r)
                    && may(r)
                {
                    let entry = by_device.entry(*to).or_default();
                    entry.0.push(r.clone());
                    entry.1.extend_from_slice(&key);
                }
            }
            if by_device.is_empty() {
                continue;
            }
            let mut sealed = Vec::new();
            for (to, (refs, keys)) in by_device {
                if let Some(x) = self.seal_for(space, to, refs, &keys) {
                    sealed.push(x);
                }
            }
            let act = Act::Keys {
                epoch: None,
                sealed,
                rewrap: Vec::new(),
            };
            if let Ok(more) = self.write_inner(space, author, time, vec![act]) {
                step.out.extend(more.out);
                step.events.extend(more.events);
            }
            break;
        }
    }

    pub fn space(&self, id: Hash) -> Option<&Space> {
        self.spaces.get(&id)
    }

    pub fn spaces(&self) -> impl Iterator<Item = &Space> {
        self.spaces.values()
    }

    /// Puts a space that was kept on disk back in place.
    pub fn insert(&mut self, space: Space) {
        self.wanted.remove(&space.id());
        self.spaces.insert(space.id(), space);
    }

    fn may_keep(space: &Space, peer: PublicKey) -> bool {
        let whole = Resource::Prefix(String::new());
        let who = Principal::Key(peer);
        space.may(who, Permission::DataKeep, &whole)
            || space.may(who, Permission::FilesRead, &whole)
    }

    /// Makes a space owned by this node's key. `time` is now, in
    /// milliseconds since 1970.
    pub fn create(&mut self, time: u64) -> (Hash, Step) {
        self.nonce += 1;
        let (space, _) = Space::create(Principal::Key(self.me()), &self.key, time, self.nonce);
        let id = space.id();
        self.spaces.insert(id, space);
        let mut step = Step::default();
        step.events.push(Event::Changed(id));
        self.upkeep(id, &mut step);
        (id, step)
    }

    /// Makes a space owned by `owner`, which also signs its first entry: for
    /// example the signing key of an identity that this node holds.
    pub fn create_as(&mut self, owner: &SecretKey, time: u64) -> (Hash, Step) {
        self.nonce += 1;
        self.add_key(owner.clone());
        let (space, _) = Space::create(Principal::Key(owner.public()), owner, time, self.nonce);
        let id = space.id();
        self.spaces.insert(id, space);
        let mut step = Step::default();
        step.events.push(Event::Changed(id));
        self.upkeep(id, &mut step);
        (id, step)
    }

    /// Makes a space owned by `owner`, such as an identity: this node signs
    /// its first entry. A node acts for the owner only as the space shows
    /// (for an identity, with a `Vouch`).
    pub fn create_owned(&mut self, owner: Principal, time: u64) -> (Hash, Step) {
        self.nonce += 1;
        let (space, _) = Space::create(owner, &self.key, time, self.nonce);
        let id = space.id();
        self.spaces.insert(id, space);
        let mut step = Step::default();
        step.events.push(Event::Changed(id));
        (id, step)
    }

    /// Makes this node a node of an identity: `ops` is its DID log, and
    /// `cert` the certificate of this node. Connected peers are told.
    ///
    /// # Errors
    ///
    /// When the certificate is not for this node, or not signed by the
    /// signing key of the DID log.
    pub fn set_identity(&mut self, ops: Vec<Bytes>, cert: Bytes) -> Result<Step, Error> {
        let did = Self::check_proof(self.me(), &ops, &cert)?;
        let mut step = Step::default();
        if self
            .identity
            .as_ref()
            .is_some_and(|(d, o, c)| *d == did && *o == ops && *c == cert)
        {
            return Ok(step);
        }
        self.identity = Some((did, ops.clone(), cert.clone()));
        // Show it in the spaces here that name the identity, such as a
        // space the identity owns.
        let time = self.last_time();
        self.vouch(did, self.me(), &ops, &cert, time, &mut step);
        for peer in &self.peers {
            step.out.push((
                *peer,
                Msg::Iam {
                    ops: ops.clone(),
                    cert: cert.clone(),
                },
            ));
        }
        Ok(step)
    }

    /// Checks that `cert` says `node` is a node of the identity whose log
    /// is `ops`. Gives the identity.
    fn check_proof(node: PublicKey, ops: &[Bytes], cert: &Bytes) -> Result<Hash, Error> {
        let cert = fileos_id::NodeCert::from_bytes(&cert.0)?;
        let did = fileos_id::did_hash(&cert.did)?;
        let ops: Vec<fileos_id::Op> = ops
            .iter()
            .map(|o| fileos_id::Op::from_bytes(&o.0))
            .collect::<Result<_, _>>()?;
        let signing = fileos_id::resolve(&cert.did, &ops)?
            .state
            .and_then(|s| s.signing_key())
            .ok_or_else(|| Error::invalid("the identity has no signing key".into()))?;
        if cert.node != node || !cert.verify(signing) {
            return Err(Error::denied(
                "the certificate is not good for this node".into(),
            ));
        }
        Ok(did)
    }

    /// Takes node revocations: from the identity space of this node, or
    /// from a peer. Writes an `Unvouch` into each space here that still
    /// counts the node as a node of its identity, and tells connected peers.
    pub fn add_revocations(&mut self, revocations: Vec<Bytes>) -> Step {
        let mut step = Step::default();
        let mut new = Vec::new();
        for bytes in revocations {
            let Ok(r) = fileos_id::NodeRevocation::from_bytes(&bytes.0) else {
                continue;
            };
            let Ok(did) = fileos_id::did_hash(&r.did) else {
                continue;
            };
            if !self.revocations.iter().any(|(_, _, b)| *b == bytes) {
                self.revocations.push((did, r.node, bytes.clone()));
                new.push(bytes);
            }
        }
        if !new.is_empty() {
            for peer in &self.peers {
                step.out.push((*peer, Msg::Revoked(new.clone())));
            }
            self.unvouch_all(&mut step);
        }
        step
    }

    /// Writes the `Unvouch`es that the spaces here still lack.
    fn unvouch_all(&mut self, step: &mut Step) {
        let time = self.last_time();
        let todo: Vec<(Hash, PublicKey, Bytes)> = self
            .revocations
            .iter()
            .flat_map(|(did, node, bytes)| {
                self.spaces
                    .values()
                    .filter(|s| s.is_node_of(*node, *did))
                    .filter(|s| !self.tried.contains(&(s.id(), *node, Hash::of(&bytes.0))))
                    .map(|s| (s.id(), *node, bytes.clone()))
                    .collect::<Vec<_>>()
            })
            .collect();
        for (space, node, bytes) in todo {
            if !self.tried.insert((space, node, Hash::of(&bytes.0))) {
                continue;
            }
            let act = Act::Unvouch { revocation: bytes };
            if let Ok((_, more)) = self.write(space, time, vec![act]) {
                step.out.extend(more.out);
                step.events.extend(more.events);
            }
        }
    }

    /// Writes a `Vouch` for `node` into each space here that names `did` and
    /// does not show yet that `node` is its node.
    fn vouch(
        &mut self,
        did: Hash,
        node: PublicKey,
        ops: &[Bytes],
        cert: &Bytes,
        time: u64,
        step: &mut Step,
    ) {
        if self
            .revocations
            .iter()
            .any(|(d, n, _)| *d == did && *n == node)
        {
            return;
        }
        let spaces: Vec<Hash> = self
            .spaces
            .values()
            .filter(|s| s.identities().contains(&did) && !s.is_node_of(node, did))
            .map(Space::id)
            .collect();
        for space in spaces {
            if !self.tried.insert((space, node, Hash::of(&cert.0))) {
                continue;
            }
            let act = Act::Vouch {
                ops: ops.to_vec(),
                cert: cert.clone(),
            };
            if let Ok((_, more)) = self.write(space, time, vec![act]) {
                step.out.extend(more.out);
                step.events.extend(more.events);
            }
        }
    }

    fn last_time(&self) -> u64 {
        self.time
    }

    /// Writes the vouches for connected peers that a space now names, for
    /// example after a grant to their identity.
    fn vouch_peers(&mut self, step: &mut Step) {
        let proofs: Vec<(PublicKey, Proof)> =
            self.proofs.iter().map(|(k, p)| (*k, p.clone())).collect();
        let time = self.last_time();
        for (peer, (did, ops, cert)) in proofs {
            let before = step.out.len();
            self.vouch(did, peer, &ops, &cert, time, step);
            if step.out.len() > before {
                step.out.push((peer, self.have_for(peer)));
            }
        }
    }

    /// The nodes this node is connected to.
    pub fn peers(&self) -> impl Iterator<Item = &PublicKey> {
        self.peers.iter()
    }

    /// The spaces this node follows and does not have yet.
    pub fn wanted(&self) -> impl Iterator<Item = &Hash> {
        self.wanted.iter()
    }

    /// Starts to get a space from peers.
    pub fn follow(&mut self, id: Hash) -> Step {
        let mut step = Step::default();
        // Asked again when it was asked before: a peer that did not send it
        // then may send it now, for example after a grant or a vouch.
        if !self.spaces.contains_key(&id) {
            self.wanted.insert(id);
            for peer in &self.peers {
                step.out.push((*peer, Msg::Want(id)));
            }
        }
        step
    }

    /// Writes acts, signed with this node's key. Gives back the new entry.
    ///
    /// # Errors
    ///
    /// When the space is not here, or the key may not do these acts.
    pub fn write(
        &mut self,
        space: Hash,
        time: u64,
        acts: Vec<Act>,
    ) -> Result<(Entry, Step), Error> {
        let key = self.key.clone();
        self.write_as(space, &key, time, acts)
    }

    /// Writes acts signed by `author`: this node or one of its apps. Gives
    /// back the new entry.
    ///
    /// # Errors
    ///
    /// When the space is not here, or `author` may not do these acts.
    pub fn write_as(
        &mut self,
        space: Hash,
        author: &SecretKey,
        time: u64,
        acts: Vec<Act>,
    ) -> Result<(Entry, Step), Error> {
        let (entry, mut step) = self.write_entry(space, author, time, acts)?;
        self.upkeep(space, &mut step);
        Ok((entry, step))
    }

    fn write_inner(
        &mut self,
        space: Hash,
        author: &SecretKey,
        time: u64,
        acts: Vec<Act>,
    ) -> Result<Step, Error> {
        self.write_entry(space, author, time, acts)
            .map(|(_, step)| step)
    }

    fn write_entry(
        &mut self,
        space: Hash,
        author: &SecretKey,
        time: u64,
        acts: Vec<Act>,
    ) -> Result<(Entry, Step), Error> {
        let s = self
            .spaces
            .get_mut(&space)
            .ok_or_else(|| Error::invalid(format!("space {space} is not here")))?;
        self.time = self.time.max(time);
        let mut new = Vec::new();
        // This node shows first that it is a node of its identity, when the
        // space names that identity.
        if author.public() == self.key.public()
            && let Some((did, ops, cert)) = &self.identity
            && s.identities().contains(did)
            && !s.is_node_of(author.public(), *did)
            && self
                .tried
                .insert((space, author.public(), Hash::of(&cert.0)))
        {
            new.push(s.write(
                author,
                time,
                vec![Act::Vouch {
                    ops: ops.clone(),
                    cert: cert.clone(),
                }],
            )?);
        }
        let entry = s.write(author, time, acts)?;
        new.push(entry.clone());
        let mut step = Step::default();
        step.events.push(Event::Changed(space));
        self.send_on(space, &new, None, &mut step);
        if !self.proofs.is_empty() && new.iter().any(|e| e.header.author == self.key.public()) {
            self.vouch_peers(&mut step);
        }
        Ok((entry, step))
    }

    /// Makes a checkpoint of a space, signed with this node's key. When to
    /// make one is the caller's policy (for example once a day, on a node of
    /// the owner).
    ///
    /// # Errors
    ///
    /// When the space is not here, or the key may not make checkpoints.
    pub fn checkpoint(
        &mut self,
        space: Hash,
        time: u64,
        retention: Retention,
    ) -> Result<Step, Error> {
        // Keys first: a new epoch that is due goes in before the checkpoint,
        // so the checkpoint can drop the keys of the old one.
        let mut step = Step::default();
        self.upkeep_at(space, time, &mut step);
        let s = self
            .spaces
            .get_mut(&space)
            .ok_or_else(|| Error::invalid(format!("space {space} is not here")))?;
        let entry = s.checkpoint(&self.key, time, retention)?;
        step.events.push(Event::Changed(space));
        self.send_on(space, &[entry], None, &mut step);
        self.again(space, Vec::new(), &mut step);
        Ok(step)
    }

    /// Drops the entries of a space before its newest checkpoint that every
    /// entry here is ordered with. When to do it is the caller's policy.
    /// Gives back false when there is nothing to drop.
    pub fn compact(&mut self, space: Hash) -> (bool, Step) {
        let mut step = Step::default();
        let Some(s) = self.spaces.get_mut(&space) else {
            return (false, step);
        };
        let stale = s.take_stale(self.key.public());
        let done = s.compact();
        self.again(space, stale, &mut step);
        (done, step)
    }

    /// Writes again, on top, this node's acts that were made at the same
    /// time as a checkpoint.
    fn again(&mut self, space: Hash, mut stale: Vec<(u64, Vec<Act>)>, step: &mut Step) {
        let me = self.key.public();
        if let Some(s) = self.spaces.get_mut(&space) {
            stale.extend(s.take_stale(me));
        }
        let mut written = 0;
        let key = self.key.clone();
        for (time, acts) in stale {
            if let Ok(more) = self.write_inner(space, &key, time, acts) {
                written += 1;
                step.out.extend(more.out);
            }
        }
        if written > 0 {
            step.events.push(Event::Again {
                space,
                entries: written,
            });
        }
        self.upkeep(space, step);
    }

    /// Sends new entries to every connected peer that may keep the space.
    fn send_on(&self, space: Hash, entries: &[Entry], except: Option<PublicKey>, step: &mut Step) {
        let Some(s) = self.spaces.get(&space) else {
            return;
        };
        for peer in &self.peers {
            if Some(*peer) != except && Self::may_keep(s, *peer) {
                step.out.push((
                    *peer,
                    Msg::Entries {
                        space,
                        entries: entries.to_vec(),
                    },
                ));
            }
        }
    }

    fn have_for(&self, peer: PublicKey) -> Msg {
        Msg::Have(
            self.spaces
                .values()
                .filter(|s| Self::may_keep(s, peer))
                .map(|s| (s.id(), s.logs()))
                .collect(),
        )
    }

    pub fn connected(&mut self, peer: PublicKey) -> Step {
        let mut step = Step::default();
        self.peers.insert(peer);
        self.announce_identity(peer, &mut step);
        step.out.push((peer, self.have_for(peer)));
        for id in &self.wanted {
            step.out.push((peer, Msg::Want(*id)));
        }
        step
    }

    fn announce_identity(&self, peer: PublicKey, step: &mut Step) {
        if let Some((_, ops, cert)) = &self.identity {
            step.out.push((
                peer,
                Msg::Iam {
                    ops: ops.clone(),
                    cert: cert.clone(),
                },
            ));
        }
        if !self.revocations.is_empty() {
            let all = self.revocations.iter().map(|(_, _, b)| b.clone()).collect();
            step.out.push((peer, Msg::Revoked(all)));
        }
    }

    pub fn disconnected(&mut self, peer: PublicKey) {
        self.peers.remove(&peer);
        self.proofs.remove(&peer);
        self.sources.remove(&peer);
        self.asked.retain(|_, request| request.from != peer);
    }

    /// Releases a request after the transport reports a failure.
    pub fn content_failed(&mut self, to: PublicKey, hash: Hash) {
        if self
            .asked
            .get(&hash)
            .is_some_and(|request| request.from == to)
        {
            self.asked.remove(&hash);
        }
    }

    /// Gives a new set of offers and requests for peers after a queue fills.
    pub fn announce(&self) -> Step {
        let mut step = Step::default();
        for &peer in &self.peers {
            self.announce_identity(peer, &mut step);
            step.out.push((peer, self.have_for(peer)));
            step.out
                .extend(self.wanted.iter().map(|&id| (peer, Msg::Want(id))));
        }
        step
    }

    /// Advances the retry clock and asks for content that is still missing.
    pub fn retry_contents(&mut self, contents: &dyn Contents) -> Step {
        self.ticks = self.ticks.wrapping_add(1);
        self.asked.retain(|_, request| {
            request.deadline.wrapping_sub(self.ticks) <= REQUEST_TICKS
                && request.deadline != self.ticks
        });
        let mut step = Step::default();
        let mut sources: Vec<_> = self.sources.iter().collect();
        if sources.is_empty() {
            return step;
        }
        let start = (self.ticks / REQUEST_TICKS) as usize % sources.len();
        sources.rotate_left(start);
        for (&peer, spaces) in sources {
            for &id in spaces {
                if self.asked.len() >= CONTENT_IN_FLIGHT {
                    return step;
                }
                if let Some(space) = self.spaces.get(&id) {
                    ask_contents(
                        &mut self.asked,
                        self.ticks,
                        peer,
                        space,
                        contents,
                        &mut step,
                    );
                }
            }
        }
        step
    }

    /// Takes a message from a peer.
    pub fn receive(&mut self, from: PublicKey, msg: Msg, contents: &dyn Contents) -> Step {
        let mut step = Step::default();
        match msg {
            Msg::Have(spaces) => {
                let mut ask = Vec::new();
                for (id, logs) in spaces {
                    let Some(s) = self.spaces.get(&id) else {
                        continue;
                    };
                    if Self::may_keep(s, from) {
                        if s.needs_base(&logs)
                            && let Some(snapshot) = s.base()
                        {
                            step.out.push((
                                from,
                                Msg::Snapshot {
                                    space: id,
                                    snapshot: Box::new(snapshot.clone()),
                                },
                            ));
                        }
                        let entries = s.missing(&logs);
                        if !entries.is_empty() {
                            step.out.push((from, Msg::Entries { space: id, entries }));
                        }
                    }
                    // The peer has entries we do not have: say what we have,
                    // so that it sends them. We take entries from anyone;
                    // they prove themselves.
                    let ours = s.logs();
                    if logs
                        .iter()
                        .any(|(k, n)| ours.get(k).copied().unwrap_or(0) < *n)
                    {
                        ask.push((id, ours));
                    }
                    self.ask_contents(from, id, contents, &mut step);
                }
                if !ask.is_empty() {
                    step.out.push((from, Msg::Have(ask)));
                }
            }
            Msg::Want(id) => {
                if let Some(s) = self.spaces.get(&id)
                    && Self::may_keep(s, from)
                {
                    if let Some(snapshot) = s.base() {
                        step.out.push((
                            from,
                            Msg::Snapshot {
                                space: id,
                                snapshot: Box::new(snapshot.clone()),
                            },
                        ));
                    }
                    step.out.push((
                        from,
                        Msg::Entries {
                            space: id,
                            entries: s.entries(),
                        },
                    ));
                }
            }
            Msg::Entries { space, entries } => {
                self.take_entries(from, space, entries, contents, &mut step);
            }
            Msg::Iam { ops, cert } => {
                if let Ok(did) = Self::check_proof(from, &ops, &cert) {
                    let time = self.last_time();
                    self.vouch(did, from, &ops, &cert, time, &mut step);
                    self.proofs.insert(from, (did, ops, cert));
                    step.out.push((from, self.have_for(from)));
                }
            }
            Msg::Revoked(revocations) => {
                let more = self.add_revocations(revocations);
                step.out.extend(more.out);
                step.events.extend(more.events);
            }
            Msg::Snapshot { space, snapshot } => {
                self.take_snapshot(from, space, *snapshot, contents, &mut step);
            }
            Msg::WantContent(hash) => {
                let allowed = self.spaces.values().any(|s| {
                    Self::may_keep(s, from)
                        && s.contents().iter().any(|c| {
                            c.hash == hash
                                || content::chunks(c, contents).is_some_and(|l| l.contains(&hash))
                        })
                });
                if allowed && let Some(bytes) = contents.get(hash) {
                    step.out.push((from, Msg::Content(Bytes(bytes))));
                }
            }
            Msg::Content(bytes) => {
                let hash = Hash::of(&bytes.0);
                if let Some(request) = self.asked.remove(&hash) {
                    if let Some(list) = request.list {
                        let Some(parts) = content::chunk_list(&list, &bytes.0) else {
                            return step;
                        };
                        for (chunk, _) in parts {
                            if !contents.has(chunk) {
                                request_content(
                                    &mut self.asked,
                                    self.ticks,
                                    request.from,
                                    chunk,
                                    None,
                                    &mut step,
                                );
                            }
                        }
                    }
                    step.events.push(Event::Content {
                        from: request.from,
                        data: bytes.0,
                    });
                }
            }
        }
        step
    }

    fn take_snapshot(
        &mut self,
        from: PublicKey,
        space: Hash,
        snapshot: Snapshot,
        contents: &dyn Contents,
        step: &mut Step,
    ) {
        if snapshot.genesis.hash() != space {
            return;
        }
        let stale = if let Some(s) = self.spaces.get_mut(&space) {
            match s.adopt(snapshot, self.key.public()) {
                Ok(stale) => stale,
                Err(_) => return,
            }
        } else {
            if !self.wanted.contains(&space) {
                return;
            }
            let Ok(s) = Space::from_snapshot(snapshot) else {
                return;
            };
            self.wanted.remove(&space);
            self.spaces.insert(space, s);
            Vec::new()
        };
        step.events.push(Event::Changed(space));
        self.ask_contents(from, space, contents, step);
        self.again(space, stale, step);
    }

    /// Remembers a source and asks it for missing content.
    fn ask_contents(
        &mut self,
        from: PublicKey,
        space: Hash,
        contents: &dyn Contents,
        step: &mut Step,
    ) {
        if let Some(s) = self.spaces.get(&space) {
            if self.peers.contains(&from) {
                self.sources.entry(from).or_default().insert(space);
            }
            ask_contents(&mut self.asked, self.ticks, from, s, contents, step);
        }
    }

    /// Fills slots freed by replies after the caller has stored their bytes.
    pub fn content_saved(&mut self, peer: PublicKey, contents: &dyn Contents) -> Step {
        let mut step = Step::default();
        self.fill_from(peer, contents, &mut step);
        step
    }

    fn fill_from(&mut self, from: PublicKey, contents: &dyn Contents, step: &mut Step) {
        if let Some(spaces) = self.sources.get(&from) {
            for id in spaces {
                if self.asked.len() >= CONTENT_IN_FLIGHT {
                    break;
                }
                if let Some(space) = self.spaces.get(id) {
                    ask_contents(&mut self.asked, self.ticks, from, space, contents, step);
                }
            }
        }
    }

    fn take_entries(
        &mut self,
        from: PublicKey,
        space: Hash,
        mut entries: Vec<Entry>,
        contents: &dyn Contents,
        step: &mut Step,
    ) {
        if !self.spaces.contains_key(&space) {
            if !self.wanted.contains(&space) {
                return;
            }
            let Some(pos) = entries.iter().position(|e| e.hash() == space) else {
                // Only new entries came, maybe because we were just given
                // access: ask for the whole space.
                step.out.push((from, Msg::Want(space)));
                return;
            };
            let genesis = entries.remove(pos);
            let Ok(s) = Space::from_genesis(&genesis) else {
                return;
            };
            self.wanted.remove(&space);
            self.spaces.insert(space, s);
            step.events.push(Event::Changed(space));
            // Ask for anything that came before we followed.
            step.out.push((from, self.have_for(from)));
        }
        let s = self.spaces.get_mut(&space).expect("the space is here");
        let (added, _) = s.insert_batch(entries);
        if added.is_empty() {
            return;
        }
        step.events.push(Event::Changed(space));
        let new: Vec<Entry> = added
            .into_iter()
            .filter_map(|hash| s.entry(hash).cloned())
            .collect();
        if let Some(t) = new.iter().map(|e| e.header.time).max() {
            self.time = self.time.max(t);
        }
        self.ask_contents(from, space, contents, step);
        self.send_on(space, &new, Some(from), step);
        self.vouch_peers(step);
        if !self.revocations.is_empty() {
            self.unvouch_all(step);
        }
        self.again(space, Vec::new(), step);
    }
}

fn ask_contents(
    asked: &mut BTreeMap<Hash, Request>,
    ticks: u64,
    from: PublicKey,
    space: &Space,
    contents: &dyn Contents,
    step: &mut Step,
) {
    for content in space.contents() {
        if asked.len() >= CONTENT_IN_FLIGHT {
            break;
        }
        if let Some(hashes) = content::chunks(&content, contents) {
            for hash in hashes.into_iter().filter(|h| !contents.has(*h)) {
                request_content(asked, ticks, from, hash, None, step);
                if asked.len() >= CONTENT_IN_FLIGHT {
                    break;
                }
            }
        } else if content.chunks && !contents.has(content.hash) {
            request_content(asked, ticks, from, content.hash, Some(content), step);
        }
    }
}

fn request_content(
    asked: &mut BTreeMap<Hash, Request>,
    ticks: u64,
    from: PublicKey,
    hash: Hash,
    list: Option<Content>,
    step: &mut Step,
) {
    if asked.len() < CONTENT_IN_FLIGHT
        && let std::collections::btree_map::Entry::Vacant(entry) = asked.entry(hash)
    {
        entry.insert(Request {
            from,
            deadline: ticks.wrapping_add(REQUEST_TICKS),
            list,
        });
        step.out.push((from, Msg::WantContent(hash)));
    }
}
