// SPDX-License-Identifier: AGPL-3.0-only

//! Sync between nodes, with no I/O.
//!
//! A [`Replica`] is the set of folders on one node. It takes messages from
//! peers and gives back messages to send and events for the node. It never
//! touches the network or the disk itself. `fos-net` carries the messages over
//! libp2p, and `fos-sim` carries them in memory.
//!
//! The rules:
//!
//! - When two nodes connect, each says which folders it has (`Have`), but
//!   only folders that the other node may keep. A node that is behind asks
//!   for the entries (`Want`).
//! - A node that is not the ordering node of a folder sends its writes to the
//!   ordering node (`Submit`). When the ordering node is not connected, the
//!   writes wait.
//! - The ordering node sends new entries to every connected node that may
//!   keep the folder.
//! - A node gets blobs from the node that sent it the entry.
//! - Every node asks its own engine about every entry.

use std::collections::{BTreeMap, BTreeSet};

use fos_core::{Bytes, Error, Hash, PublicKey, SecretKey};
use fos_fs::{Authz, Entry, Folder, Fork, Op, Reject, SignedWrite};
use serde::{Deserialize, Serialize};

/// A message between two nodes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Msg {
    /// The folders I have, each with the `seq` of my next entry.
    Have(Vec<(Hash, u64)>),
    /// Send me the entries of a folder, from `from` on.
    Want {
        folder: Hash,
        from: u64,
    },
    Entries {
        folder: Hash,
        entries: Vec<Entry>,
    },
    /// A write for the ordering node.
    Submit(SignedWrite),
    /// The ordering node did not take a write.
    Refused {
        write: Hash,
        reason: String,
    },
    WantBlob(Hash),
    Blob(Bytes),
}

/// A message to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Out {
    pub to: PublicKey,
    pub msg: Msg,
}

/// Something the node may want to act on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// A folder is now on this node.
    Added(Hash),
    /// New entries were taken. The node keeps them, and tells the apps.
    Entries { folder: Hash, entries: Vec<Entry> },
    /// This node's engine did not take an entry, so the copy forked.
    Forked { folder: Hash, fork: Fork },
    /// The ordering node did not take one of our writes.
    Refused { write: Hash, reason: String },
    /// A blob came. The node keeps it.
    Blob(Vec<u8>),
}

/// The output of one step.
#[derive(Debug, Default)]
pub struct Step {
    pub out: Vec<Out>,
    pub events: Vec<Event>,
}

impl Step {
    fn send(&mut self, to: PublicKey, msg: Msg) {
        self.out.push(Out { to, msg });
    }
}

/// What the replica needs to know about the blobs on this node.
pub trait Blobs {
    fn has(&self, hash: Hash) -> bool;
    fn get(&self, hash: Hash) -> Option<Vec<u8>>;
}

/// The folders on one node, and what it knows about its peers.
#[derive(Debug)]
pub struct Replica {
    key: SecretKey,
    /// Other keys that this node puts folders in order for, such as the key
    /// of an identity that this node holds.
    keys: Vec<SecretKey>,
    folders: BTreeMap<Hash, Folder>,
    /// Folders to get, whose genesis entry this node does not have yet.
    wanted: BTreeSet<Hash>,
    peers: BTreeSet<PublicKey>,
    /// Writes waiting for their ordering node.
    pending: Vec<SignedWrite>,
    nonce: u64,
}

impl Replica {
    /// `nonce` should be random, so that writes after a restart are different.
    pub fn new(key: SecretKey, nonce: u64) -> Self {
        Self {
            key,
            keys: Vec::new(),
            folders: BTreeMap::new(),
            wanted: BTreeSet::new(),
            peers: BTreeSet::new(),
            pending: Vec::new(),
            nonce,
        }
    }

    pub fn me(&self) -> PublicKey {
        self.key.public()
    }

    pub fn folder(&self, id: Hash) -> Option<&Folder> {
        self.folders.get(&id)
    }

    pub fn folders(&self) -> impl Iterator<Item = &Folder> {
        self.folders.values()
    }

    pub fn peers(&self) -> impl Iterator<Item = &PublicKey> {
        self.peers.iter()
    }

    /// Writes waiting for their ordering node.
    pub fn pending(&self) -> &[SignedWrite] {
        &self.pending
    }

    /// Puts a folder that was kept on disk back in place.
    pub fn insert(&mut self, folder: Folder) {
        self.wanted.remove(&folder.id());
        self.folders.insert(folder.id(), folder);
    }

    /// Lets this node put in order the folders whose ordering node is `key`.
    pub fn add_key(&mut self, key: SecretKey) {
        if key.public() != self.me() && !self.keys.iter().any(|k| k.public() == key.public()) {
            self.keys.push(key);
        }
    }

    /// Our secret key for `orderer`, if this node puts its folders in order.
    fn ordering_key(&self, orderer: PublicKey) -> Option<&SecretKey> {
        std::iter::once(&self.key)
            .chain(&self.keys)
            .find(|k| k.public() == orderer)
    }

    /// Makes a new folder. This node is its creator and ordering node.
    pub fn create(&mut self, name: &str, authz: &mut dyn Authz) -> (Hash, Step) {
        let key = self.key.clone();
        self.create_as(name, &key, authz)
    }

    /// Makes a new folder with `key` as its creator and ordering node. This
    /// node keeps `key`, so it puts the folder in order.
    pub fn create_as(
        &mut self,
        name: &str,
        key: &SecretKey,
        authz: &mut dyn Authz,
    ) -> (Hash, Step) {
        self.add_key(key.clone());
        let (folder, genesis) = Folder::create(name, key, authz);
        let id = folder.id();
        self.folders.insert(id, folder);
        let mut step = Step::default();
        step.events.push(Event::Added(id));
        step.events.push(Event::Entries {
            folder: id,
            entries: vec![genesis],
        });
        (id, step)
    }

    /// Starts to get a folder from peers. Their engines say if they send it.
    pub fn follow(&mut self, id: Hash) -> Step {
        let mut step = Step::default();
        if self.folders.contains_key(&id) {
            return step;
        }
        self.wanted.insert(id);
        for peer in &self.peers {
            step.send(
                *peer,
                Msg::Want {
                    folder: id,
                    from: 0,
                },
            );
        }
        step
    }

    /// Makes a write signed by `author`: the node itself or one of its apps.
    ///
    /// # Errors
    ///
    /// When the folder is not here, or the engine of this node says no.
    pub fn write(
        &mut self,
        folder: Hash,
        op: Op,
        author: &SecretKey,
        authz: &mut dyn Authz,
    ) -> Result<Step, Error> {
        self.nonce += 1;
        let write = SignedWrite::sign(Some(folder), self.nonce, op, author);
        let orderer = self
            .folders
            .get(&folder)
            .ok_or_else(|| Error(format!("folder {folder} is not here")))?
            .orderer();
        let ordering_key = self.ordering_key(orderer).cloned();
        let f = self
            .folders
            .get_mut(&folder)
            .ok_or_else(|| Error(format!("folder {folder} is not here")))?;
        // Our own engine checks the write first, so we do not send writes it
        // would not take back.
        f.try_write(&write, authz).map_err(Error)?;
        let mut step = Step::default();
        if let Some(key) = ordering_key {
            let entry = f.order(write, &key, authz)?;
            self.publish(folder, vec![entry], &mut step, authz);
        } else {
            let orderer = f.orderer();
            if self.peers.contains(&orderer) {
                step.send(orderer, Msg::Submit(write.clone()));
            }
            self.pending.push(write);
        }
        Ok(step)
    }

    /// A peer connected.
    pub fn connected(&mut self, peer: PublicKey, authz: &mut dyn Authz) -> Step {
        self.peers.insert(peer);
        let mut step = Step::default();
        let have = self.folders_for(peer, authz);
        step.send(peer, Msg::Have(have));
        for id in &self.wanted {
            step.send(
                peer,
                Msg::Want {
                    folder: *id,
                    from: 0,
                },
            );
        }
        for write in &self.pending {
            let orderer = write
                .write
                .folder
                .and_then(|id| self.folders.get(&id))
                .map(Folder::orderer);
            if orderer == Some(peer) {
                step.send(peer, Msg::Submit(write.clone()));
            }
        }
        step
    }

    pub fn disconnected(&mut self, peer: PublicKey) {
        self.peers.remove(&peer);
    }

    /// Takes one message from a peer.
    pub fn handle(
        &mut self,
        from: PublicKey,
        msg: Msg,
        authz: &mut dyn Authz,
        blobs: &dyn Blobs,
    ) -> Step {
        let mut step = Step::default();
        match msg {
            Msg::Have(list) => {
                for (id, next) in list {
                    // A copy that forked takes no more entries, so it asks for none.
                    let ours = match self.folders.get(&id) {
                        Some(f) if f.fork().is_none() => f.next_seq(),
                        None if self.wanted.contains(&id) => 0,
                        _ => continue,
                    };
                    if next > ours {
                        step.send(
                            from,
                            Msg::Want {
                                folder: id,
                                from: ours,
                            },
                        );
                    }
                }
            }
            Msg::Want { folder, from: seq } => {
                if let Some(f) = self.folders.get(&folder)
                    && f.storers(authz).contains(&from)
                {
                    let entries = f.entries_from(seq).to_vec();
                    if !entries.is_empty() {
                        step.send(from, Msg::Entries { folder, entries });
                    }
                }
            }
            Msg::Entries { folder, entries } => {
                self.take(from, folder, entries, &mut step, authz, blobs);
            }
            Msg::Submit(write) => self.submit(from, write, &mut step, authz, blobs),
            Msg::Refused { write, reason } => {
                if self.pending.iter().any(|w| w.hash() == write) {
                    self.pending.retain(|w| w.hash() != write);
                    step.events.push(Event::Refused { write, reason });
                }
            }
            Msg::WantBlob(hash) => {
                if self.may_get_blob(from, hash, authz)
                    && let Some(data) = blobs.get(hash)
                {
                    step.send(from, Msg::Blob(Bytes(data)));
                }
            }
            Msg::Blob(data) => {
                // Only blobs that a folder here names are kept.
                let hash = Hash::of(&data.0);
                if self.folders.values().any(|f| f.blobs().contains(&hash)) {
                    step.events.push(Event::Blob(data.0));
                }
            }
        }
        step
    }

    /// The folders that `peer` may keep, with our next `seq` for each.
    fn folders_for(&self, peer: PublicKey, authz: &mut dyn Authz) -> Vec<(Hash, u64)> {
        self.folders
            .values()
            .filter(|f| f.storers(authz).contains(&peer))
            .map(|f| (f.id(), f.next_seq()))
            .collect()
    }

    fn may_get_blob(&self, peer: PublicKey, hash: Hash, authz: &mut dyn Authz) -> bool {
        // The ordering node may get the blob of a write we sent it.
        let sent = self.pending.iter().any(|w| {
            matches!(w.write.op, Op::Put { blob, .. } if blob == hash)
                && w.write
                    .folder
                    .and_then(|id| self.folders.get(&id))
                    .map(Folder::orderer)
                    == Some(peer)
        });
        sent || self
            .folders
            .values()
            .any(|f| f.blobs().contains(&hash) && f.storers(authz).contains(&peer))
    }

    fn take(
        &mut self,
        from: PublicKey,
        id: Hash,
        entries: Vec<Entry>,
        step: &mut Step,
        authz: &mut dyn Authz,
        blobs: &dyn Blobs,
    ) {
        let mut entries = entries.into_iter().peekable();
        if !self.folders.contains_key(&id) {
            if !self.wanted.contains(&id) {
                return;
            }
            let Some(genesis) = entries.next_if(|e| e.seq == 0) else {
                return;
            };
            match Folder::from_genesis(genesis.clone(), authz) {
                Ok(folder) if folder.id() == id => {
                    self.wanted.remove(&id);
                    self.folders.insert(id, folder);
                    step.events.push(Event::Added(id));
                    step.events.push(Event::Entries {
                        folder: id,
                        entries: vec![genesis],
                    });
                }
                _ => return,
            }
        }
        let folder = self.folders.get_mut(&id).expect("the folder is here");
        let mut taken = Vec::new();
        for entry in entries {
            match folder.accept(entry.clone(), authz) {
                Ok(()) => taken.push(entry),
                Err(Reject::Old) => {}
                Err(Reject::Gap { next }) => {
                    step.send(
                        from,
                        Msg::Want {
                            folder: id,
                            from: next,
                        },
                    );
                    break;
                }
                Err(Reject::Fork(fork)) => {
                    step.events.push(Event::Forked { folder: id, fork });
                    break;
                }
                Err(Reject::Invalid(_) | Reject::Forked) => break,
            }
        }
        if taken.is_empty() {
            return;
        }
        for entry in &taken {
            if let Op::Put { blob, .. } = &entry.write.write.op
                && !blobs.has(*blob)
            {
                step.send(from, Msg::WantBlob(*blob));
            }
        }
        let done: BTreeSet<Hash> = taken.iter().map(|e| e.write.hash()).collect();
        self.pending.retain(|w| !done.contains(&w.hash()));
        step.events.push(Event::Entries {
            folder: id,
            entries: taken,
        });
    }

    fn submit(
        &mut self,
        from: PublicKey,
        write: SignedWrite,
        step: &mut Step,
        authz: &mut dyn Authz,
        blobs: &dyn Blobs,
    ) {
        let Some(id) = write.write.folder else { return };
        let Some(orderer) = self.folders.get(&id).map(Folder::orderer) else {
            return;
        };
        let key = self.ordering_key(orderer).unwrap_or(&self.key).clone();
        let Some(folder) = self.folders.get_mut(&id) else {
            return;
        };
        let hash = write.hash();
        // A write that is already in the log came again: the sender did not
        // see the entry. Send it the entries.
        if let Some(seq) = folder
            .entries_from(0)
            .iter()
            .position(|e| e.write.hash() == hash)
        {
            let entries = folder.entries_from(seq as u64).to_vec();
            step.send(
                from,
                Msg::Entries {
                    folder: id,
                    entries,
                },
            );
            return;
        }
        match folder.order(write, &key, authz) {
            Ok(entry) => {
                if let Op::Put { blob, .. } = &entry.write.write.op
                    && !blobs.has(*blob)
                {
                    step.send(from, Msg::WantBlob(*blob));
                }
                self.publish(id, vec![entry], step, authz);
            }
            Err(e) => step.send(
                from,
                Msg::Refused {
                    write: hash,
                    reason: e.0,
                },
            ),
        }
    }

    /// Sends new entries to every connected node that may keep the folder.
    fn publish(&self, id: Hash, entries: Vec<Entry>, step: &mut Step, authz: &mut dyn Authz) {
        let folder = &self.folders[&id];
        for peer in folder.storers(authz) {
            if self.peers.contains(&peer) {
                step.send(
                    peer,
                    Msg::Entries {
                        folder: id,
                        entries: entries.clone(),
                    },
                );
            }
        }
        step.events.push(Event::Entries {
            folder: id,
            entries,
        });
    }
}
