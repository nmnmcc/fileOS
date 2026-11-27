// SPDX-License-Identifier: AGPL-3.0-only

//! One space, and the state that comes from its entries.
//!
//! The state is worked out again from all the entries after every change, in
//! causal order:
//!
//! 1. **Causal check.** An entry is valid when its author had the
//!    permissions for every act in it, in the state of its causal past.
//! 2. **Finality.** An entry made at the same time as a valid checkpoint (not
//!    in its past, and not after it) has no effect. Its author's node puts
//!    its acts on top again, as new entries (see [`Space::take_stale`]).
//! 3. **Strong removal.** The causal check again, but a grant also counts as
//!    taken back when a `Take` of it that was valid in step 1 is
//!    *concurrent* with the entry. So what a person did at the same time as
//!    their access was taken away has no effect, and nor does what depends on
//!    it. Two managers who take each other's access at the same time both
//!    lose it. The owner can never lose access.
//!
//! The result depends only on the set of entries, never on the order they
//! came in, so every node with the same entries gets the same state.
//!
//! **Checkpoints** keep this small. A checkpoint signs a hash of the state
//! in its past. At an owner-signed checkpoint a node can
//! [`compact`](Space::compact): it keeps the state as a [`Snapshot`] and
//! drops the earlier entries. A new node can check the owner signature
//! and start from that snapshot.

use std::collections::{BTreeMap, BTreeSet};

use fileos_core::{Error, Hash, PublicKey, SecretKey};
use fileos_id::{NodeCert, NodeRevocation, Op as DidOp};
use serde::{Deserialize, Serialize};

use crate::crypto::{self, Key, Recipient, Wrapped};
use crate::entry::{
    Act, Content, Effect, Entry, Id, KeyRef, Permission, Principal, Resource, Retention, Sealed,
};

/// The place of what came from the base, before every entry.
const BASE: usize = usize::MAX;

/// An epoch older than this is changed for a new one, so that keys of
/// deleted data and old secrets stop being of use (a policy, with a
/// default: 30 days, the time deleted files stay in the bin).
pub const EPOCH_MAX_AGE: u64 = 30 * 24 * 60 * 60 * 1000;

/// One version of a file.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Version {
    pub id: Id,
    pub name: String,
    /// `None` when this version deletes the file.
    pub content: Option<Content>,
    pub author: PublicKey,
    /// The time the author says it was made, in milliseconds since 1970.
    /// Only for people and for retention; never for access.
    pub time: u64,
    pub parents: Vec<Id>,
    /// The data key, wrapped, when the content is encrypted.
    #[serde(default)]
    pub key: Option<Wrapped>,
}

/// A file and its newest versions. There is more than one when people
/// changed it at the same time; all are kept, side by side.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct File {
    pub id: Id,
    pub versions: Vec<Version>,
}

impl File {
    /// Says if every newest version deletes the file.
    pub fn deleted(&self) -> bool {
        self.versions.iter().all(|v| v.content.is_none())
    }

    /// The names of the newest versions.
    pub fn names(&self) -> BTreeSet<&str> {
        self.versions.iter().map(|v| v.name.as_str()).collect()
    }
}

/// A grant as it was given.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Given {
    pub id: Id,
    pub giver: Principal,
    pub to: Principal,
    pub effect: Effect,
    pub permissions: Vec<Permission>,
    pub on: Resource,
}

/// A grant that is in place.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Grant {
    pub given: Given,
    /// False when the giver no longer has what it gave (taken back above it).
    pub in_effect: bool,
}

/// What a node needs to start a space at a checkpoint.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub genesis: Entry,
    /// The checkpoint, which signs the hash of `summary`.
    pub checkpoint: Entry,
    pub summary: Summary,
    /// Signed entries with older versions that the retention policy keeps.
    /// They are not in the hash: history is kept as each node has it.
    pub history: Vec<Entry>,
}

/// An entry that is in the graph.
#[derive(Clone, Debug)]
struct Known {
    entry: Entry,
    acts: Vec<Act>,
}

mod past;
mod summary;
mod view;

#[doc(inline)]
pub use summary::Summary;

use past::Past;
use view::{Target, View};

/// A version, with the place of its entry in causal order.
#[derive(Clone, Debug)]
struct Ver {
    at: usize,
    file: Id,
    version: Version,
    /// An older version kept by retention in the base: history only.
    old: bool,
}

/// A grant, with the place of its entry in causal order.
#[derive(Clone, Debug)]
struct Giv {
    at: usize,
    given: Given,
}

/// The state worked out from the base and the entries.
#[derive(Clone, Debug, Default)]
struct State {
    /// Hashes in causal order (ties by hash).
    order: Vec<Hash>,
    place: BTreeMap<Hash, usize>,
    /// The strict causal past of each entry.
    past: Vec<Past>,
    versions: Vec<Ver>,
    by_file: BTreeMap<Id, Vec<usize>>,
    by_name: BTreeMap<String, BTreeSet<Id>>,
    gives: Vec<Giv>,
    /// (place of the entry, the grant taken back)
    takes: Vec<(usize, Id)>,
    /// (place of the entry, the ID of a new epoch)
    epochs: Vec<(usize, Hash)>,
    /// (place of the entry, keys sealed to a device)
    sealed: Vec<(usize, Sealed)>,
    /// (place of the entry, a file, its data key wrapped again)
    rewraps: Vec<(usize, Id, Wrapped)>,
    /// (place of the entry, a device, its recipient key, the time)
    recipients: Vec<(usize, PublicKey, Key, u64)>,
    /// (place of the entry, the group, its owner)
    groups: Vec<(usize, Id, Principal)>,
    /// (place of the entry, the identity, its DID operations, a certificate)
    vouches: Vec<(usize, Hash, Vec<DidOp>, Option<NodeCert>)>,
    /// (place of the entry, the identity, a node revocation)
    unvouches: Vec<(usize, Hash, NodeRevocation)>,
    /// The result of step 1.
    causal: Vec<bool>,
    /// Entries made at the same time as a checkpoint (step 2).
    stale: Vec<bool>,
    /// The result of step 3: the entries that have effect.
    valid: Vec<bool>,
    /// Why entries have no effect.
    why: BTreeMap<Hash, String>,
}

/// The base of a compacted space.
#[derive(Clone, Debug)]
struct Base {
    snapshot: Snapshot,
    /// Hashes that entries may come after although they are not here: the
    /// past of the checkpoint, as far as new entries name it.
    gone: BTreeSet<Hash>,
}

/// One space: its entries and the state that comes from them.
#[derive(Clone, Debug)]
pub struct Space {
    id: Hash,
    owner: Principal,
    genesis: Entry,
    /// The state at the last compaction, if any.
    base: Option<Base>,
    known: BTreeMap<Hash, Known>,
    /// Entries that wait for entries they come after.
    waiting: BTreeMap<Hash, Entry>,
    /// Stale entries of this node whose acts were already put on top again.
    rebased: BTreeSet<Hash>,
    state: State,
}

impl Space {
    /// Makes a new space owned by `owner`. `author` signs the first entry.
    /// `nonce` should be random.
    ///
    /// # Panics
    ///
    /// Never: the first entry it makes is always good.
    pub fn create(owner: Principal, author: &SecretKey, time: u64, nonce: u64) -> (Self, Entry) {
        let entry = Entry::sign(
            None,
            0,
            None,
            Vec::new(),
            time,
            &[Act::Genesis { owner, nonce }],
            author,
        );
        let space = Self::from_genesis(&entry).expect("a genesis entry that was just made");
        (space, entry)
    }

    fn owner_of(entry: &Entry) -> Result<Principal, Error> {
        let acts = entry.open()?;
        let h = &entry.header;
        match acts.as_slice() {
            [Act::Genesis { owner, .. }]
                if h.space.is_none() && h.seq == 0 && h.prev.is_none() && h.deps.is_empty() =>
            {
                Ok(*owner)
            }
            _ => Err(Error::invalid("not the first entry of a space".into())),
        }
    }

    /// Starts a space from its first entry, as a node that follows it does.
    ///
    /// # Errors
    ///
    /// When the entry is not a good first entry.
    pub fn from_genesis(entry: &Entry) -> Result<Self, Error> {
        let owner = Self::owner_of(entry)?;
        let id = entry.hash();
        let mut space = Self {
            id,
            owner,
            genesis: entry.clone(),
            base: None,
            known: BTreeMap::new(),
            waiting: BTreeMap::new(),
            rebased: BTreeSet::new(),
            state: State::default(),
        };
        space.known.insert(
            id,
            Known {
                entry: entry.clone(),
                acts: entry.open()?,
            },
        );
        space.work_out();
        Ok(space)
    }

    /// Starts a space from a snapshot signed by its original owner key.
    ///
    /// # Errors
    ///
    /// When the snapshot is invalid or lacks an owner signature.
    pub fn from_snapshot(snapshot: Snapshot) -> Result<Self, Error> {
        let owner = Self::owner_of(&snapshot.genesis)?;
        if owner != Principal::Key(snapshot.checkpoint.header.author) {
            return Err(Error::invalid(
                "a new snapshot needs the space owner's signature".into(),
            ));
        }
        Self::with_base(snapshot, owner)
    }

    /// Restores a snapshot this node already checked and saved on disk.
    ///
    /// # Errors
    ///
    /// When the saved snapshot is invalid.
    pub fn restore_local_snapshot(snapshot: Snapshot) -> Result<Self, Error> {
        let owner = Self::owner_of(&snapshot.genesis)?;
        Self::with_base(snapshot, owner)
    }

    fn with_base(snapshot: Snapshot, owner: Principal) -> Result<Self, Error> {
        let id = snapshot.genesis.hash();
        let mut space = Self {
            id,
            owner,
            genesis: snapshot.genesis.clone(),
            base: None,
            known: BTreeMap::new(),
            waiting: BTreeMap::new(),
            rebased: BTreeSet::new(),
            state: State::default(),
        };
        space.set_base(snapshot)?;
        Ok(space)
    }

    /// Makes `snapshot` the base, with only its checkpoint as an entry.
    fn set_base(&mut self, snapshot: Snapshot) -> Result<(), Error> {
        let checkpoint = &snapshot.checkpoint;
        if checkpoint.header.space != Some(self.id) {
            return Err(Error::invalid("a checkpoint of another space".into()));
        }
        let acts = checkpoint.open()?;
        let [Act::Checkpoint { state, .. }] = acts.as_slice() else {
            return Err(Error::invalid("not a checkpoint".into()));
        };
        if !snapshot.summary.matches_hash(*state) {
            return Err(Error::invalid(
                "the snapshot is not the one the checkpoint signs".into(),
            ));
        }
        let mut gone: BTreeSet<Hash> = checkpoint.header.parents().collect();
        gone.extend(snapshot.summary.logs.iter().map(|(_, h, _)| *h));
        let hash = checkpoint.hash();
        let entry = checkpoint.clone();
        self.base = Some(Base { snapshot, gone });
        self.known = BTreeMap::from([(hash, Known { entry, acts })]);
        self.work_out();
        if !self.state.valid.first().copied().unwrap_or(false) {
            return Err(Error::invalid(format!(
                "the checkpoint is not valid: {}",
                self.state.why.get(&hash).cloned().unwrap_or_default()
            )));
        }
        Ok(())
    }

    /// The ID of the space: the hash of its first entry.
    pub fn id(&self) -> Hash {
        self.id
    }

    pub fn owner(&self) -> Principal {
        self.owner
    }

    /// The number of entries in the graph (not counting the base).
    pub fn len(&self) -> usize {
        self.known.len()
    }

    /// Never true: a space always has its first entry or a checkpoint.
    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    pub fn has(&self, entry: Hash) -> bool {
        self.known.contains_key(&entry)
    }

    /// The checkpoint that the base is at, if the space was compacted.
    pub fn base(&self) -> Option<&Snapshot> {
        self.base.as_ref().map(|b| &b.snapshot)
    }

    /// The entries that no other entry comes after.
    pub fn heads(&self) -> Vec<Hash> {
        let mut heads: BTreeSet<Hash> = self.known.keys().copied().collect();
        for k in self.known.values() {
            for p in k.entry.header.parents() {
                heads.remove(&p);
            }
        }
        heads.into_iter().collect()
    }

    /// For each author, the number of entries of its log here, with the base.
    pub fn logs(&self) -> BTreeMap<PublicKey, u64> {
        let mut logs: BTreeMap<PublicKey, u64> = self
            .base()
            .map(|b| b.summary.logs.iter().map(|(k, _, s)| (*k, s + 1)).collect())
            .unwrap_or_default();
        for k in self.known.values() {
            let n = logs.entry(k.entry.header.author).or_insert(0);
            *n = (*n).max(k.entry.header.seq + 1);
        }
        logs
    }

    /// Says if a node with `logs` lacks entries that are only in the base
    /// here, so that it needs the snapshot.
    pub fn needs_base(&self, logs: &BTreeMap<PublicKey, u64>) -> bool {
        self.base().is_some_and(|b| {
            b.summary
                .logs
                .iter()
                .any(|(k, _, s)| logs.get(k).copied().unwrap_or(0) <= *s)
        })
    }

    /// The entries that a node with `logs` does not have, in causal order.
    pub fn missing(&self, logs: &BTreeMap<PublicKey, u64>) -> Vec<Entry> {
        self.iter()
            .filter(|e| e.header.seq >= logs.get(&e.header.author).copied().unwrap_or(0))
            .cloned()
            .collect()
    }

    /// Every entry here, in causal order: the first entry, or the checkpoint
    /// of the base, and the entries after it.
    pub fn entries(&self) -> Vec<Entry> {
        self.iter().cloned().collect()
    }

    /// Borrows the entries in causal order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &Entry> {
        self.state.order.iter().map(|hash| &self.known[hash].entry)
    }

    /// Takes entries from another node. Entries that come after entries this
    /// space does not have yet wait for them. Gives back the hashes of the
    /// entries that were added to the graph.
    ///
    /// # Errors
    ///
    /// When an entry is bad (a wrong signature, another space, a broken log).
    /// The good entries are still taken.
    ///
    /// # Panics
    ///
    /// Never: only entries that were made ready are taken out.
    pub fn insert(&mut self, entries: Vec<Entry>) -> Result<Vec<Hash>, Error> {
        let (added, error) = self.insert_batch(entries);
        error.map_or(Ok(added), Err)
    }

    pub(crate) fn insert_batch(&mut self, entries: Vec<Entry>) -> (Vec<Hash>, Option<Error>) {
        let mut first_error = None;
        for entry in entries {
            let hash = entry.hash();
            if self.known.contains_key(&hash) || self.base_has(&entry) {
                continue;
            }
            if let Err(e) = self.check_form(&entry) {
                first_error.get_or_insert(e);
                continue;
            }
            self.waiting.insert(hash, entry);
        }
        let mut added = Vec::new();
        loop {
            let ready: Vec<Hash> = self
                .waiting
                .iter()
                .filter(|(_, e)| e.header.parents().all(|p| self.is_here(p)))
                .map(|(h, _)| *h)
                .collect();
            if ready.is_empty() {
                break;
            }
            for hash in ready {
                let entry = self.waiting.remove(&hash).expect("a waiting entry");
                match self.check_log(&entry) {
                    Ok(acts) => {
                        self.known.insert(hash, Known { entry, acts });
                        added.push(hash);
                    }
                    Err(e) => {
                        first_error.get_or_insert(e);
                    }
                }
            }
        }
        if !added.is_empty() {
            self.work_out();
        }
        (added, first_error)
    }

    pub(crate) fn entry(&self, hash: Hash) -> Option<&Entry> {
        self.known.get(&hash).map(|known| &known.entry)
    }

    /// Says if `entry` is in the past of the base, as far as the logs say.
    fn base_has(&self, entry: &Entry) -> bool {
        self.base().is_some_and(|b| {
            b.summary
                .logs
                .iter()
                .any(|(k, _, s)| *k == entry.header.author && entry.header.seq <= *s)
        })
    }

    fn is_here(&self, hash: Hash) -> bool {
        self.known.contains_key(&hash) || self.base.as_ref().is_some_and(|b| b.gone.contains(&hash))
    }

    /// The checks that need no other entry.
    fn check_form(&self, entry: &Entry) -> Result<(), Error> {
        let h = &entry.header;
        if h.space != Some(self.id) {
            return Err(Error::invalid(format!(
                "an entry that is not of space {}",
                self.id
            )));
        }
        if (h.seq == 0) != h.prev.is_none() {
            return Err(Error::invalid(
                "an entry whose place in its log is wrong".into(),
            ));
        }
        if h.prev.is_none() && h.deps.is_empty() {
            return Err(Error::invalid("an entry that comes after nothing".into()));
        }
        if h.deps.len() > 256 {
            return Err(Error::invalid("an entry with too many dependencies".into()));
        }
        entry.open().map(drop)
    }

    /// The checks that need the entries this one comes after.
    fn check_log(&self, entry: &Entry) -> Result<Vec<Act>, Error> {
        let h = &entry.header;
        if let Some(prev) = h.prev {
            let (author, seq) = match self.known.get(&prev) {
                Some(p) => (p.entry.header.author, p.entry.header.seq),
                None => self
                    .base()
                    .and_then(|b| b.summary.logs.iter().find(|(_, x, _)| *x == prev))
                    .map(|(k, _, s)| (*k, *s))
                    .ok_or_else(|| Error::invalid("an entry after one that is gone".into()))?,
            };
            if author != h.author || seq + 1 != h.seq {
                return Err(Error::invalid("an entry whose log is broken".into()));
            }
        }
        entry.open()
    }

    /// Works out the state from the base and all entries again.
    fn work_out(&mut self) {
        let mut state = State::default();
        // Causal order, with ties by hash (Kahn's algorithm). Parents in the
        // base are before everything.
        let mut children: BTreeMap<Hash, Vec<Hash>> = BTreeMap::new();
        let mut parents_left: BTreeMap<Hash, usize> = BTreeMap::new();
        for (h, k) in &self.known {
            let parents: BTreeSet<Hash> = k
                .entry
                .header
                .parents()
                .filter(|p| self.known.contains_key(p))
                .collect();
            parents_left.insert(*h, parents.len());
            for p in parents {
                children.entry(p).or_default().push(*h);
            }
        }
        let mut ready: BTreeSet<Hash> = parents_left
            .iter()
            .filter(|(_, n)| **n == 0)
            .map(|(h, _)| *h)
            .collect();
        while let Some(h) = ready.pop_first() {
            state.place.insert(h, state.order.len());
            state.order.push(h);
            for c in children.get(&h).into_iter().flatten() {
                let n = parents_left.get_mut(c).expect("a known child");
                *n -= 1;
                if *n == 0 {
                    ready.insert(*c);
                }
            }
        }
        let n = state.order.len();
        state.past.reserve(n);
        for i in 0..n {
            let mut past = Past::default();
            for p in self.known[&state.order[i]].entry.header.parents() {
                if let Some(&j) = state.place.get(&p) {
                    past.add_parent(j, &state.past[j]);
                }
            }
            state.past.push(past);
        }
        // What the base and each entry do.
        if let Some(base) = &self.base {
            for (file, versions) in &base.snapshot.summary.files {
                for version in versions {
                    state.versions.push(Ver {
                        at: BASE,
                        file: *file,
                        version: version.clone(),
                        old: false,
                    });
                }
            }
            for entry in &base.snapshot.history {
                if let Ok(acts) = entry.open() {
                    for (index, act) in acts.iter().enumerate() {
                        if let Some(ver) = version_of(entry, index, act, BASE) {
                            state.versions.push(Ver { old: true, ..ver });
                        }
                    }
                }
            }
            for (id, owner) in &base.snapshot.summary.groups {
                state.groups.push((BASE, *id, *owner));
            }
            for r in &base.snapshot.summary.revocations {
                if let Ok(r) = NodeRevocation::from_bytes(&r.0)
                    && let Ok(did) = fileos_id::did_hash(&r.did)
                {
                    state.unvouches.push((BASE, did, r));
                }
            }
            for (did, ops, certs) in &base.snapshot.summary.identities {
                let ops: Vec<DidOp> = ops
                    .iter()
                    .filter_map(|o| DidOp::from_bytes(&o.0).ok())
                    .collect();
                state.vouches.push((BASE, *did, ops, None));
                for c in certs {
                    state
                        .vouches
                        .push((BASE, *did, Vec::new(), NodeCert::from_bytes(&c.0).ok()));
                }
            }
            let summary = &base.snapshot.summary;
            state
                .epochs
                .extend(summary.epochs.iter().map(|e| (BASE, *e)));
            state
                .sealed
                .extend(summary.sealed.iter().map(|x| (BASE, x.clone())));
            state
                .rewraps
                .extend(summary.rewrap.iter().map(|(f, w)| (BASE, *f, w.clone())));
            for (device, key, time) in &summary.recipients {
                if let Ok(key) = key.0.as_slice().try_into() {
                    state.recipients.push((BASE, *device, key, *time));
                }
            }
            for given in &base.snapshot.summary.grants {
                state.gives.push(Giv {
                    at: BASE,
                    given: given.clone(),
                });
            }
        }
        for (at, h) in state.order.iter().enumerate() {
            let k = &self.known[h];
            for (index, act) in k.acts.iter().enumerate() {
                if let Some(ver) = version_of(&k.entry, index, act, at) {
                    state.versions.push(ver);
                }
                let id = Id {
                    entry: *h,
                    index: index as u32,
                };
                match act {
                    Act::Give {
                        to,
                        effect,
                        permissions,
                        on,
                    } => state.gives.push(Giv {
                        at,
                        given: Given {
                            id,
                            giver: Principal::Key(k.entry.header.author),
                            to: *to,
                            effect: *effect,
                            permissions: permissions.clone(),
                            on: on.clone(),
                        },
                    }),
                    Act::Take { grant } => state.takes.push((at, *grant)),
                    Act::Group { owner } => state.groups.push((at, id, *owner)),
                    Act::Unvouch { revocation } => {
                        if let Ok(r) = NodeRevocation::from_bytes(&revocation.0)
                            && let Ok(did) = fileos_id::did_hash(&r.did)
                        {
                            state.unvouches.push((at, did, r));
                        }
                    }
                    Act::Vouch { ops, cert } => {
                        let cert = NodeCert::from_bytes(&cert.0).ok();
                        if let Some(did) =
                            cert.as_ref().and_then(|c| fileos_id::did_hash(&c.did).ok())
                        {
                            let ops = ops
                                .iter()
                                .filter_map(|o| DidOp::from_bytes(&o.0).ok())
                                .collect();
                            state.vouches.push((at, did, ops, cert));
                        }
                    }
                    Act::Recipient { key } => {
                        if let Ok(key) = key.0.as_slice().try_into() {
                            state.recipients.push((
                                at,
                                k.entry.header.author,
                                key,
                                k.entry.header.time,
                            ));
                        }
                    }
                    Act::Keys {
                        epoch,
                        sealed,
                        rewrap,
                    } => {
                        if let Some(e) = epoch {
                            state.epochs.push((at, *e));
                        }
                        state.sealed.extend(sealed.iter().map(|x| (at, x.clone())));
                        state
                            .rewraps
                            .extend(rewrap.iter().map(|(f, w)| (at, *f, w.clone())));
                    }
                    _ => {}
                }
            }
        }
        for (index, version) in state.versions.iter().enumerate() {
            state.by_file.entry(version.file).or_default().push(index);
            if !version.old && version.version.content.is_some() {
                state
                    .by_name
                    .entry(version.version.name.clone())
                    .or_default()
                    .insert(version.file);
            }
        }
        // Step 1.
        state.stale = vec![false; n];
        let mut causal = vec![false; n];
        for i in 0..n {
            causal[i] = self.check_at(&state, i, &causal, None).is_ok();
        }
        // Step 2.
        let checkpoints: Vec<usize> = (0..n)
            .filter(|&i| causal[i] && self.is_checkpoint(&state, i))
            .collect();
        for j in 0..n {
            state.stale[j] = !self.is_checkpoint(&state, j)
                && checkpoints
                    .iter()
                    .any(|&c| c != j && !state.past[c].has(j) && !state.past[j].has(c));
        }
        // Step 3.
        let mut valid = vec![false; n];
        for i in 0..n {
            if state.stale[i] {
                state.why.insert(
                    state.order[i],
                    "made at the same time as a checkpoint; it is put on top again".into(),
                );
                continue;
            }
            match self.check_at(&state, i, &valid, Some(&causal)) {
                Ok(()) => valid[i] = true,
                Err(why) => {
                    state.why.insert(state.order[i], why.to_string());
                }
            }
        }
        state.causal = causal;
        state.valid = valid;
        self.state = state;
    }

    fn is_checkpoint(&self, state: &State, i: usize) -> bool {
        matches!(
            self.known[&state.order[i]].acts.as_slice(),
            [Act::Checkpoint { .. }]
        )
    }

    fn check_at(
        &self,
        state: &State,
        i: usize,
        valid: &[bool],
        causal: Option<&[bool]>,
    ) -> Result<(), Error> {
        let h = state.order[i];
        let k = &self.known[&h];
        let view = View {
            space: self,
            state,
            at: Some(i),
            valid,
            causal,
        };
        view.check(&k.entry, &k.acts, h == self.id)
    }

    fn now(&self) -> View<'_> {
        View {
            space: self,
            state: &self.state,
            at: None,
            valid: &self.state.valid,
            causal: Some(&self.state.causal),
        }
    }

    /// The last entry of `author` that is not stale, here or in the base.
    fn last_of(&self, author: PublicKey) -> Option<(Hash, u64)> {
        let here = self
            .state
            .order
            .iter()
            .enumerate()
            .filter(|(i, _)| !self.state.stale[*i])
            .map(|(_, h)| (*h, &self.known[h].entry.header))
            .filter(|(_, e)| e.author == author)
            .map(|(h, e)| (h, e.seq));
        let base = self
            .base()
            .into_iter()
            .flat_map(|b| b.summary.logs.iter())
            .filter(|(k, _, _)| *k == author)
            .map(|(_, h, s)| (*h, *s));
        here.chain(base).max_by_key(|(h, s)| (*s, *h))
    }

    /// Makes, signs and adds an entry by `author` after every entry here.
    /// `time` is the time now, in milliseconds since 1970.
    ///
    /// # Errors
    ///
    /// When `author` may not do these acts now. Nothing is added then.
    pub fn write(&mut self, author: &SecretKey, time: u64, acts: Vec<Act>) -> Result<Entry, Error> {
        let me = author.public();
        let prev = self.last_of(me);
        let deps: Vec<Hash> = self
            .heads()
            .into_iter()
            .filter(|h| Some(*h) != prev.map(|p| p.0))
            .filter(|h| {
                // Not after stale entries: they are put on top again.
                self.state
                    .place
                    .get(h)
                    .is_none_or(|&i| !self.state.stale[i])
            })
            .collect();
        let entry = Entry::sign(
            Some(self.id),
            prev.map_or(0, |p| p.1 + 1),
            prev.map(|p| p.0),
            deps,
            time,
            &acts,
            author,
        );
        self.now().check(&entry, &acts, false)?;
        self.insert(vec![entry.clone()])?;
        Ok(entry)
    }

    /// Makes a checkpoint of the state now, signed by `author`.
    ///
    /// # Errors
    ///
    /// When `author` may not make checkpoints.
    pub fn checkpoint(
        &mut self,
        author: &SecretKey,
        time: u64,
        retention: Retention,
    ) -> Result<Entry, Error> {
        let state = self.now().summary(time, &retention).hash();
        self.write(author, time, vec![Act::Checkpoint { state, retention }])
    }

    /// The acts of `author` that were made at the same time as a checkpoint
    /// and so have no effect, made ready to be written again on top, with
    /// the time they were first made. Each stale entry is given back once.
    pub fn take_stale(&mut self, author: PublicKey) -> Vec<(u64, Vec<Act>)> {
        let stale: Vec<Hash> = self
            .state
            .order
            .iter()
            .enumerate()
            .filter(|(i, h)| {
                self.state.stale[*i]
                    && self.known[*h].entry.header.author == author
                    && !self.rebased.contains(*h)
            })
            .map(|(_, h)| *h)
            .collect();
        let mut out = Vec::new();
        for h in stale {
            self.rebased.insert(h);
            let k = &self.known[&h];
            let acts: Vec<Act> = k.acts.iter().filter_map(|a| self.again(a)).collect();
            if !acts.is_empty() {
                out.push((k.entry.header.time, acts));
            }
        }
        out
    }

    /// Drops the entries before the newest valid checkpoint that every entry
    /// here is ordered with, and keeps the state there as the base. Stale
    /// entries are dropped too, so take [`take_stale`](Self::take_stale) acts first.
    /// Older versions are kept as the checkpoint's retention says. Gives back
    /// false when there is no such checkpoint.
    pub fn compact(&mut self) -> bool {
        let n = self.state.order.len();
        let Some(c) = (0..n).rev().find(|&c| {
            self.state.valid[c]
                && self.is_checkpoint(&self.state, c)
                // Keep the proof of a delegated checkpoint in the log. A
                // node that lacks the log can verify an owner signature.
                && self.owner == Principal::Key(self.known[&self.state.order[c]].entry.header.author)
                && (0..n).all(|j| {
                    j == c
                        || self.state.stale[j]
                        || self.state.past[c].has(j)
                        || self.state.past[j].has(c)
                })
        }) else {
            return false;
        };
        let hash = self.state.order[c];
        let checkpoint = self.known[&hash].entry.clone();
        let [Act::Checkpoint { retention, .. }] = self.known[&hash].acts.as_slice() else {
            return false;
        };
        let view = View {
            space: self,
            state: &self.state,
            at: Some(c),
            valid: &self.state.valid,
            causal: Some(&self.state.causal),
        };
        let summary = view.summary(checkpoint.header.time, retention);
        // Older versions that the retention keeps, with their signed entries
        // (from this graph, or from the history of the base).
        let heads: BTreeSet<Id> = summary
            .files
            .iter()
            .flat_map(|(_, vs)| vs.iter().map(|v| v.id))
            .collect();
        let older: Vec<(Id, u64)> = self
            .state
            .versions
            .iter()
            .filter(|v| view.counts(v.at) && v.version.content.is_some())
            .filter(|v| summary.files.iter().any(|(f, _)| *f == v.file))
            .filter(|v| !heads.contains(&v.version.id))
            .map(|v| (v.version.id, v.version.time))
            .collect();
        let keep: BTreeSet<Hash> = retention
            .keep(&older, checkpoint.header.time)
            .into_iter()
            .map(|id| id.entry)
            .collect();
        let base_history = self.base().map(|b| b.history.clone()).unwrap_or_default();
        let history: Vec<Entry> = keep
            .iter()
            .filter_map(|h| {
                self.known
                    .get(h)
                    .map(|k| k.entry.clone())
                    .or_else(|| base_history.iter().find(|e| e.hash() == *h).cloned())
            })
            .collect();
        let after: Vec<Entry> = (0..n)
            .filter(|&j| self.state.past[j].has(c))
            .map(|j| self.known[&self.state.order[j]].entry.clone())
            .collect();
        let snapshot = Snapshot {
            genesis: self.genesis.clone(),
            checkpoint,
            summary,
            history,
        };
        let Ok(mut compacted) = Self::from_snapshot(snapshot) else {
            return false;
        };
        let count = after.len();
        if !matches!(compacted.insert(after), Ok(added) if added.len() == count) {
            return false;
        }
        std::mem::swap(&mut compacted.waiting, &mut self.waiting);
        std::mem::swap(&mut compacted.rebased, &mut self.rebased);
        *self = compacted;
        true
    }

    /// Takes a snapshot from another node: starts again from it, and keeps
    /// the entries here that come after it. Gives back the acts of `author`
    /// that were lost because they were made at the same time as the
    /// checkpoint, to be written again (see [`take_stale`](Self::take_stale)).
    ///
    /// # Errors
    ///
    /// When the snapshot is not good. Nothing changes then.
    pub fn adopt(
        &mut self,
        snapshot: Snapshot,
        author: PublicKey,
    ) -> Result<Vec<(u64, Vec<Act>)>, Error> {
        if snapshot.genesis.hash() != self.id {
            return Err(Error::invalid("a snapshot of another space".into()));
        }
        if self.has(snapshot.checkpoint.hash()) {
            return Ok(Vec::new());
        }
        if let Some(base) = self.base() {
            let current = &base.checkpoint.header;
            let incoming = &snapshot.checkpoint.header;
            if incoming.author == current.author && incoming.seq <= current.seq {
                return Err(Error::invalid(
                    "a snapshot is older than the current base".into(),
                ));
            }
        }
        let mut fresh = Self::from_snapshot(snapshot)?;
        let ours: Vec<Entry> = self.known.values().map(|k| k.entry.clone()).collect();
        let _ = fresh.insert(ours);
        // Our own entries that did not get in, or got in stale, were made at
        // the same time as the checkpoint: their acts go on top again.
        let mut out: Vec<(u64, Vec<Act>)> = self
            .known
            .iter()
            .filter(|(h, k)| {
                k.entry.header.author == author && !fresh.has(**h) && !fresh.base_has(&k.entry)
            })
            .map(|(_, k)| {
                let acts = k.acts.iter().filter_map(|a| fresh.again(a)).collect();
                (k.entry.header.time, acts)
            })
            .filter(|(_, acts): &(u64, Vec<Act>)| !acts.is_empty())
            .collect();
        out.extend(fresh.take_stale(author));
        *self = fresh;
        Ok(out)
    }

    /// `act` made ready to be written again on top: a change comes after the
    /// newest versions now. Checkpoints are not written again.
    fn again(&self, act: &Act) -> Option<Act> {
        match act {
            Act::Checkpoint { .. } | Act::Genesis { .. } => None,
            Act::Change {
                file,
                name,
                content,
                key,
                ..
            } => {
                let parents: Vec<Id> = self.file(*file)?.versions.iter().map(|v| v.id).collect();
                Some(Act::Change {
                    file: *file,
                    parents,
                    name: name.clone(),
                    content: *content,
                    key: key.clone(),
                })
            }
            other => Some(other.clone()),
        }
    }

    /// The newest time that an entry here says.
    pub fn latest_time(&self) -> u64 {
        self.known
            .values()
            .map(|k| k.entry.header.time)
            .max()
            .unwrap_or_default()
    }

    /// The epoch that new content is encrypted under: of the newest epochs,
    /// the one with the highest ID, so every node picks the same one.
    pub fn epoch(&self) -> Option<Hash> {
        self.now().latest_epochs().into_iter().max()
    }

    /// The keys sealed to any of `mine`, opened.
    pub fn keyring(&self, mine: &[&SecretKey]) -> Keyring {
        self.keyring_with(mine, &[])
    }

    /// The keys sealed to any of `mine`, or to any of the recipient keys
    /// `recipients` that this node still holds, opened.
    ///
    /// # Panics
    ///
    /// Never: the keys are cut in pieces of 32 bytes.
    pub fn keyring_with(&self, mine: &[&SecretKey], recipients: &[Recipient]) -> Keyring {
        let view = self.now();
        let mut ring = Keyring::default();
        for (at, x) in &self.state.sealed {
            if !view.counts(*at) || !mine.iter().any(|k| k.public() == x.to) {
                continue;
            }
            let keys = match &x.recipient {
                Some(pk) => recipients
                    .iter()
                    .filter(|r| r.public[..] == pk.0[..])
                    .find_map(|r| crypto::open_with_recipient(r, &self.id.0, &x.enc.0, &x.keys.0)),
                None => mine
                    .iter()
                    .find(|k| k.public() == x.to)
                    .and_then(|me| crypto::open_with(me, &self.id.0, &x.enc.0, &x.keys.0)),
            };
            let Some(keys) = keys else {
                continue;
            };
            for (r, key) in x.refs.iter().zip(keys.as_chunks::<32>().0) {
                let key: Key = *key;
                match r {
                    KeyRef::Root(e) => {
                        ring.roots.insert(*e, key);
                    }
                    KeyRef::Prefix(e, p) => {
                        ring.prefixes.insert((*e, p.clone()), key);
                    }
                    KeyRef::File(id, e) => {
                        ring.files.insert((*id, *e), key);
                    }
                }
            }
        }
        ring
    }

    /// The data keys that may open `version` of `file`, with `ring`: from
    /// its own wrap, from wraps of the file under newer epochs, and from a
    /// key of the file that was given. The caller tries them in order.
    pub fn data_keys(&self, ring: &Keyring, file: Id, version: &Version) -> Vec<Key> {
        let view = self.now();
        let mut keys = Vec::new();
        let wraps = version.key.iter().chain(
            self.state
                .rewraps
                .iter()
                .filter(|(at, f, _)| *f == file && view.counts(*at))
                .map(|(_, _, w)| w),
        );
        for w in wraps {
            if let Some(kek) = ring.prefix(w.epoch, &w.prefix)
                && let Some(key) = crypto::unwrap(&kek, w)
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        for ((_, _), key) in ring
            .files
            .range((file, Hash([0; 32]))..=(file, Hash([255; 32])))
        {
            if !keys.contains(key) {
                keys.push(*key);
            }
        }
        keys
    }

    /// The data key of `file` in `epoch`, with `ring`: from a wrap of the
    /// file under that epoch, or from that key given on its own.
    pub fn data_key_in(&self, ring: &Keyring, file: Id, epoch: Hash) -> Option<Key> {
        if let Some(key) = ring.files.get(&(file, epoch)) {
            return Some(*key);
        }
        let view = self.now();
        let own = self
            .state
            .versions
            .iter()
            .filter(|v| v.file == file && view.counts(v.at))
            .filter_map(|v| v.version.key.as_ref());
        let again = self
            .state
            .rewraps
            .iter()
            .filter(|(at, f, _)| *f == file && view.counts(*at))
            .map(|(_, _, w)| w);
        own.chain(again)
            .filter(|w| w.epoch == epoch)
            .find_map(|w| crypto::unwrap(&ring.prefix(epoch, &w.prefix)?, w))
    }

    /// The keys of the principal `who` acts for, as device keys: a key is
    /// itself; a group is the keys in it; a person is their nodes.
    fn devices(&self, view: &View<'_>, who: Principal) -> Vec<PublicKey> {
        match who {
            Principal::Key(k) => vec![k],
            Principal::Group(_) | Principal::Identity(_) => {
                let mut candidates: BTreeSet<PublicKey> =
                    self.known.values().map(|k| k.entry.header.author).collect();
                candidates.extend(
                    self.state
                        .vouches
                        .iter()
                        .filter_map(|(_, _, _, c)| c.as_ref().map(|c| c.node)),
                );
                for g in &self.state.gives {
                    if let Principal::Key(k) = g.given.to {
                        candidates.insert(k);
                    }
                }
                if let Principal::Key(k) = self.owner {
                    candidates.insert(k);
                }
                candidates
                    .into_iter()
                    .filter(|k| view.acts_as(Principal::Key(*k), who, 0))
                    .collect()
            }
        }
    }

    /// The keys that devices should have now and were not given yet: the
    /// root key of the epoch for the owner, and for each reader the key of
    /// what its grant is on.
    pub fn key_gaps(&self) -> Vec<(PublicKey, KeyRef)> {
        let Some(epoch) = self.epoch() else {
            return Vec::new();
        };
        let view = self.now();
        let mut want: BTreeSet<(PublicKey, KeyRef)> = BTreeSet::new();
        // The owner gets the root key: a key itself, or each device of an
        // identity or a group that owns the space.
        for device in self.devices(&view, self.owner) {
            want.insert((device, KeyRef::Root(epoch)));
        }
        for g in self.grants() {
            let given = &g.given;
            if !g.in_effect
                || given.effect != Effect::Allow
                || !given.permissions.contains(&Permission::FilesRead)
            {
                continue;
            }
            let r = match &given.on {
                Resource::Prefix(p) => KeyRef::Prefix(epoch, crypto::dir_of(p)),
                Resource::File(id) => {
                    // The epoch of the data key of the newest version.
                    let Some(e) = self.file(*id).and_then(|f| {
                        f.versions
                            .iter()
                            .max_by_key(|v| (v.time, v.id))
                            .and_then(|v| v.key.as_ref().map(|w| w.epoch))
                    }) else {
                        continue;
                    };
                    KeyRef::File(*id, e)
                }
                Resource::Group(_) => continue,
            };
            for device in self.devices(&view, given.to) {
                want.insert((device, r.clone()));
            }
        }
        let have: Vec<(PublicKey, &KeyRef)> = self
            .state
            .sealed
            .iter()
            .filter(|(at, _)| view.counts(*at))
            .filter(|(_, x)| {
                x.recipient.as_ref().map(|b| b.0.as_slice())
                    == view.recipient(x.to).as_ref().map(|k| &k[..])
            })
            .flat_map(|(_, x)| x.refs.iter().map(move |r| (x.to, r)))
            .collect();
        // Keys go only to keys that show a recipient key.
        want.into_iter()
            .filter(|(to, _)| view.recipient(*to).is_some())
            .filter(|(to, r)| !have.iter().any(|(t, h)| t == to && covers_key(h, r)))
            .collect()
    }

    /// How long before `now` (or the newest time here, if later) the epoch
    /// began.
    fn epoch_age(&self, epoch: Hash, now: u64) -> u64 {
        let started = self
            .state
            .epochs
            .iter()
            .filter(|(_, e)| *e == epoch)
            .map(|(at, _)| {
                if *at == BASE {
                    self.base().map_or(0, |b| b.checkpoint.header.time)
                } else {
                    self.known[&self.state.order[*at]].entry.header.time
                }
            })
            .min()
            .unwrap_or(0);
        self.latest_time().max(now).saturating_sub(started)
    }

    /// The recipient key that keys for `device` should be sealed to now, and
    /// when it was made.
    pub fn recipient_of(&self, device: PublicKey) -> Option<(Key, u64)> {
        self.now()
            .newest_recipients()
            .into_iter()
            .find(|(d, _, _)| *d == device)
            .map(|(_, k, t)| (k, t))
    }

    /// Says if the keys should change: there is no epoch yet, the epoch is
    /// older than [`EPOCH_MAX_AGE`] at `now`, or a device
    /// that was given a key of the newest epoch may no longer read what that
    /// key opens. This covers every way a device loses access: a take-back,
    /// a device taken away from an identity, a new signing key of an
    /// identity, and a member taken out of a group.
    pub fn needs_new_epoch(&self, now: u64) -> bool {
        let Some(epoch) = self.epoch() else {
            return true;
        };
        if self.epoch_age(epoch, now) >= EPOCH_MAX_AGE {
            return true;
        }
        let view = self.now();
        self.state
            .sealed
            .iter()
            .filter(|(at, _)| view.counts(*at))
            .any(|(_, x)| {
                let who = Principal::Key(x.to);
                x.refs.iter().any(|r| match r {
                    KeyRef::Root(e) => {
                        *e == epoch
                            && !view.holds(
                                who,
                                Permission::SpaceCheckpoint,
                                &Target::Prefix(String::new()),
                            )
                    }
                    KeyRef::Prefix(e, p) => {
                        *e == epoch
                            && !view.holds(who, Permission::FilesRead, &Target::Prefix(p.clone()))
                    }
                    KeyRef::File(id, e) => {
                        *e == epoch
                            && !view.holds(
                                who,
                                Permission::FilesRead,
                                &Target::File(*id, view.names(*id)),
                            )
                    }
                })
            })
    }

    /// Every file that is not deleted, with its newest versions.
    pub fn files(&self) -> Vec<File> {
        let view = self.now();
        self.state
            .by_file
            .keys()
            .copied()
            .filter_map(|id| Self::file_in(&view, id))
            .filter(|f| !f.deleted())
            .collect()
    }

    fn file_in(view: &View<'_>, id: Id) -> Option<File> {
        let mut versions: Vec<Version> = view.heads(id).into_iter().cloned().collect();
        if versions.is_empty() {
            return None;
        }
        versions.sort_by_key(|v| v.id);
        Some(File { id, versions })
    }

    /// One file by its ID, also when it is deleted.
    pub fn file(&self, id: Id) -> Option<File> {
        Self::file_in(&self.now(), id)
    }

    /// Every version of a file that is here, old and new, oldest first.
    pub fn history(&self, id: Id) -> Vec<Version> {
        let view = self.now();
        let mut versions: Vec<Version> = self
            .state
            .by_file
            .get(&id)
            .into_iter()
            .flatten()
            .map(|&i| &self.state.versions[i])
            .filter(|v| view.counts(v.at))
            .map(|v| v.version.clone())
            .collect();
        versions.sort_by_key(|v| (v.time, v.id));
        versions
    }

    /// The files that have `name` now. More than one when two files got the
    /// same name at the same time.
    pub fn find(&self, name: &str) -> Vec<File> {
        let view = self.now();
        self.state
            .by_name
            .get(name)
            .into_iter()
            .flatten()
            .filter_map(|&id| Self::file_in(&view, id))
            .filter(|f| {
                f.versions
                    .iter()
                    .any(|v| v.name == name && v.content.is_some())
            })
            .collect()
    }

    /// Says if `who` has `permission` on `on` now.
    pub fn may(&self, who: Principal, permission: Permission, on: &Resource) -> bool {
        let view = self.now();
        let target = view.target(on);
        view.holds(who, permission, &target)
    }

    /// Says if `who` is in `group` now (also through groups in groups, or
    /// as its owner).
    pub fn is_in(&self, who: Principal, group: Id) -> bool {
        self.now().acts_as(who, Principal::Group(group), 0)
    }

    /// Says if `key` is a node of the identity `did` now, as a `Vouch` in
    /// this space shows.
    pub fn is_node_of(&self, key: PublicKey, did: Hash) -> bool {
        self.now().vouched(key, did)
    }

    /// The identities that this space names: its owner, and the receivers of
    /// its grants.
    pub fn identities(&self) -> BTreeSet<Hash> {
        let mut out = BTreeSet::new();
        if let Principal::Identity(d) = self.owner {
            out.insert(d);
        }
        for g in &self.state.gives {
            if let Principal::Identity(d) = g.given.to {
                out.insert(d);
            }
        }
        out
    }

    /// Says if `who` may read `file` now.
    pub fn may_read(&self, who: Principal, file: Id) -> bool {
        self.may(who, Permission::FilesRead, &Resource::File(file))
    }

    /// The grants that are in place now (given and not taken back).
    pub fn grants(&self) -> Vec<Grant> {
        let view = self.now();
        view.grants()
            .into_iter()
            .map(|given| {
                let target = view.target(&given.on);
                let in_effect = view.holds(given.giver, Permission::AccessGive, &target);
                Grant {
                    given: given.clone(),
                    in_effect,
                }
            })
            .collect()
    }

    /// The entries that are in the graph but have no effect, and why.
    pub fn rejected(&self) -> &BTreeMap<Hash, String> {
        &self.state.why
    }

    /// The content of every version that has effect, newest or not.
    pub fn contents(&self) -> BTreeSet<Content> {
        let view = self.now();
        self.state
            .versions
            .iter()
            .filter(|v| view.counts(v.at))
            .filter_map(|v| v.version.content)
            .collect()
    }

    /// A hash of the state that has effect, to compare two nodes. It is the
    /// same before and after compaction.
    pub fn digest(&self) -> Hash {
        type Heads = Vec<(Id, String, Option<Content>)>;
        let files: Vec<(Id, Heads)> = self
            .files()
            .into_iter()
            .map(|f| {
                (
                    f.id,
                    f.versions
                        .into_iter()
                        .map(|v| (v.id, v.name, v.content))
                        .collect(),
                )
            })
            .collect();
        let grants: Vec<(Id, bool)> = self
            .grants()
            .into_iter()
            .map(|g| (g.given.id, g.in_effect))
            .collect();
        Hash::of_value(&(files, grants))
    }
}

/// Says if having the key `have` gives the key `want`.
fn covers_key(have: &KeyRef, want: &KeyRef) -> bool {
    match (have, want) {
        (KeyRef::Root(a), KeyRef::Root(b) | KeyRef::Prefix(b, _)) => a == b,
        (KeyRef::Prefix(a, p), KeyRef::Prefix(b, q)) => a == b && q.starts_with(p.as_str()),
        (KeyRef::File(a, e), KeyRef::File(b, f)) => a == b && e == f,
        _ => false,
    }
}

/// The keys a node has opened in one space.
#[derive(Clone, Default)]
pub struct Keyring {
    pub roots: BTreeMap<Hash, Key>,
    pub prefixes: BTreeMap<(Hash, String), Key>,
    pub files: BTreeMap<(Id, Hash), Key>,
}

impl std::fmt::Debug for Keyring {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keyring")
            .field("roots", &self.roots.len())
            .field("prefixes", &self.prefixes.len())
            .field("files", &self.files.len())
            .finish()
    }
}

impl Keyring {
    /// The key of a directory prefix in an epoch, if this ring has it or a
    /// key above it.
    pub fn prefix(&self, epoch: Hash, prefix: &str) -> Option<Key> {
        if let Some(root) = self.roots.get(&epoch) {
            return Some(crypto::prefix_key(root, prefix));
        }
        self.prefixes
            .iter()
            .filter(|((e, p), _)| *e == epoch && prefix.starts_with(p.as_str()))
            .find_map(|((_, p), key)| crypto::derive(key, p, prefix))
    }

    /// The key that `r` names, if this ring has it or can work it out.
    pub fn get(&self, r: &KeyRef) -> Option<Key> {
        match r {
            KeyRef::Root(e) => self.roots.get(e).copied(),
            KeyRef::Prefix(e, p) => self.prefix(*e, p),
            KeyRef::File(id, e) => self.files.get(&(*id, *e)).copied(),
        }
    }
}

/// The version that `act` makes, if it makes one.
fn version_of(entry: &Entry, index: usize, act: &Act, at: usize) -> Option<Ver> {
    let id = Id {
        entry: entry.hash(),
        index: index as u32,
    };
    let (file, name, content, parents, key) = match act {
        Act::Create { name, content, key } => (id, name, Some(*content), Vec::new(), key.clone()),
        Act::Change {
            file,
            parents,
            name,
            content,
            key,
        } => (*file, name, *content, parents.clone(), key.clone()),
        _ => return None,
    };
    Some(Ver {
        at,
        file,
        old: false,
        version: Version {
            id,
            name: name.clone(),
            content,
            author: entry.header.author,
            time: entry.header.time,
            parents,
            key,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkpoint_made_before_identity_grants_still_loads() {
        #[derive(Serialize)]
        struct OldSummary {
            files: [(); 0],
            grants: [(); 0],
            groups: [(); 0],
            logs: Vec<(PublicKey, Hash, u64)>,
        }

        let owner = SecretKey::from_seed(&[7; 32]);
        let (mut space, _) = Space::create(Principal::Key(owner.public()), &owner, 1, 2);
        let time = 3;
        let retention = Retention::default();

        let old = OldSummary {
            files: [],
            grants: [],
            groups: [],
            logs: vec![(owner.public(), space.id(), 0)],
        };
        let bytes = fileos_core::encode(&old);
        let state = Hash::of(&bytes);
        let decoded: Summary = fileos_core::decode(&bytes).unwrap();
        assert_eq!(decoded.logs, old.logs);
        space
            .write(&owner, time, vec![Act::Checkpoint { state, retention }])
            .unwrap();
        assert!(space.compact());
        let snapshot = space.base().unwrap();
        let restored = Space::from_snapshot(snapshot.clone()).unwrap();
        assert_eq!(restored.digest(), space.digest());

        let mut changed = snapshot.clone();
        changed.summary.epochs.push(Hash::of(b"another epoch"));
        assert!(!changed.summary.matches_hash(state));
    }
}
