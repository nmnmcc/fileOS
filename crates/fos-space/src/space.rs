// SPDX-License-Identifier: AGPL-3.0-only

//! One space, and the state that comes from its entries.
//!
//! The state is worked out again from all the entries after every change, in
//! two passes over the entries in causal order:
//!
//! 1. **Causal check.** An entry is valid when its author had the
//!    permissions for every act in it, in the state of its causal past.
//! 2. **Strong removal.** The same check again, but a grant also counts as
//!    taken back when a `Take` of it that was valid in pass 1 is
//!    *concurrent* with the entry. So what a person did at the same time as
//!    their access was taken away has no effect, and nor does what depends on
//!    it. Two managers who take each other's access at the same time both
//!    lose it. The owner can never lose access.
//!
//! The result depends only on the set of entries, never on the order they
//! came in, so every node with the same entries gets the same state. Doing
//! all the work again each time is simple and slow; checkpoints (a later
//! step) will keep it small.

use std::collections::{BTreeMap, BTreeSet};

use fos_core::{Error, Hash, PublicKey, SecretKey};

use crate::entry::{Act, Content, Effect, Entry, Id, Permission, Principal, Resource, good_name};

/// One version of a file.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Version {
    pub id: Id,
    pub name: String,
    /// `None` when this version deletes the file.
    pub content: Option<Content>,
    pub author: PublicKey,
    pub parents: Vec<Id>,
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

/// A grant that is in place.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Grant {
    pub id: Id,
    pub giver: Principal,
    pub to: Principal,
    pub effect: Effect,
    pub permissions: Vec<Permission>,
    pub on: Resource,
    /// False when the giver no longer has what it gave (taken back above it).
    pub in_effect: bool,
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
}

/// A grant, with the place of its entry in causal order.
#[derive(Clone, Debug)]
struct Giv {
    at: usize,
    id: Id,
    giver: Principal,
    to: Principal,
    effect: Effect,
    permissions: Vec<Permission>,
    on: Resource,
}

/// The state worked out from the entries.
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
    /// The result of pass 1.
    causal: Vec<bool>,
    /// The result of pass 2: the entries that have effect.
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
}

fn covers(on: &Resource, target: &Target) -> bool {
    match (on, target) {
        (Resource::Prefix(p), Target::Name(n) | Target::Prefix(n)) => n.starts_with(p.as_str()),
        (Resource::Prefix(p), Target::File(_, names)) => {
            !names.is_empty() && names.iter().all(|n| n.starts_with(p.as_str()))
        }
        (Resource::File(a), Target::File(b, _)) => a == b,
        _ => false,
    }
}

/// A point in the graph to check at, and which entries count there.
struct View<'s> {
    state: &'s State,
    owner: Principal,
    /// The entry being checked; `None` means after every entry.
    at: Option<usize>,
    /// Which entries have effect, as far as they are known.
    valid: &'s [bool],
    /// In pass 2: the results of pass 1, for takes that are concurrent.
    causal: Option<&'s [bool]>,
}

impl View<'_> {
    /// Says if entry `j` is in the past of the point.
    fn before(&self, j: usize) -> bool {
        match self.at {
            None => true,
            Some(i) => self.state.past[i].has(j),
        }
    }

    /// Says if entry `j` counts at this point.
    fn counts(&self, j: usize) -> bool {
        self.valid[j] && self.before(j)
    }

    /// Says if `grant` is taken back at this point. A take-back that was
    /// valid in its causal past always counts, also when strong removal
    /// undid its author's other acts: taking away access is always safe. So
    /// two managers who take each other's access at the same time both lose
    /// it. In pass 2, take-backs that are concurrent with the point count
    /// too.
    fn taken(&self, grant: Id) -> bool {
        let took = self.causal.unwrap_or(self.valid);
        self.state.takes.iter().any(|&(k, g)| {
            g == grant
                && took[k]
                && (self.before(k)
                    || match (self.at, self.causal) {
                        (Some(i), Some(_)) => k != i && !self.state.past[k].has(i),
                        _ => false,
                    })
        })
    }

    /// The newest versions of a file at this point.
    fn heads(&self, file: Id) -> Vec<&Version> {
        let here: Vec<&Ver> = self
            .state
            .versions
            .iter()
            .filter(|v| v.file == file && self.counts(v.at))
            .collect();
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
        }
    }

    /// Says if `who` has `permission` on `target` at this point.
    fn holds(&self, who: Principal, permission: Permission, target: &Target) -> bool {
        self.holds_inner(who, permission, target, &mut Vec::new())
    }

    fn holds_inner(
        &self,
        who: Principal,
        permission: Permission,
        target: &Target,
        seen: &mut Vec<Principal>,
    ) -> bool {
        if who == self.owner {
            return true;
        }
        if seen.contains(&who) {
            return false;
        }
        seen.push(who);
        let mut allowed = false;
        let mut denied = false;
        for g in &self.state.gives {
            if g.to != who
                || !g.permissions.contains(&permission)
                || !covers(&g.on, target)
                || !self.counts(g.at)
                || self.taken(g.id)
            {
                continue;
            }
            // A grant gives only what its giver still has.
            let giver = self.holds_inner(g.giver, Permission::AccessGive, target, seen)
                && (g.effect == Effect::Deny
                    || self.holds_inner(g.giver, permission, target, seen));
            if giver {
                match g.effect {
                    Effect::Allow => allowed = true,
                    Effect::Deny => denied = true,
                }
            }
        }
        seen.pop();
        allowed && !denied
    }

    /// Says why `acts` by `author` are not allowed at this point, if they
    /// are not.
    fn check(&self, author: PublicKey, acts: &[Act], genesis: bool) -> Result<(), String> {
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
                } => {
                    if !good_name(name) {
                        return Err(format!("{name:?} is not a good name"));
                    }
                    let versions: Vec<&Ver> = self
                        .state
                        .versions
                        .iter()
                        .filter(|v| v.file == *file && parents.contains(&v.version.id))
                        .filter(|v| self.counts(v.at))
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
                        .find(|g| g.id == *grant && self.counts(g.at))
                    else {
                        return Err("taking back a grant that is not there".into());
                    };
                    // The giver can always take back what it gave.
                    if g.giver != who {
                        need(Permission::AccessTake, &self.target(&g.on))?;
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
    known: BTreeMap<Hash, Known>,
    /// Entries that wait for entries they come after.
    waiting: BTreeMap<Hash, Entry>,
    state: State,
}

impl Space {
    /// Makes a new space owned by `owner`. `author` signs the first entry.
    /// `nonce` should be random.
    ///
    /// # Panics
    ///
    /// Never: the first entry it makes is always good.
    pub fn create(owner: Principal, author: &SecretKey, nonce: u64) -> (Self, Entry) {
        let entry = Entry::sign(
            None,
            0,
            None,
            Vec::new(),
            &[Act::Genesis { owner, nonce }],
            author,
        );
        let space = Self::from_genesis(&entry).expect("a genesis entry that was just made");
        (space, entry)
    }

    /// Starts a space from its first entry, as a node that follows it does.
    ///
    /// # Errors
    ///
    /// When the entry is not a good first entry.
    pub fn from_genesis(entry: &Entry) -> Result<Self, Error> {
        let acts = entry.open()?;
        let h = &entry.header;
        let owner = match acts.as_slice() {
            [Act::Genesis { owner, .. }]
                if h.space.is_none() && h.seq == 0 && h.prev.is_none() && h.deps.is_empty() =>
            {
                *owner
            }
            _ => return Err(Error("not the first entry of a space".into())),
        };
        let id = entry.hash();
        let mut space = Self {
            id,
            owner,
            known: BTreeMap::new(),
            waiting: BTreeMap::new(),
            state: State::default(),
        };
        space.known.insert(
            id,
            Known {
                entry: entry.clone(),
                acts,
            },
        );
        space.work_out();
        Ok(space)
    }

    /// The ID of the space: the hash of its first entry.
    pub fn id(&self) -> Hash {
        self.id
    }

    pub fn owner(&self) -> Principal {
        self.owner
    }

    /// The number of entries in the graph.
    pub fn len(&self) -> usize {
        self.known.len()
    }

    /// Never true: a space always has its first entry.
    pub fn is_empty(&self) -> bool {
        self.known.is_empty()
    }

    pub fn has(&self, entry: Hash) -> bool {
        self.known.contains_key(&entry)
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

    /// For each author, the number of entries of its log here.
    pub fn logs(&self) -> BTreeMap<PublicKey, u64> {
        let mut logs = BTreeMap::new();
        for k in self.known.values() {
            let n = logs.entry(k.entry.header.author).or_insert(0);
            *n = (*n).max(k.entry.header.seq + 1);
        }
        logs
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

    /// Every entry, in causal order.
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
            if self.known.contains_key(&hash) {
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
                .filter(|(_, e)| e.header.parents().all(|p| self.known.contains_key(&p)))
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
            let p = &self.known[&prev].entry.header;
            if p.author != h.author || p.seq + 1 != h.seq {
                return Err(Error("an entry whose log is broken".into()));
            }
        }
        entry.open()
    }

    /// Works out the state from all entries again.
    fn work_out(&mut self) {
        let mut state = State::default();
        // Causal order, with ties by hash (Kahn's algorithm).
        let mut children: BTreeMap<Hash, Vec<Hash>> = BTreeMap::new();
        let mut parents_left: BTreeMap<Hash, usize> = BTreeMap::new();
        for (h, k) in &self.known {
            let parents: BTreeSet<Hash> = k.entry.header.parents().collect();
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
                let j = state.place[&p];
                past.set(j);
                let pj = state.past[j].clone();
                past.add(&pj);
            }
            state.past.push(past);
        }
        // What each entry does.
        for (at, h) in state.order.iter().enumerate() {
            let k = &self.known[h];
            let author = k.entry.header.author;
            for (index, act) in k.acts.iter().enumerate() {
                let id = Id {
                    entry: *h,
                    index: index as u32,
                };
                match act {
                    Act::Create { name, content } => state.versions.push(Ver {
                        at,
                        file: id,
                        version: Version {
                            id,
                            name: name.clone(),
                            content: Some(*content),
                            author,
                            parents: Vec::new(),
                        },
                    }),
                    Act::Change {
                        file,
                        parents,
                        name,
                        content,
                    } => state.versions.push(Ver {
                        at,
                        file: *file,
                        version: Version {
                            id,
                            name: name.clone(),
                            content: *content,
                            author,
                            parents: parents.clone(),
                        },
                    }),
                    Act::Give {
                        to,
                        effect,
                        permissions,
                        on,
                    } => state.gives.push(Giv {
                        at,
                        id,
                        giver: Principal::Key(author),
                        to: *to,
                        effect: *effect,
                        permissions: permissions.clone(),
                        on: on.clone(),
                    }),
                    Act::Take { grant } => state.takes.push((at, *grant)),
                    Act::Genesis { .. } => {}
                }
            }
        }
        // Pass 1 and pass 2.
        let mut causal = vec![false; n];
        for i in 0..n {
            causal[i] = self.check_at(&state, i, &causal, None).is_ok();
        }
        let mut valid = vec![false; n];
        for i in 0..n {
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
            state,
            owner: self.owner,
            at: Some(i),
            valid,
            causal,
        };
        view.check(k.entry.header.author, &k.acts, h == self.id)
    }

    fn now(&self) -> View<'_> {
        View {
            state: &self.state,
            owner: self.owner,
            at: None,
            valid: &self.state.valid,
            causal: Some(&self.state.causal),
        }
    }

    /// Makes, signs and adds an entry by `author` after every entry here.
    ///
    /// # Errors
    ///
    /// When `author` may not do these acts now. Nothing is added then.
    pub fn write(&mut self, author: &SecretKey, acts: Vec<Act>) -> Result<Entry, Error> {
        self.now()
            .check(author.public(), &acts, false)
            .map_err(Error)?;
        let me = author.public();
        let prev = self
            .known
            .iter()
            .filter(|(_, k)| k.entry.header.author == me)
            .max_by_key(|(h, k)| (k.entry.header.seq, **h))
            .map(|(h, k)| (*h, k.entry.header.seq));
        let deps: Vec<Hash> = self
            .heads()
            .into_iter()
            .filter(|h| Some(*h) != prev.map(|p| p.0))
            .collect();
        let entry = Entry::sign(
            Some(self.id),
            prev.map_or(0, |p| p.1 + 1),
            prev.map(|p| p.0),
            deps,
            &acts,
            author,
        );
        self.insert(vec![entry.clone()])?;
        Ok(entry)
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

    /// Says if `who` may read `file` now.
    pub fn may_read(&self, who: Principal, file: Id) -> bool {
        self.may(who, Permission::FilesRead, &Resource::File(file))
    }

    /// The grants that are in place now (given and not taken back).
    pub fn grants(&self) -> Vec<Grant> {
        let view = self.now();
        self.state
            .gives
            .iter()
            .filter(|g| view.counts(g.at) && !view.taken(g.id))
            .map(|g| {
                let target = view.target(&g.on);
                let in_effect = view.holds(g.giver, Permission::AccessGive, &target);
                Grant {
                    id: g.id,
                    giver: g.giver,
                    to: g.to,
                    effect: g.effect,
                    permissions: g.permissions.clone(),
                    on: g.on.clone(),
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
        self.state
            .versions
            .iter()
            .filter(|v| self.state.valid[v.at])
            .filter_map(|v| v.version.content)
            .collect()
    }

    /// A hash of the state that has effect, to compare two nodes.
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
            .map(|g| (g.id, g.in_effect))
            .collect();
        Hash::of_value(&(files, grants))
    }
}
