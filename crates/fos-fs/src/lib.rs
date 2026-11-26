// SPDX-License-Identifier: AGPL-3.0-only

//! Folders: the one base of fileOS.
//!
//! A folder is a signed log and the blobs it names. Every write is a
//! [`SignedWrite`] by its author. The ordering node of the folder puts writes
//! in order as [`Entry`] values; this order is the main line. A node takes
//! each entry only when its own [`Authz`] engine says yes. When its engine
//! says no, the copy on that node stops at that entry: it has a [`Fork`].
//!
//! This crate is sans-IO and portable: no threads, files, sockets or clocks.

use std::collections::BTreeMap;

use fos_core::{Bytes, Error, Hash, PublicKey, SecretKey, Signature, encode};
use serde::{Deserialize, Serialize};

/// What an operation does. The same list as `ability` in `wit/fos.wit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Ability {
    Read,
    Write,
    Append,
    Grant,
    Store,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny(String),
}

/// The rules of a node: its authorization engine.
///
/// The core asks; the engine answers. The engine keeps its own state for each
/// folder as bytes, and the folder keeps these bytes for it. The methods take
/// `&mut self` because an engine is often a WebAssembly instance.
pub trait Authz {
    /// The state of a new folder.
    fn init(&mut self, folder: Hash, creator: PublicKey) -> Vec<u8>;
    /// Reads one policy entry. Gives the new state, or the reason it is not taken.
    ///
    /// # Errors
    ///
    /// When the engine does not take the entry.
    fn apply(&mut self, state: &[u8], author: PublicKey, policy: &[u8]) -> Result<Vec<u8>, String>;
    /// Says if `principal` may do `ability` on `path`.
    fn check(
        &mut self,
        state: &[u8],
        principal: PublicKey,
        ability: Ability,
        folder: Hash,
        path: &str,
    ) -> Decision;
    /// The keys that may read the folder.
    fn readers(&mut self, state: &[u8]) -> Vec<PublicKey>;
    /// The keys of the nodes that may keep a copy of the folder.
    fn storers(&mut self, state: &[u8]) -> Vec<PublicKey>;
    /// The policy entry that gives `to` these abilities on `path`.
    fn grant(&mut self, to: PublicKey, abilities: &[Ability], path: &str) -> Vec<u8>;
}

/// One operation on a folder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Op {
    /// The first entry. Its author makes the folder and is its first ordering node.
    Genesis { name: String },
    /// Put a file. The bytes are the blob with this hash.
    Put { path: String, blob: Hash, size: u64 },
    /// Take a file or a mount away. Old versions stay in the log.
    Delete { path: String },
    /// Add one record to the end of a stream.
    Append { path: String, data: Bytes },
    /// Put another folder at a path.
    Mount { path: String, folder: Hash },
    /// A change to the rules. Only the engine gives it meaning.
    Policy(Bytes),
    /// Give the job of putting writes in order to another node.
    SetOrderer(PublicKey),
}

impl Op {
    /// The ability that the author needs, and the path it is needed on.
    /// Policy entries are checked by the engine itself, in [`Authz::apply`].
    fn needs(&self) -> Option<(Ability, &str)> {
        match self {
            Op::Put { path, .. } | Op::Delete { path } | Op::Mount { path, .. } => {
                Some((Ability::Write, path))
            }
            Op::Append { path, .. } => Some((Ability::Append, path)),
            Op::SetOrderer(_) => Some((Ability::Grant, "/")),
            Op::Genesis { .. } | Op::Policy(_) => None,
        }
    }
}

/// A write, before it is signed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Write {
    /// `None` only for the genesis write, because the folder ID is its hash.
    pub folder: Option<Hash>,
    pub author: PublicKey,
    /// Makes two writes with the same operation different.
    pub nonce: u64,
    pub op: Op,
}

/// A write signed by its author.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignedWrite {
    pub write: Write,
    pub sig: Signature,
}

impl SignedWrite {
    /// Signs a write. The author is the key that signs.
    pub fn sign(folder: Option<Hash>, nonce: u64, op: Op, key: &SecretKey) -> Self {
        let write = Write { folder, author: key.public(), nonce, op };
        let sig = key.sign(&encode(&write));
        Self { write, sig }
    }

    pub fn verify(&self) -> bool {
        self.write.author.verify(&encode(&self.write), &self.sig)
    }

    pub fn hash(&self) -> Hash {
        Hash::of_value(self)
    }
}

/// A write in its place in the main line, signed by the ordering node.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub folder: Hash,
    pub seq: u64,
    /// The hash of the entry before this one. Zero for the genesis entry.
    pub prev: Hash,
    pub write: SignedWrite,
    pub orderer: PublicKey,
    pub sig: Signature,
}

impl Entry {
    fn message(folder: Hash, seq: u64, prev: Hash, write: &SignedWrite, orderer: PublicKey) -> Vec<u8> {
        encode(&(folder, seq, prev, write.hash(), orderer))
    }

    fn sign(folder: Hash, seq: u64, prev: Hash, write: SignedWrite, key: &SecretKey) -> Self {
        let orderer = key.public();
        let sig = key.sign(&Self::message(folder, seq, prev, &write, orderer));
        Self { folder, seq, prev, write, orderer, sig }
    }

    fn verify(&self) -> bool {
        let message = Self::message(self.folder, self.seq, self.prev, &self.write, self.orderer);
        self.orderer.verify(&message, &self.sig) && self.write.verify()
    }

    pub fn hash(&self) -> Hash {
        Hash::of_value(self)
    }
}

/// One version of a file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Version {
    pub seq: u64,
    pub author: PublicKey,
    /// The blob and its size, or `None` when the file was deleted.
    pub blob: Option<(Hash, u64)>,
}

/// One record of a stream.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub seq: u64,
    pub author: PublicKey,
    pub data: Vec<u8>,
}

/// What is at a path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// Every version, the oldest first.
    File(Vec<Version>),
    Stream(Vec<Record>),
    Mount(Hash),
}

/// The place where a copy stopped following the main line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fork {
    /// The entry that this node's engine did not take.
    pub seq: u64,
    pub reason: String,
}

/// Why an entry was not taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reject {
    /// This copy already has the entry.
    Old,
    /// Entries before this one are missing. Get them first.
    Gap { next: u64 },
    /// The entry is broken or not from the ordering node. Drop it.
    Invalid(String),
    /// This node's engine did not take the entry, so this copy forks here.
    Fork(Fork),
    /// This copy forked before; it takes no more entries.
    Forked,
}

/// A copy of one folder: its log and the state made from it.
#[derive(Clone, Debug)]
pub struct Folder {
    id: Hash,
    name: String,
    creator: PublicKey,
    orderer: PublicKey,
    log: Vec<Entry>,
    tree: BTreeMap<String, Node>,
    policy: Vec<u8>,
    fork: Option<Fork>,
}

/// A change to the state that a write makes, found before it is made.
enum Change {
    Put(String, Version),
    Append(String, Record),
    Mount(String, Hash),
    Unmount(String),
    Policy(Vec<u8>),
    Orderer(PublicKey),
}

impl Folder {
    /// Makes a new folder. Gives the folder and its genesis entry.
    pub fn create(name: &str, key: &SecretKey, authz: &mut dyn Authz) -> (Folder, Entry) {
        let write = SignedWrite::sign(None, 0, Op::Genesis { name: name.into() }, key);
        let entry = Entry::sign(write.hash(), 0, Hash::default(), write, key);
        let folder = Self::from_genesis(entry.clone(), authz).expect("a new genesis entry is valid");
        (folder, entry)
    }

    /// Starts a copy of a folder from its genesis entry.
    ///
    /// # Errors
    ///
    /// When the entry is not a valid genesis entry.
    pub fn from_genesis(entry: Entry, authz: &mut dyn Authz) -> Result<Folder, Error> {
        let bad = |why: &str| Err(Error(format!("not a genesis entry: {why}")));
        let Op::Genesis { name } = &entry.write.write.op else {
            return bad("the operation is not genesis");
        };
        if entry.seq != 0 || entry.prev != Hash::default() || entry.write.write.folder.is_some() {
            return bad("wrong place in the log");
        }
        if entry.folder != entry.write.hash() {
            return bad("the folder ID is not the hash of the genesis write");
        }
        let creator = entry.write.write.author;
        if entry.orderer != creator || !entry.verify() {
            return bad("bad signature");
        }
        Ok(Folder {
            id: entry.folder,
            name: name.clone(),
            creator,
            orderer: creator,
            policy: authz.init(entry.folder, creator),
            log: vec![entry],
            tree: BTreeMap::new(),
            fork: None,
        })
    }

    pub fn id(&self) -> Hash {
        self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn creator(&self) -> PublicKey {
        self.creator
    }

    /// The node that puts writes in order.
    pub fn orderer(&self) -> PublicKey {
        self.orderer
    }

    /// The `seq` of the next entry.
    pub fn next_seq(&self) -> u64 {
        self.log.len() as u64
    }

    /// The hash of the last entry.
    pub fn head(&self) -> Hash {
        self.log.last().expect("a folder has a genesis entry").hash()
    }

    /// Entries from `seq` on.
    pub fn entries_from(&self, seq: u64) -> &[Entry] {
        self.log.get(seq as usize..).unwrap_or_default()
    }

    /// The engine's state for this folder.
    pub fn policy(&self) -> &[u8] {
        &self.policy
    }

    /// Where this copy stopped following the main line, if it did.
    pub fn fork(&self) -> Option<&Fork> {
        self.fork.as_ref()
    }

    pub fn get(&self, path: &str) -> Option<&Node> {
        self.tree.get(path)
    }

    /// The current blob of a file and its size.
    pub fn file(&self, path: &str) -> Option<(Hash, u64)> {
        match self.tree.get(path)? {
            Node::File(versions) => versions.last()?.blob,
            _ => None,
        }
    }

    /// Every file that is there now, with its blob and size, by path.
    pub fn files(&self) -> impl Iterator<Item = (&str, Hash, u64)> {
        self.tree.keys().filter_map(|path| self.file(path).map(|(hash, size)| (path.as_str(), hash, size)))
    }

    /// The paths where other folders are mounted, with their IDs.
    pub fn mounts(&self) -> impl Iterator<Item = (&str, Hash)> {
        self.tree.iter().filter_map(|(path, node)| match node {
            Node::Mount(id) => Some((path.as_str(), *id)),
            _ => None,
        })
    }

    /// Every blob that the current state names.
    pub fn blobs(&self) -> Vec<Hash> {
        self.tree
            .keys()
            .filter_map(|path| self.file(path).map(|(hash, _)| hash))
            .collect()
    }

    /// The names directly under `dir`, without deleted files.
    pub fn list(&self, dir: &str) -> Vec<String> {
        let prefix = if dir == "/" { "/".to_string() } else { format!("{dir}/") };
        let mut names: Vec<String> = Vec::new();
        for (path, node) in self.tree.range(prefix.clone()..) {
            let Some(rest) = path.strip_prefix(&prefix) else { break };
            if matches!(node, Node::File(v) if v.last().is_some_and(|v| v.blob.is_none())) {
                continue;
            }
            let name = rest.split('/').next().unwrap_or(rest);
            if names.last().is_none_or(|last| last != name) {
                names.push(name.to_string());
            }
        }
        names
    }

    /// Asks the engine if `principal` may do `ability` on `path`.
    pub fn check(&self, authz: &mut dyn Authz, principal: PublicKey, ability: Ability, path: &str) -> Decision {
        authz.check(&self.policy, principal, ability, self.id, path)
    }

    /// The nodes that may keep a copy, by the engine.
    pub fn storers(&self, authz: &mut dyn Authz) -> Vec<PublicKey> {
        authz.storers(&self.policy)
    }

    /// Says if a write would be taken now, without making it.
    ///
    /// # Errors
    ///
    /// The reason it would not be taken.
    pub fn try_write(&self, write: &SignedWrite, authz: &mut dyn Authz) -> Result<(), String> {
        if !write.verify() || write.write.folder != Some(self.id) {
            return Err("the write is not signed for this folder".into());
        }
        self.validate(&write.write, self.next_seq(), authz).map(|_| ())
    }

    /// Puts a write in order and takes it. Only the ordering node does this.
    ///
    /// # Errors
    ///
    /// When `key` is not the ordering node, the write is not valid, or the
    /// engine says no.
    pub fn order(&mut self, write: SignedWrite, key: &SecretKey, authz: &mut dyn Authz) -> Result<Entry, Error> {
        if key.public() != self.orderer {
            return Err(Error("this node does not put this folder in order".into()));
        }
        if !write.verify() || write.write.folder != Some(self.id) {
            return Err(Error("the write is not signed for this folder".into()));
        }
        if self.fork.is_some() {
            return Err(Error("this copy has forked".into()));
        }
        let change = self.validate(&write.write, self.next_seq(), authz).map_err(Error)?;
        let entry = Entry::sign(self.id, self.next_seq(), self.head(), write, key);
        self.commit(entry.clone(), change);
        Ok(entry)
    }

    /// Takes an entry from the main line, if this node's engine says yes.
    ///
    /// # Errors
    ///
    /// See [`Reject`].
    pub fn accept(&mut self, entry: Entry, authz: &mut dyn Authz) -> Result<(), Reject> {
        if self.fork.is_some() {
            return Err(Reject::Forked);
        }
        if entry.folder != self.id {
            return Err(Reject::Invalid("the entry is for another folder".into()));
        }
        if entry.seq < self.next_seq() {
            return Err(Reject::Old);
        }
        if entry.seq > self.next_seq() {
            return Err(Reject::Gap { next: self.next_seq() });
        }
        if entry.prev != self.head() {
            return Err(Reject::Invalid("the entry does not follow the head".into()));
        }
        if entry.orderer != self.orderer || !entry.verify() || entry.write.write.folder != Some(self.id) {
            return Err(Reject::Invalid("bad signature".into()));
        }
        match self.validate(&entry.write.write, entry.seq, authz) {
            Ok(change) => {
                self.commit(entry, change);
                Ok(())
            }
            Err(reason) => {
                let fork = Fork { seq: entry.seq, reason };
                self.fork = Some(fork.clone());
                Err(Reject::Fork(fork))
            }
        }
    }

    /// Says what a write changes, or why it may not be made.
    fn validate(&self, write: &Write, seq: u64, authz: &mut dyn Authz) -> Result<Change, String> {
        if let Some((ability, path)) = write.op.needs() {
            check_path(path)?;
            if let Decision::Deny(reason) = self.check(authz, write.author, ability, path) {
                return Err(reason);
            }
        }
        let author = write.author;
        Ok(match &write.op {
            Op::Genesis { .. } => return Err("a folder has only one genesis entry".into()),
            Op::Put { path, blob, size } => {
                if matches!(self.tree.get(path), Some(Node::Stream(_) | Node::Mount(_))) {
                    return Err(format!("{path} is not a file"));
                }
                Change::Put(path.clone(), Version { seq, author, blob: Some((*blob, *size)) })
            }
            Op::Delete { path } => match self.tree.get(path) {
                Some(Node::File(_)) if self.file(path).is_some() => {
                    Change::Put(path.clone(), Version { seq, author, blob: None })
                }
                Some(Node::Mount(_)) => Change::Unmount(path.clone()),
                _ => return Err(format!("{path} is not there")),
            },
            Op::Append { path, data } => {
                if matches!(self.tree.get(path), Some(Node::File(_) | Node::Mount(_))) {
                    return Err(format!("{path} is not a stream"));
                }
                Change::Append(path.clone(), Record { seq, author, data: data.0.clone() })
            }
            Op::Mount { path, folder } => {
                if !matches!(self.tree.get(path), None | Some(Node::Mount(_))) {
                    return Err(format!("{path} is already there"));
                }
                Change::Mount(path.clone(), *folder)
            }
            Op::Policy(bytes) => Change::Policy(authz.apply(&self.policy, author, &bytes.0)?),
            Op::SetOrderer(key) => Change::Orderer(*key),
        })
    }

    fn commit(&mut self, entry: Entry, change: Change) {
        match change {
            Change::Put(path, version) => match self.tree.entry(path).or_insert(Node::File(Vec::new())) {
                Node::File(versions) => versions.push(version),
                _ => unreachable!("validate checked the kind of node"),
            },
            Change::Append(path, record) => match self.tree.entry(path).or_insert(Node::Stream(Vec::new())) {
                Node::Stream(records) => records.push(record),
                _ => unreachable!("validate checked the kind of node"),
            },
            Change::Mount(path, folder) => {
                self.tree.insert(path, Node::Mount(folder));
            }
            Change::Unmount(path) => {
                self.tree.remove(&path);
            }
            Change::Policy(state) => self.policy = state,
            Change::Orderer(key) => self.orderer = key,
        }
        self.log.push(entry);
    }
}

/// Checks that a path starts with "/" and has no empty, "." or ".." parts.
///
/// # Errors
///
/// When the path is not like that.
pub fn check_path(path: &str) -> Result<(), String> {
    if path == "/" {
        return Ok(());
    }
    let ok = path.starts_with('/')
        && path[1..].split('/').all(|part| !part.is_empty() && part != "." && part != "..")
        && !path.chars().any(char::is_control);
    if ok { Ok(()) } else { Err(format!("not a valid path: {path:?}")) }
}

/// Follows mounts from the root folder. Gives the folder that has `path`,
/// and the path inside that folder.
///
/// # Errors
///
/// When the path is not valid, or a mounted folder is not on this node.
pub fn resolve<'a>(
    root: Hash,
    path: &str,
    lookup: impl Fn(Hash) -> Option<&'a Folder>,
) -> Result<(Hash, String), Error> {
    check_path(path).map_err(Error)?;
    let mut folder = root;
    let mut parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    'outer: loop {
        let current = lookup(folder).ok_or_else(|| Error(format!("folder {folder} is not on this node")))?;
        for i in 1..=parts.len() {
            let prefix = format!("/{}", parts[..i].join("/"));
            if let Some(Node::Mount(inner)) = current.get(&prefix) {
                folder = *inner;
                parts.drain(..i);
                continue 'outer;
            }
        }
        return Ok((folder, format!("/{}", parts.join("/"))));
    }
}

/// An engine that says yes to every key except the ones in `deny`. Every
/// key in `storers` may keep a copy. For tests and the simulation.
#[derive(Clone, Debug, Default)]
pub struct Open {
    pub deny: Vec<PublicKey>,
    pub storers: Vec<PublicKey>,
}

impl Authz for Open {
    fn init(&mut self, _folder: Hash, _creator: PublicKey) -> Vec<u8> {
        Vec::new()
    }

    fn apply(&mut self, state: &[u8], author: PublicKey, _policy: &[u8]) -> Result<Vec<u8>, String> {
        if self.deny.contains(&author) { Err("denied".into()) } else { Ok(state.to_vec()) }
    }

    fn check(&mut self, _: &[u8], principal: PublicKey, _: Ability, _: Hash, _: &str) -> Decision {
        if self.deny.contains(&principal) { Decision::Deny("denied".into()) } else { Decision::Allow }
    }

    fn readers(&mut self, _state: &[u8]) -> Vec<PublicKey> {
        self.storers.clone()
    }

    fn storers(&mut self, _state: &[u8]) -> Vec<PublicKey> {
        self.storers.clone()
    }

    fn grant(&mut self, _to: PublicKey, _abilities: &[Ability], _path: &str) -> Vec<u8> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests;
