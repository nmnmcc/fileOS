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

use std::collections::{BTreeMap, BTreeSet};

use fos_core::{Bytes, Error, Hash, PublicKey, SecretKey};
use serde::{Deserialize, Serialize};

use crate::content;
use crate::entry::{Act, Entry, Permission, Principal, Resource, Retention};
use crate::space::{Snapshot, Space};

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
}

/// Something the caller acts on.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Event {
    /// The state of a space changed.
    Changed(Hash),
    /// Content came. The caller keeps it.
    Content(Vec<u8>),
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

/// The spaces on one node, and its peers.
#[derive(Debug)]
pub struct Node {
    key: SecretKey,
    spaces: BTreeMap<Hash, Space>,
    /// Spaces to get, whose first entry this node does not have yet.
    wanted: BTreeSet<Hash>,
    peers: BTreeSet<PublicKey>,
    /// Pieces of content asked for and not yet here.
    asked: BTreeSet<Hash>,
    /// The ones of them that are lists of chunks.
    lists: BTreeSet<Hash>,
    nonce: u64,
    /// The identity this node is a node of: its DID operations and the
    /// certificate of this node.
    identity: Option<Proof>,
    /// What connected peers showed about their identities.
    proofs: BTreeMap<PublicKey, Proof>,
    /// The newest time this node has seen, for what it writes when a
    /// message comes (this crate has no clock).
    time: u64,
}

impl Node {
    /// `nonce` should be random, so that spaces made after a restart are
    /// different.
    pub fn new(key: SecretKey, nonce: u64) -> Self {
        Self {
            key,
            spaces: BTreeMap::new(),
            wanted: BTreeSet::new(),
            peers: BTreeSet::new(),
            asked: BTreeSet::new(),
            lists: BTreeSet::new(),
            nonce,
            identity: None,
            proofs: BTreeMap::new(),
            time: 0,
        }
    }

    pub fn me(&self) -> PublicKey {
        self.key.public()
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
        (id, step)
    }

    /// Makes a space owned by `owner`, which also signs its first entry: for
    /// example the signing key of an identity that this node holds.
    pub fn create_as(&mut self, owner: &SecretKey, time: u64) -> (Hash, Step) {
        self.nonce += 1;
        let (space, _) = Space::create(Principal::Key(owner.public()), owner, time, self.nonce);
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
        let cert = fos_id::NodeCert::from_bytes(&cert.0)?;
        let did = fos_id::did_hash(&cert.did)?;
        let ops: Vec<fos_id::Op> = ops
            .iter()
            .map(|o| fos_id::Op::from_bytes(&o.0))
            .collect::<Result<_, _>>()?;
        let signing = fos_id::resolve(&cert.did, &ops)?
            .state
            .and_then(|s| s.signing_key())
            .ok_or_else(|| Error("the identity has no signing key".into()))?;
        if cert.node != node || !cert.verify(signing) {
            return Err(Error("the certificate is not good for this node".into()));
        }
        Ok(did)
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
        let spaces: Vec<Hash> = self
            .spaces
            .values()
            .filter(|s| s.identities().contains(&did) && !s.is_node_of(node, did))
            .map(Space::id)
            .collect();
        for space in spaces {
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
        if !self.spaces.contains_key(&id) && self.wanted.insert(id) {
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
        let s = self
            .spaces
            .get_mut(&space)
            .ok_or_else(|| Error(format!("space {space} is not here")))?;
        self.time = self.time.max(time);
        let mut new = Vec::new();
        // This node shows first that it is a node of its identity, when the
        // space names that identity.
        if author.public() == self.key.public()
            && let Some((did, ops, cert)) = &self.identity
            && s.identities().contains(did)
            && !s.is_node_of(author.public(), *did)
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
        let s = self
            .spaces
            .get_mut(&space)
            .ok_or_else(|| Error(format!("space {space} is not here")))?;
        let entry = s.checkpoint(&self.key, time, retention)?;
        let mut step = Step::default();
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
        for (time, acts) in stale {
            if let Ok((_, more)) = self.write(space, time, acts) {
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
        if let Some((_, ops, cert)) = &self.identity {
            step.out.push((
                peer,
                Msg::Iam {
                    ops: ops.clone(),
                    cert: cert.clone(),
                },
            ));
        }
        step.out.push((peer, self.have_for(peer)));
        for id in &self.wanted {
            step.out.push((peer, Msg::Want(*id)));
        }
        step
    }

    pub fn disconnected(&mut self, peer: PublicKey) {
        self.peers.remove(&peer);
        self.proofs.remove(&peer);
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
                if self.asked.remove(&hash) {
                    if self.lists.remove(&hash)
                        && let Ok(list) = fos_core::decode::<Vec<(Hash, u64)>>(&bytes.0)
                    {
                        for (chunk, _) in list {
                            if !contents.has(chunk) && self.asked.insert(chunk) {
                                step.out.push((from, Msg::WantContent(chunk)));
                            }
                        }
                    }
                    step.events.push(Event::Content(bytes.0));
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

    /// Asks `from` for the content of `space` that is not here.
    fn ask_contents(
        &mut self,
        from: PublicKey,
        space: Hash,
        contents: &dyn Contents,
        step: &mut Step,
    ) {
        let Some(s) = self.spaces.get(&space) else {
            return;
        };
        let mut wanted = Vec::new();
        for c in s.contents() {
            if let Some(hashes) = content::chunks(&c, contents) {
                wanted.extend(hashes.into_iter().filter(|h| !contents.has(*h)));
            } else {
                self.lists.insert(c.hash);
                wanted.push(c.hash);
            }
        }
        for hash in wanted {
            if self.asked.insert(hash) {
                step.out.push((from, Msg::WantContent(hash)));
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
        let added = s.insert(entries.clone()).unwrap_or_default();
        if added.is_empty() {
            return;
        }
        step.events.push(Event::Changed(space));
        let new: Vec<Entry> = entries
            .into_iter()
            .filter(|e| added.contains(&e.hash()))
            .collect();
        if let Some(t) = new.iter().map(|e| e.header.time).max() {
            self.time = self.time.max(t);
        }
        self.ask_contents(from, space, contents, step);
        self.send_on(space, &new, Some(from), step);
        self.vouch_peers(step);
        self.again(space, Vec::new(), step);
    }
}
