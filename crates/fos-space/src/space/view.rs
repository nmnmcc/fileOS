// SPDX-License-Identifier: AGPL-3.0-only

//! Looks up files and grants at one point in the causal graph.

use std::collections::{BTreeMap, BTreeSet};

use fos_core::{Bytes, Error, Hash, PublicKey};
use fos_id::{NodeCert, Op as DidOp};

use crate::entry::{Act, Effect, Entry, Id, Permission, Principal, Resource, Retention, good_name};

use super::{BASE, Given, Space, State, Summary, Ver, Version};

/// What a permission is checked against.
#[derive(Clone, Debug)]
pub(super) enum Target {
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

/// A point in the graph to check at, and which entries count there.
pub(super) struct View<'s> {
    pub(super) space: &'s Space,
    pub(super) state: &'s State,
    /// The entry being checked; `None` means after every entry.
    pub(super) at: Option<usize>,
    /// Which entries have effect, as far as they are known.
    pub(super) valid: &'s [bool],
    /// In step 3: the results of step 1, for takes that are concurrent.
    pub(super) causal: Option<&'s [bool]>,
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
    pub(super) fn counts(&self, j: usize) -> bool {
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
    fn versions(&self, file: Id) -> impl Iterator<Item = &Ver> {
        self.state
            .by_file
            .get(&file)
            .into_iter()
            .flatten()
            .map(|&i| &self.state.versions[i])
            .filter(|v| !v.old && self.counts(v.at))
    }

    /// The newest versions of a file at this point.
    pub(super) fn heads(&self, file: Id) -> Vec<&Version> {
        let here = self.versions(file).map(|v| &v.version).collect::<Vec<_>>();
        if here.len() <= 1 {
            return here;
        }
        let parents = here
            .iter()
            .flat_map(|version| &version.parents)
            .copied()
            .collect::<BTreeSet<_>>();
        here.into_iter()
            .filter(|v| !parents.contains(&v.id))
            .collect()
    }

    fn names(&self, file: Id) -> Vec<String> {
        let mut names: Vec<String> = self.heads(file).iter().map(|v| v.name.clone()).collect();
        names.sort();
        names.dedup();
        names
    }

    pub(super) fn target(&self, on: &Resource) -> Target {
        match on {
            Resource::Prefix(p) => Target::Prefix(p.clone()),
            Resource::File(id) => Target::File(*id, self.names(*id)),
            Resource::Group(id) => Target::Group(*id),
        }
    }

    /// Says if `who` has `permission` on `target` at this point.
    pub(super) fn holds(&self, who: Principal, permission: Permission, target: &Target) -> bool {
        self.holds_inner(who, permission, target, 0)
    }

    /// Says if `who` acts as `as_`: it is `as_`, or `as_` is a group that
    /// `who` is in (also through groups in groups), or that `who` manages.
    pub(super) fn acts_as(&self, who: Principal, as_: Principal, depth: u32) -> bool {
        who == as_
            || match as_ {
                Principal::Group(g) => {
                    self.holds_inner(who, Permission::Member, &Target::Group(g), depth + 1)
                }
                Principal::Identity(did) => match who {
                    Principal::Key(key) => self.vouched(key, did),
                    _ => false,
                },
                Principal::Key(_) => false,
            }
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
        fos_id::resolve(&fos_id::did_of(did), &ops)
            .ok()?
            .state?
            .signing_key()
    }

    /// Says if `key` is a node of `did` at this point: a `Vouch` that counts
    /// has a certificate for it, signed by the signing key of now. A new
    /// signing key ends every certificate of the old one.
    pub(super) fn vouched(&self, key: PublicKey, did: Hash) -> bool {
        let Some(signing) = self.signing_key(did, &[]) else {
            return false;
        };
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
    pub(super) fn grants(&self) -> Vec<&Given> {
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
    pub(super) fn summary(&self, time: u64, retention: &Retention) -> Summary {
        let ids: BTreeSet<Id> = self
            .state
            .versions
            .iter()
            .filter(|v| self.counts(v.at))
            .map(|v| v.file)
            .collect();
        let files = ids
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
        Summary {
            files,
            grants,
            groups,
            identities,
            logs: logs.into_iter().map(|(k, (h, s))| (k, h, s)).collect(),
        }
    }

    /// Says why the acts of `entry` are not allowed at this point, if they
    /// are not.
    pub(super) fn check(&self, entry: &Entry, acts: &[Act], genesis: bool) -> Result<(), Error> {
        let author = entry.header.author;
        let who = Principal::Key(author);
        let need = |permission: Permission, target: &Target| {
            if self.holds(who, permission, target) {
                Ok(())
            } else {
                Err(Error::denied(format!(
                    "{author} does not have {permission:?} on {target:?}"
                )))
            }
        };
        if acts.is_empty() {
            return Err(Error::invalid("an entry with no acts".into()));
        }
        for act in acts {
            match act {
                Act::Genesis { .. } => {
                    if !genesis || acts.len() != 1 {
                        return Err(Error::invalid(
                            "a genesis act that is not the first entry".into(),
                        ));
                    }
                }
                Act::Create { name, .. } => {
                    if !good_name(name) {
                        return Err(Error::invalid(format!("{name:?} is not a good name")));
                    }
                    need(Permission::FilesCreate, &Target::Name(name.clone()))?;
                }
                Act::Change {
                    file,
                    parents,
                    name,
                    content,
                } => {
                    if !good_name(name) {
                        return Err(Error::invalid(format!("{name:?} is not a good name")));
                    }
                    let versions: Vec<&Ver> = self
                        .versions(*file)
                        .filter(|v| parents.contains(&v.version.id))
                        .collect();
                    if parents.is_empty() || versions.len() != parents.len() {
                        return Err(Error::invalid(
                            "a change after versions that are not there".into(),
                        ));
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
                        return Err(Error::invalid("a grant on a file that is not there".into()));
                    }
                    if let Target::Group(g) = &target
                        && self.group_owner(*g).is_none()
                    {
                        return Err(Error::invalid(
                            "a grant on a group that is not there".into(),
                        ));
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
                        return Err(Error::invalid(
                            "taking back a grant that is not there".into(),
                        ));
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
                        return Err(Error::invalid(
                            "a group owned by a group that is not there".into(),
                        ));
                    }
                    if !self.acts_as(who, *owner, 0) {
                        return Err(Error::denied(format!(
                            "{author} does not act as the owner {owner:?}"
                        )));
                    }
                }
                Act::Vouch { ops, cert } => {
                    if ops.len() > 64 {
                        return Err(Error::invalid(
                            "a vouch with too many DID operations".into(),
                        ));
                    }
                    let ops: Vec<DidOp> = ops
                        .iter()
                        .map(|o| DidOp::from_bytes(&o.0))
                        .collect::<Result<_, _>>()?;
                    let cert = NodeCert::from_bytes(&cert.0)?;
                    let did = fos_id::did_hash(&cert.did)?;
                    let signing = self.signing_key(did, &ops).ok_or_else(|| {
                        Error::invalid("a vouch whose DID log has no signing key".into())
                    })?;
                    if !cert.verify(signing) {
                        return Err(Error::denied(
                            "a vouch whose certificate is not signed by the signing key".into(),
                        ));
                    }
                }
                Act::Checkpoint { state, retention } => {
                    if acts.len() != 1 {
                        return Err(Error::invalid("a checkpoint with other acts".into()));
                    }
                    need(Permission::SpaceCheckpoint, &Target::Prefix(String::new()))?;
                    if !self
                        .summary(entry.header.time, retention)
                        .matches_hash(*state)
                    {
                        return Err(Error::invalid("a checkpoint whose state is wrong".into()));
                    }
                }
            }
        }
        Ok(())
    }
}
