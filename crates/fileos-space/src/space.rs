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
//! in its past. A node can then [`compact`](Space::compact): it keeps the
//! state at the checkpoint as a **base** (a [`Snapshot`]) and drops the
//! entries before it. New nodes start from the snapshot, and check it
//! against the signed hash.

use std::collections::{BTreeMap, BTreeSet};

use fileos_core::{Bytes, Error, Hash, PublicKey, SecretKey};
use fileos_id::{NodeCert, NodeRevocation, Op as DidOp};
use serde::{Deserialize, Serialize};

use crate::crypto::{self, Key, Wrapped};
use crate::entry::{
    Act, Content, Effect, Entry, Id, KeyRef, Permission, Principal, Resource, Retention, Sealed,
    good_name,
};

/// The place of what came from the base, before every entry.
const BASE: usize = usize::MAX;

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

/// The state that a checkpoint signs: the newest versions of every file,
/// the grants in place, and the last entry of each log.
#[derive(Clone, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
pub struct Summary {
    pub files: Vec<(Id, Vec<Version>)>,
    pub grants: Vec<Given>,
    /// Every group, with its owner.
    pub groups: Vec<(Id, Principal)>,
    /// For each identity: the operations of its DID log, and the node
    /// certificates that are still good.
    pub identities: Vec<(Hash, Vec<Bytes>, Vec<Bytes>)>,
    /// Node revocations that are good.
    pub revocations: Vec<Bytes>,
    /// For each author: its last entry and the place of it.
    pub logs: Vec<(PublicKey, Hash, u64)>,
    /// The newest epochs.
    pub epochs: Vec<Hash>,
    /// The sealed keys that the newest versions and the newest epochs still
    /// need. Keys that nothing needs are left out, so content that is gone
    /// cannot be opened again (crypto-shredding).
    pub sealed: Vec<Sealed>,
    /// The data keys of files, wrapped again under the newest epochs.
    pub rewrap: Vec<(Id, Wrapped)>,
}

impl Summary {
    pub fn hash(&self) -> Hash {
        Hash::of_value(self)
    }
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

/// A set of bits, one for each entry in causal order.
#[derive(Clone, Debug, Default)]
struct Bits(Vec<u64>);

impl Bits {
    fn with_len(n: usize) -> Self {
        Self(vec![0; n.div_ceil(64)])
    }

    fn set(&mut self, i: usize) {
        self.0[i / 64] |= 1 << (i % 64);
    }

    fn has(&self, i: usize) -> bool {
        self.0.get(i / 64).is_some_and(|w| w & (1 << (i % 64)) != 0)
    }

    fn add(&mut self, other: &Self) {
        for (a, b) in self.0.iter_mut().zip(&other.0) {
            *a |= b;
        }
    }
}

/// An entry that is in the graph.
#[derive(Clone, Debug)]
struct Known {
    entry: Entry,
    acts: Vec<Act>,
}

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
    past: Vec<Bits>,
    versions: Vec<Ver>,
    gives: Vec<Giv>,
    /// (place of the entry, the grant taken back)
    takes: Vec<(usize, Id)>,
    /// (place of the entry, the ID of a new epoch)
    epochs: Vec<(usize, Hash)>,
    /// (place of the entry, keys sealed to a device)
    sealed: Vec<(usize, Sealed)>,
    /// (place of the entry, a file, its data key wrapped again)
    rewraps: Vec<(usize, Id, Wrapped)>,
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

/// What a permission is checked against.
#[derive(Clone, Debug)]
enum Target {
    /// A name for a new file.
    Name(String),
    /// A file, with its names at that point.
    File(Id, Vec<String>),
    /// Everything under a prefix.
    Prefix(String),
    /// A group: who is in it, and who manages it.
    Group(Id),
}

fn covers(on: &Resource, target: &Target) -> bool {
    match (on, target) {
        (Resource::Prefix(p), Target::Name(n) | Target::Prefix(n)) => n.starts_with(p.as_str()),
        (Resource::Prefix(p), Target::File(_, names)) => {
            !names.is_empty() && names.iter().all(|n| n.starts_with(p.as_str()))
        }
        (Resource::File(a), Target::File(b, _)) | (Resource::Group(a), Target::Group(b)) => a == b,
        _ => false,
    }
}

/// The base of a compacted space.
#[derive(Clone, Debug)]
struct Base {
    snapshot: Snapshot,
    /// Hashes that entries may come after although they are not here: the
    /// past of the checkpoint, as far as new entries name it.
    gone: BTreeSet<Hash>,
}

/// A point in the graph to check at, and which entries count there.
struct View<'s> {
    space: &'s Space,
    state: &'s State,
    /// The entry being checked; `None` means after every entry.
    at: Option<usize>,
    /// Which entries have effect, as far as they are known.
    valid: &'s [bool],
    /// In step 3: the results of step 1, for takes that are concurrent.
    causal: Option<&'s [bool]>,
}

impl View<'_> {
    /// Says if entry `j` is in the past of the point.
    fn before(&self, j: usize) -> bool {
        j == BASE
            || match self.at {
                None => true,
                Some(i) => self.state.past[i].has(j),
            }
    }

    /// Says if entry `j` counts at this point.
    fn counts(&self, j: usize) -> bool {
        j == BASE || (self.valid[j] && self.before(j))
    }

    /// Says if `grant` is taken back at this point. A take-back that was
    /// valid in its causal past always counts, also when strong removal
    /// undid its author's other acts: taking away access is always safe. So
    /// two managers who take each other's access at the same time both lose
    /// it. In step 3, take-backs that are concurrent with the point count
    /// too. Take-backs made at the same time as a checkpoint do not count.
    fn taken(&self, grant: Id) -> bool {
        let took = self.causal.unwrap_or(self.valid);
        self.state.takes.iter().any(|&(k, g)| {
            g == grant
                && took[k]
                && !self.state.stale.get(k).copied().unwrap_or(false)
                && (self.before(k)
                    || match (self.at, self.causal) {
                        (Some(i), Some(_)) => k != i && !self.state.past[k].has(i),
                        _ => false,
                    })
        })
    }

    /// The versions of a file that count at this point, without history
    /// from the base.
    fn versions(&self, file: Id) -> Vec<&Ver> {
        self.state
            .versions
            .iter()
            .filter(|v| v.file == file && !v.old && self.counts(v.at))
            .collect()
    }

    /// The newest versions of a file at this point.
    fn heads(&self, file: Id) -> Vec<&Version> {
        let here = self.versions(file);
        here.iter()
            .filter(|v| {
                !here
                    .iter()
                    .any(|w| w.version.parents.contains(&v.version.id))
            })
            .map(|v| &v.version)
            .collect()
    }

    fn names(&self, file: Id) -> Vec<String> {
        let mut names: Vec<String> = self.heads(file).iter().map(|v| v.name.clone()).collect();
        names.sort();
        names.dedup();
        names
    }

    fn target(&self, on: &Resource) -> Target {
        match on {
            Resource::Prefix(p) => Target::Prefix(p.clone()),
            Resource::File(id) => Target::File(*id, self.names(*id)),
            Resource::Group(id) => Target::Group(*id),
        }
    }

    /// Says if `who` has `permission` on `target` at this point.
    fn holds(&self, who: Principal, permission: Permission, target: &Target) -> bool {
        self.holds_inner(who, permission, target, 0)
    }

    /// Says if `who` acts as `as_`: it is `as_`, or `as_` is a group that
    /// `who` is in (also through groups in groups), or that `who` manages.
    fn acts_as(&self, who: Principal, as_: Principal, depth: u32) -> bool {
        who == as_
            || match as_ {
                Principal::Group(g) => {
                    self.holds_inner(who, Permission::Member, &Target::Group(g), depth + 1)
                }
                Principal::Identity(d) => match who {
                    Principal::Key(k) => self.vouched(k, d),
                    _ => false,
                },
                Principal::Key(_) => false,
            }
    }

    /// Says if a revocation that counts, signed by `signing`, ends `key` as a
    /// node of `did`.
    fn revoked(&self, key: PublicKey, did: Hash, signing: PublicKey) -> bool {
        self.state
            .unvouches
            .iter()
            .any(|(at, d, r)| *d == did && r.node == key && self.counts(*at) && r.verify(signing))
    }

    /// The operations of the DID log of `did` that count at this point.
    fn did_ops(&self, did: Hash) -> Vec<DidOp> {
        let mut ops: Vec<DidOp> = Vec::new();
        for (at, d, more, _) in &self.state.vouches {
            if *d == did && self.counts(*at) {
                for op in more {
                    if !ops.contains(op) {
                        ops.push(op.clone());
                    }
                }
            }
        }
        ops
    }

    /// The signing key of `did` now, from the DID operations that count.
    fn signing_key(&self, did: Hash, more: &[DidOp]) -> Option<PublicKey> {
        let mut ops = self.did_ops(did);
        ops.extend(more.iter().cloned());
        fileos_id::resolve(&fileos_id::did_of(did), &ops)
            .ok()?
            .state?
            .signing_key()
    }

    /// Says if `key` is a node of `did` at this point: a `Vouch` that counts
    /// has a certificate for it, signed by the signing key of now. A new
    /// signing key ends every certificate of the old one.
    fn vouched(&self, key: PublicKey, did: Hash) -> bool {
        let Some(signing) = self.signing_key(did, &[]) else {
            return false;
        };
        if self.revoked(key, did, signing) {
            return false;
        }
        self.state.vouches.iter().any(|(at, d, _, cert)| {
            *d == did
                && self.counts(*at)
                && cert
                    .as_ref()
                    .is_some_and(|c| c.node == key && c.verify(signing))
        })
    }

    /// Chains of grants and groups longer than this give nothing. So a cycle
    /// (A gives to B, B gives to A) gives nothing unless it comes from the
    /// owner in fewer steps.
    const DEPTH: u32 = 12;

    fn holds_inner(
        &self,
        who: Principal,
        permission: Permission,
        target: &Target,
        depth: u32,
    ) -> bool {
        if depth > Self::DEPTH {
            return false;
        }
        if self.acts_as(who, self.space.owner, depth) {
            return true;
        }
        // The owner of a group has every permission on it.
        if let Target::Group(g) = target
            && let Some(owner) = self.group_owner(*g)
            && self.acts_as(who, owner, depth)
        {
            return true;
        }
        let mut allowed = false;
        let mut denied = false;
        for g in &self.state.gives {
            let given = &g.given;
            if !given.permissions.contains(&permission)
                || !covers(&given.on, target)
                || !self.counts(g.at)
                || !self.acts_as(who, given.to, depth)
                || self.taken(given.id)
            {
                continue;
            }
            // A grant gives only what its giver still has.
            let giver = self.holds_inner(given.giver, Permission::AccessGive, target, depth + 1)
                && (given.effect == Effect::Deny
                    || self.holds_inner(given.giver, permission, target, depth + 1));
            if giver {
                match given.effect {
                    Effect::Allow => allowed = true,
                    Effect::Deny => denied = true,
                }
            }
        }
        allowed && !denied
    }

    /// The owner of a group, if the group is there at this point.
    fn group_owner(&self, group: Id) -> Option<Principal> {
        self.state
            .groups
            .iter()
            .find(|(at, id, _)| *id == group && self.counts(*at))
            .map(|(_, _, owner)| *owner)
    }

    /// The grants given and not taken back at this point.
    fn grants(&self) -> Vec<&Given> {
        self.state
            .gives
            .iter()
            .filter(|g| self.counts(g.at) && !self.taken(g.given.id))
            .map(|g| &g.given)
            .collect()
    }

    /// The state at this point, as a checkpoint made at `time` with
    /// `retention` signs it. Files that were deleted longer ago than the
    /// retention keeps deleted files are left out.
    fn summary(&self, time: u64, retention: &Retention) -> Summary {
        let ids: BTreeSet<Id> = self
            .state
            .versions
            .iter()
            .filter(|v| self.counts(v.at))
            .map(|v| v.file)
            .collect();
        let files: Vec<(Id, Vec<Version>)> = ids
            .into_iter()
            .filter_map(|id| {
                let mut heads: Vec<Version> = self.heads(id).into_iter().cloned().collect();
                heads.sort_by_key(|v| v.id);
                let gone = heads
                    .iter()
                    .all(|v| v.content.is_none() && !retention.keeps_deleted(v.time, time));
                (!gone).then_some((id, heads))
            })
            .collect();
        let mut grants: Vec<Given> = self.grants().into_iter().cloned().collect();
        grants.sort_by_key(|g| g.id);
        let mut logs: BTreeMap<PublicKey, (Hash, u64)> = self
            .space
            .base
            .as_ref()
            .map(|b| {
                b.snapshot
                    .summary
                    .logs
                    .iter()
                    .map(|(k, h, s)| (*k, (*h, *s)))
                    .collect()
            })
            .unwrap_or_default();
        for (j, h) in self.state.order.iter().enumerate() {
            if self.before(j) {
                let header = &self.space.known[h].entry.header;
                let last = logs.entry(header.author).or_insert((*h, header.seq));
                if (header.seq, *h) > (last.1, last.0) {
                    *last = (*h, header.seq);
                }
            }
        }
        let mut groups: Vec<(Id, Principal)> = self
            .state
            .groups
            .iter()
            .filter(|(at, _, _)| self.counts(*at))
            .map(|(_, id, owner)| (*id, *owner))
            .collect();
        groups.sort();
        let dids: BTreeSet<Hash> = self.state.vouches.iter().map(|v| v.1).collect();
        let identities = dids
            .into_iter()
            .filter_map(|did| {
                let ops = self.did_ops(did);
                let signing = self.signing_key(did, &[])?;
                let mut certs: Vec<Bytes> = self
                    .state
                    .vouches
                    .iter()
                    .filter(|(at, d, _, _)| *d == did && self.counts(*at))
                    .filter_map(|(_, _, _, c)| c.clone())
                    .filter(|c| c.verify(signing))
                    .map(|c| Bytes(c.to_bytes()))
                    .collect();
                certs.sort_by(|a, b| a.0.cmp(&b.0));
                certs.dedup();
                let mut ops: Vec<Bytes> = ops.iter().map(|o| Bytes(o.to_bytes())).collect();
                ops.sort_by(|a, b| a.0.cmp(&b.0));
                Some((did, ops, certs))
            })
            .collect();
        let mut revocations: Vec<Bytes> = self
            .state
            .unvouches
            .iter()
            .filter(|(at, d, r)| {
                self.counts(*at) && self.signing_key(*d, &[]).is_some_and(|s| r.verify(s))
            })
            .map(|(_, _, r)| Bytes(r.to_bytes()))
            .collect();
        revocations.sort_by(|a, b| a.0.cmp(&b.0));
        revocations.dedup();
        let epochs = self.latest_epochs();
        let kept: BTreeSet<Id> = files.iter().map(|(id, _)| *id).collect();
        let mut needed: BTreeSet<Hash> = epochs.iter().copied().collect();
        for (_, heads) in &files {
            needed.extend(heads.iter().filter_map(|v| v.key.as_ref().map(|w| w.epoch)));
        }
        let mut rewrap: Vec<(Id, Wrapped)> = self
            .state
            .rewraps
            .iter()
            .filter(|(at, file, w)| {
                self.counts(*at) && kept.contains(file) && epochs.contains(&w.epoch)
            })
            .map(|(_, file, w)| (*file, w.clone()))
            .collect();
        rewrap.sort_by(|a, b| {
            (a.0, &a.1.epoch, &a.1.sealed.0).cmp(&(b.0, &b.1.epoch, &b.1.sealed.0))
        });
        let mut sealed: Vec<Sealed> = self
            .state
            .sealed
            .iter()
            .filter(|(at, _)| self.counts(*at))
            .filter(|(_, x)| {
                x.refs.iter().any(|r| match r {
                    KeyRef::Root(e) | KeyRef::Prefix(e, _) => needed.contains(e),
                    KeyRef::File(id, _) => kept.contains(id),
                })
            })
            .map(|(_, x)| x.clone())
            .collect();
        sealed.sort_by(|a, b| (a.to, &a.refs, &a.keys.0).cmp(&(b.to, &b.refs, &b.keys.0)));
        sealed.dedup();
        Summary {
            files,
            grants,
            groups,
            identities,
            revocations,
            logs: logs.into_iter().map(|(k, (h, s))| (k, h, s)).collect(),
            epochs,
            sealed,
            rewrap,
        }
    }

    /// The newest epochs at this point: those that no other epoch comes
    /// after.
    fn latest_epochs(&self) -> Vec<Hash> {
        let here: Vec<&(usize, Hash)> = self
            .state
            .epochs
            .iter()
            .filter(|(at, _)| self.counts(*at))
            .collect();
        let mut latest: Vec<Hash> = here
            .iter()
            .filter(|(at, _)| {
                !here.iter().any(|(other, _)| {
                    other != at
                        && *other != BASE
                        && (*at == BASE || self.state.past[*other].has(*at))
                })
            })
            .map(|(_, e)| *e)
            .collect();
        latest.sort();
        latest.dedup();
        latest
    }

    /// Says why the acts of `entry` are not allowed at this point, if they
    /// are not.
    fn check(&self, entry: &Entry, acts: &[Act], genesis: bool) -> Result<(), String> {
        let author = entry.header.author;
        let who = Principal::Key(author);
        let need = |permission: Permission, target: &Target| {
            if self.holds(who, permission, target) {
                Ok(())
            } else {
                Err(format!(
                    "{author} does not have {permission:?} on {target:?}"
                ))
            }
        };
        if acts.is_empty() {
            return Err("an entry with no acts".into());
        }
        for act in acts {
            match act {
                Act::Genesis { .. } => {
                    if !genesis || acts.len() != 1 {
                        return Err("a genesis act that is not the first entry".into());
                    }
                }
                Act::Create { name, .. } => {
                    if !good_name(name) {
                        return Err(format!("{name:?} is not a good name"));
                    }
                    need(Permission::FilesCreate, &Target::Name(name.clone()))?;
                }
                Act::Change {
                    file,
                    parents,
                    name,
                    content,
                    ..
                } => {
                    if !good_name(name) {
                        return Err(format!("{name:?} is not a good name"));
                    }
                    let versions: Vec<&Ver> = self
                        .versions(*file)
                        .into_iter()
                        .filter(|v| parents.contains(&v.version.id))
                        .collect();
                    if parents.is_empty() || versions.len() != parents.len() {
                        return Err("a change after versions that are not there".into());
                    }
                    let mut names: Vec<String> =
                        versions.iter().map(|v| v.version.name.clone()).collect();
                    names.sort();
                    names.dedup();
                    let old = Target::File(*file, names.clone());
                    if content.is_none() {
                        need(Permission::FilesDelete, &old)?;
                    } else {
                        need(Permission::FilesChange, &old)?;
                        if !names.contains(name) {
                            need(Permission::FilesCreate, &Target::Name(name.clone()))?;
                        }
                    }
                }
                Act::Give {
                    effect,
                    permissions,
                    on,
                    ..
                } => {
                    let target = self.target(on);
                    if let Target::File(_, names) = &target
                        && names.is_empty()
                    {
                        return Err("a grant on a file that is not there".into());
                    }
                    if let Target::Group(g) = &target
                        && self.group_owner(*g).is_none()
                    {
                        return Err("a grant on a group that is not there".into());
                    }
                    need(Permission::AccessGive, &target)?;
                    if *effect == Effect::Allow {
                        // Give on only what you have: the same or less.
                        for permission in permissions {
                            need(*permission, &target)?;
                        }
                    }
                }
                Act::Take { grant } => {
                    let Some(g) = self
                        .state
                        .gives
                        .iter()
                        .find(|g| g.given.id == *grant && self.counts(g.at))
                    else {
                        return Err("taking back a grant that is not there".into());
                    };
                    // The giver can always take back what it gave.
                    if g.given.giver != who {
                        need(Permission::AccessTake, &self.target(&g.given.on))?;
                    }
                }
                Act::Group { owner } => {
                    // You make groups that you own, or that a group you are
                    // in owns. Owners are fixed, and must be there first, so
                    // owners never make a cycle.
                    if let Principal::Group(g) = owner
                        && self.group_owner(*g).is_none()
                    {
                        return Err("a group owned by a group that is not there".into());
                    }
                    if !self.acts_as(who, *owner, 0) {
                        return Err(format!("{author} does not act as the owner {owner:?}"));
                    }
                }
                Act::Vouch { ops, cert } => {
                    if ops.len() > 64 {
                        return Err("a vouch with too many DID operations".into());
                    }
                    let ops: Vec<DidOp> = ops
                        .iter()
                        .map(|o| DidOp::from_bytes(&o.0))
                        .collect::<Result<_, _>>()
                        .map_err(|e| e.0)?;
                    let cert = NodeCert::from_bytes(&cert.0).map_err(|e| e.0)?;
                    let did = fileos_id::did_hash(&cert.did).map_err(|e| e.0)?;
                    let signing = self
                        .signing_key(did, &ops)
                        .ok_or("a vouch whose DID log has no signing key")?;
                    if !cert.verify(signing) {
                        return Err(
                            "a vouch whose certificate is not signed by the signing key".into()
                        );
                    }
                }
                Act::Unvouch { revocation } => {
                    let r = NodeRevocation::from_bytes(&revocation.0).map_err(|e| e.0)?;
                    let did = fileos_id::did_hash(&r.did).map_err(|e| e.0)?;
                    let signing = self
                        .signing_key(did, &[])
                        .ok_or("a revocation for an identity that is not shown here")?;
                    if !r.verify(signing) {
                        return Err("a revocation not signed by the signing key".into());
                    }
                }
                Act::Keys {
                    epoch,
                    sealed,
                    rewrap,
                } => {
                    let whole = Target::Prefix(String::new());
                    if epoch.is_some() {
                        need(Permission::SpaceCheckpoint, &whole)?;
                    } else if !rewrap.is_empty() {
                        return Err("wraps again with no new epoch".into());
                    }
                    for x in sealed {
                        for r in &x.refs {
                            let target = match r {
                                KeyRef::Root(_) => {
                                    need(Permission::SpaceCheckpoint, &whole)?;
                                    continue;
                                }
                                KeyRef::Prefix(_, p) => Target::Prefix(p.clone()),
                                KeyRef::File(id, _) => Target::File(*id, self.names(*id)),
                            };
                            // Who may give the access may give the key; and a
                            // reader may give a key to a device that may read
                            // it too, such as another device of the same
                            // person, which leaks nothing.
                            let to = Principal::Key(x.to);
                            let shared = self.holds(who, Permission::FilesRead, &target)
                                && self.holds(to, Permission::FilesRead, &target);
                            if !shared {
                                need(Permission::AccessGive, &target)?;
                            }
                        }
                    }
                }
                Act::Checkpoint { state, retention } => {
                    if acts.len() != 1 {
                        return Err("a checkpoint with other acts".into());
                    }
                    need(Permission::SpaceCheckpoint, &Target::Prefix(String::new()))?;
                    if self.summary(entry.header.time, retention).hash() != *state {
                        return Err("a checkpoint whose state is wrong".into());
                    }
                }
            }
        }
        Ok(())
    }
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
            _ => Err(Error("not the first entry of a space".into())),
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

    /// Starts a space from a snapshot at a checkpoint.
    ///
    /// # Errors
    ///
    /// When the checkpoint is not signed well, does not sign this summary,
    /// or its author may not make checkpoints.
    pub fn from_snapshot(snapshot: Snapshot) -> Result<Self, Error> {
        let owner = Self::owner_of(&snapshot.genesis)?;
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
            return Err(Error("a checkpoint of another space".into()));
        }
        let acts = checkpoint.open()?;
        let [Act::Checkpoint { state, .. }] = acts.as_slice() else {
            return Err(Error("not a checkpoint".into()));
        };
        if snapshot.summary.hash() != *state {
            return Err(Error(
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
            return Err(Error(format!(
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
        self.state
            .order
            .iter()
            .map(|h| &self.known[h].entry)
            .filter(|e| e.header.seq >= logs.get(&e.header.author).copied().unwrap_or(0))
            .cloned()
            .collect()
    }

    /// Every entry here, in causal order: the first entry, or the checkpoint
    /// of the base, and the entries after it.
    pub fn entries(&self) -> Vec<Entry> {
        self.missing(&BTreeMap::new())
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
        match first_error {
            Some(e) => Err(e),
            None => Ok(added),
        }
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
            return Err(Error(format!("an entry that is not of space {}", self.id)));
        }
        if (h.seq == 0) != h.prev.is_none() {
            return Err(Error("an entry whose place in its log is wrong".into()));
        }
        if h.prev.is_none() && h.deps.is_empty() {
            return Err(Error("an entry that comes after nothing".into()));
        }
        if h.deps.len() > 256 {
            return Err(Error("an entry with too many dependencies".into()));
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
                    .ok_or_else(|| Error("an entry after one that is gone".into()))?,
            };
            if author != h.author || seq + 1 != h.seq {
                return Err(Error("an entry whose log is broken".into()));
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
        for i in 0..n {
            let mut past = Bits::with_len(n);
            for p in self.known[&state.order[i]].entry.header.parents() {
                if let Some(&j) = state.place.get(&p) {
                    past.set(j);
                    let pj = state.past[j].clone();
                    past.add(&pj);
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
                    state.why.insert(state.order[i], why);
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
    ) -> Result<(), String> {
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
        self.now().check(&entry, &acts, false).map_err(Error)?;
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
        if self.set_base(snapshot).is_err() {
            return false;
        }
        let _ = self.insert(after);
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
        if self.has(snapshot.checkpoint.hash()) {
            return Ok(Vec::new());
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
    ///
    /// # Panics
    ///
    /// Never: the keys are cut in pieces of 32 bytes.
    pub fn keyring(&self, mine: &[&SecretKey]) -> Keyring {
        let view = self.now();
        let mut ring = Keyring::default();
        for (at, x) in &self.state.sealed {
            if !view.counts(*at) {
                continue;
            }
            let Some(me) = mine.iter().find(|k| k.public() == x.to) else {
                continue;
            };
            let Some(keys) = crypto::open_with(me, &self.id.0, &x.enc.0, &x.keys.0) else {
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
        if let Principal::Key(k) = self.owner {
            want.insert((k, KeyRef::Root(epoch)));
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
            .flat_map(|(_, x)| x.refs.iter().map(move |r| (x.to, r)))
            .collect();
        want.into_iter()
            .filter(|(to, r)| !have.iter().any(|(t, h)| t == to && covers_key(h, r)))
            .collect()
    }

    /// Says if the keys should change: there is no epoch yet, or a device
    /// that was given a key of the newest epoch may no longer read what that
    /// key opens. This covers every way a device loses access: a take-back,
    /// a device taken away from an identity, a new signing key of an
    /// identity, and a member taken out of a group.
    pub fn needs_new_epoch(&self) -> bool {
        let Some(epoch) = self.epoch() else {
            return true;
        };
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
        let ids: BTreeSet<Id> = self.state.versions.iter().map(|v| v.file).collect();
        ids.into_iter()
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
            .versions
            .iter()
            .filter(|v| v.file == id && view.counts(v.at))
            .map(|v| v.version.clone())
            .collect();
        versions.sort_by_key(|v| (v.time, v.id));
        versions
    }

    /// The files that have `name` now. More than one when two files got the
    /// same name at the same time.
    pub fn find(&self, name: &str) -> Vec<File> {
        self.files()
            .into_iter()
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
#[derive(Clone, Debug, Default)]
pub struct Keyring {
    pub roots: BTreeMap<Hash, Key>,
    pub prefixes: BTreeMap<(Hash, String), Key>,
    pub files: BTreeMap<(Id, Hash), Key>,
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
