// SPDX-License-Identifier: AGPL-3.0-only

//! Signed entries, and the acts in them.
//!
//! An entry has a small header (the space, the writer, its place in the
//! writer's log, the entries it depends on, and the hash of the body) and a
//! body (a list of [`Act`]s). The writer signs the header. The header names
//! the body by its hash, so a body can be deleted later and the chain of
//! hashes stays good.

use std::collections::{BTreeMap, BTreeSet};

use fos_core::{Bytes, Error, Hash, PublicKey, SecretKey, Signature, decode, encode};
use serde::{Deserialize, Serialize};

/// The place of an act: the hash of its entry and its index in that entry.
///
/// Files, versions and grants are named by the act that made them, so their
/// IDs prove themselves, as the ID of a space and a `did:fos` do.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Id {
    pub entry: Hash,
    pub index: u32,
}

/// Someone who acts: today a key (a device, a node or an app). Groups and
/// people (by `did:fos`) come later, as new variants.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Principal {
    Key(PublicKey),
}

/// An exact permission. Templates such as "Read and write" are lists of
/// these, made by the interface; the kernel sees only the list.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Permission {
    #[serde(rename = "files.list")]
    FilesList,
    #[serde(rename = "files.read")]
    FilesRead,
    #[serde(rename = "files.create")]
    FilesCreate,
    #[serde(rename = "files.change")]
    FilesChange,
    #[serde(rename = "files.delete")]
    FilesDelete,
    #[serde(rename = "files.history")]
    FilesHistory,
    #[serde(rename = "files.watch")]
    FilesWatch,
    #[serde(rename = "access.view")]
    AccessView,
    #[serde(rename = "access.give")]
    AccessGive,
    #[serde(rename = "access.take")]
    AccessTake,
    #[serde(rename = "rules.set")]
    RulesSet,
    #[serde(rename = "share.send")]
    ShareSend,
    #[serde(rename = "data.keep")]
    DataKeep,
    #[serde(rename = "space.checkpoint")]
    SpaceCheckpoint,
}

/// What a grant is on.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Resource {
    /// Every name that starts with this text. `""` is the whole space.
    Prefix(String),
    /// One file, by its ID. It follows the file when the file is renamed.
    File(Id),
}

/// Allow or deny. A deny wins over every allow.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Effect {
    Allow,
    Deny,
}

/// The bytes of a version. The bytes themselves are kept apart, in the
/// content store, in chunks (see [`crate::content`]).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub struct Content {
    /// The hash of the bytes when there is one chunk; the hash of the list
    /// of chunks when there are more.
    pub hash: Hash,
    pub size: u64,
    /// True when `hash` names a list of chunks.
    pub chunks: bool,
}

impl Content {
    /// The content of `bytes`, cut into chunks as [`crate::content::split`]
    /// does. The caller keeps the pieces that `split` gives.
    pub fn of(bytes: &[u8]) -> Self {
        crate::content::split(bytes).0
    }
}

/// One thing that an entry does.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub enum Act {
    /// The first entry of a space. Its hash is the ID of the space.
    Genesis {
        owner: Principal,
        /// So that two spaces made at the same time are different.
        nonce: u64,
    },
    /// Makes a file. The ID of the file is the place of this act.
    Create { name: String, content: Content },
    /// Makes a new version of a file, after the versions in `parents`. It can
    /// change the name, the content, or both. With no content, the file is
    /// deleted.
    Change {
        file: Id,
        parents: Vec<Id>,
        name: String,
        content: Option<Content>,
    },
    /// Gives a grant. Its ID is the place of this act.
    Give {
        to: Principal,
        effect: Effect,
        permissions: Vec<Permission>,
        on: Resource,
    },
    /// Takes back a grant, and with it every grant that was given on from it.
    Take { grant: Id },
    /// Makes the state in the past of this entry final, and signs its hash
    /// (see `space::Summary`). Made by the owner, or by a principal with
    /// `space.checkpoint` on the whole space. It is alone in its entry.
    Checkpoint { state: Hash, retention: Retention },
}

const HOUR: u64 = 60 * 60 * 1000;
const DAY: u64 = 24 * HOUR;
const WEEK: u64 = 7 * DAY;

/// How long old versions and deleted files are kept: thinner with age, as
/// Time Machine and restic's `forget --keep-…` do. A checkpoint says which
/// retention it uses, so every node keeps the same.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Retention {
    /// Keep one version for each of the last this many hours.
    pub hours: u32,
    /// Then one for each of this many days.
    pub days: u32,
    /// Then one for each of this many weeks.
    pub weeks: u32,
    /// Keep deleted files (the bin) for this many days.
    pub deleted_days: u32,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            hours: 24,
            days: 30,
            weeks: 52,
            deleted_days: 30,
        }
    }
}

impl Retention {
    /// The versions to keep, of `versions` (each with its time), at `now`:
    /// the newest in each hour, day and week that is still kept.
    pub fn keep(&self, versions: &[(Id, u64)], now: u64) -> BTreeSet<Id> {
        let mut keep = BTreeSet::new();
        for (size, count) in [(HOUR, self.hours), (DAY, self.days), (WEEK, self.weeks)] {
            let mut newest: BTreeMap<u64, (u64, Id)> = BTreeMap::new();
            for (id, time) in versions {
                if now.saturating_sub(*time) >= size * u64::from(count) {
                    continue;
                }
                let slot = newest.entry(time / size).or_insert((*time, *id));
                if (*time, *id) > *slot {
                    *slot = (*time, *id);
                }
            }
            keep.extend(newest.values().map(|(_, id)| *id));
        }
        keep
    }

    /// Says if a file deleted at `time` is still kept at `now`.
    pub fn keeps_deleted(&self, time: u64, now: u64) -> bool {
        now.saturating_sub(time) < DAY * u64::from(self.deleted_days)
    }
}

/// The signed part of an entry.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Header {
    /// The space. `None` only in the first entry, whose hash is the ID.
    pub space: Option<Hash>,
    pub author: PublicKey,
    /// The place in the author's log in this space, from 0.
    pub seq: u64,
    /// The author's entry before this one (with `seq - 1`).
    pub prev: Option<Hash>,
    /// Entries of other logs that this one comes after.
    pub deps: Vec<Hash>,
    /// The time the author says it wrote this, in milliseconds since 1970.
    /// Only for people and for retention; never for access.
    pub time: u64,
    /// The hash of the body.
    pub body: Hash,
}

impl Header {
    /// The entries this one comes after: `prev` and `deps`.
    pub fn parents(&self) -> impl Iterator<Item = Hash> + '_ {
        self.prev.iter().copied().chain(self.deps.iter().copied())
    }
}

/// A signed entry.
#[derive(Clone, PartialEq, Eq, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub header: Header,
    pub signature: Signature,
    /// The CBOR form of a list of acts.
    pub body: Bytes,
}

impl Entry {
    /// Makes and signs an entry.
    pub fn sign(
        space: Option<Hash>,
        seq: u64,
        prev: Option<Hash>,
        deps: Vec<Hash>,
        time: u64,
        acts: &[Act],
        key: &SecretKey,
    ) -> Self {
        let body = encode(&acts);
        let header = Header {
            space,
            author: key.public(),
            seq,
            prev,
            deps,
            time,
            body: Hash::of(&body),
        };
        let signature = key.sign(&encode(&header));
        Self {
            header,
            signature,
            body: Bytes(body),
        }
    }

    /// The hash of the header, which names the entry.
    pub fn hash(&self) -> Hash {
        Hash::of_value(&self.header)
    }

    /// Checks the signature and the body, and gives back the acts.
    ///
    /// # Errors
    ///
    /// When the signature or the hash of the body is wrong, or the body is
    /// not a list of acts.
    pub fn open(&self) -> Result<Vec<Act>, Error> {
        if !self
            .header
            .author
            .verify(&encode(&self.header), &self.signature)
        {
            return Err(Error("the signature of the entry is wrong".into()));
        }
        if Hash::of(&self.body.0) != self.header.body {
            return Err(Error("the body is not the one the header names".into()));
        }
        decode(&self.body.0)
    }
}

/// Says if `name` is a good file name: not empty, no `/` at the start or
/// end, no empty, `.` or `..` parts, and at most 1024 bytes.
pub fn good_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 1024
        && name
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}
