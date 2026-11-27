// SPDX-License-Identifier: AGPL-3.0-only

//! Signed entries, and the acts in them.
//!
//! An entry has a small header (the space, the writer, its place in the
//! writer's log, the entries it depends on, and the hash of the body) and a
//! body (a list of [`Act`]s). The writer signs the header. The header names
//! the body by its hash, so a body can be deleted later and the chain of
//! hashes stays good.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

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

impl fmt::Display for Id {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.entry, self.index)
    }
}

impl std::str::FromStr for Id {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        let (entry, index) = text
            .split_once(':')
            .ok_or_else(|| Error::invalid(format!("not an entry and index: {text:?}")))?;
        Ok(Self {
            entry: entry.parse()?,
            index: index
                .parse()
                .map_err(|e| Error::invalid(format!("not an index: {index:?}")).with_source(e))?,
        })
    }
}

/// Someone who acts.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Principal {
    /// A key: a device, a node or an app. It signs entries.
    Key(PublicKey),
    /// A group, by the act that made it. It has no key: its members act for
    /// it.
    Group(Id),
    /// A person: a `did:fos` identity, by the hash of the first operation of
    /// its DID log. Its nodes act for it, once a `Vouch` in the space shows
    /// that they are its nodes.
    Identity(Hash),
}

impl fmt::Display for Principal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Key(key) => key.fmt(f),
            Self::Group(id) => write!(f, "group:{id}"),
            Self::Identity(did) => f.write_str(&fos_id::did_of(*did)),
        }
    }
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
    /// On a group: being in it.
    #[serde(rename = "member")]
    Member,
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::FilesList => "files.list",
            Self::FilesRead => "files.read",
            Self::FilesCreate => "files.create",
            Self::FilesChange => "files.change",
            Self::FilesDelete => "files.delete",
            Self::FilesHistory => "files.history",
            Self::FilesWatch => "files.watch",
            Self::AccessView => "access.view",
            Self::AccessGive => "access.give",
            Self::AccessTake => "access.take",
            Self::RulesSet => "rules.set",
            Self::ShareSend => "share.send",
            Self::DataKeep => "data.keep",
            Self::SpaceCheckpoint => "space.checkpoint",
            Self::Member => "member",
        })
    }
}

impl std::str::FromStr for Permission {
    type Err = Error;

    fn from_str(text: &str) -> Result<Self, Error> {
        match text {
            "files.list" => Ok(Self::FilesList),
            "files.read" => Ok(Self::FilesRead),
            "files.create" => Ok(Self::FilesCreate),
            "files.change" => Ok(Self::FilesChange),
            "files.delete" => Ok(Self::FilesDelete),
            "files.history" => Ok(Self::FilesHistory),
            "files.watch" => Ok(Self::FilesWatch),
            "access.view" => Ok(Self::AccessView),
            "access.give" => Ok(Self::AccessGive),
            "access.take" => Ok(Self::AccessTake),
            "rules.set" => Ok(Self::RulesSet),
            "share.send" => Ok(Self::ShareSend),
            "data.keep" => Ok(Self::DataKeep),
            "space.checkpoint" => Ok(Self::SpaceCheckpoint),
            "member" => Ok(Self::Member),
            _ => Err(Error::invalid(format!("not a permission: {text:?}"))),
        }
    }
}

/// What a grant is on.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Resource {
    /// Every name that starts with this text. `""` is the whole space.
    Prefix(String),
    /// One file, by its ID. It follows the file when the file is renamed.
    File(Id),
    /// A group. A grant of `member` on it puts someone in it; `access.give`
    /// on it lets someone put others in it.
    Group(Id),
}

impl fmt::Display for Resource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Prefix(prefix) if prefix.is_empty() => f.write_str("the whole space"),
            Self::Prefix(prefix) => write!(f, "prefix {prefix:?}"),
            Self::File(id) => write!(f, "file {id}"),
            Self::Group(id) => write!(f, "group {id}"),
        }
    }
}

/// Allow or deny. A deny wins over every allow.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
pub enum Effect {
    Allow,
    Deny,
}

impl fmt::Display for Effect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Allow => "allow",
            Self::Deny => "deny",
        })
    }
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
    /// Makes a group. Its ID is the place of this act. The owner never
    /// changes, and may be a group (owned by a group, and so on).
    Group { owner: Principal },
    /// Shows that a node is a node of an identity: the operations of the DID
    /// log, and a node certificate signed by the signing key of the identity.
    /// Every node checks it with nothing else, so anyone may write it.
    Vouch { ops: Vec<Bytes>, cert: Bytes },
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
            return Err(Error::invalid("the signature of the entry is wrong".into()));
        }
        if Hash::of(&self.body.0) != self.header.body {
            return Err(Error::invalid(
                "the body is not the one the header names".into(),
            ));
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
