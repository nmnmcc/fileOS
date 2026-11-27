// SPDX-License-Identifier: AGPL-3.0-only

//! One fileOS node.
//!
//! A node puts the parts together: the data directory ([`Store`]), the
//! spaces and sync ([`fileos_space::Node`]), the apps, and the network.
//!
//! Paths here are node paths. The node has a **home space**. Other spaces
//! are mounted in it at names such as `photos` or `apps/chat`, so the node
//! path `/photos/2026/a.jpg` is the name `2026/a.jpg` in the space mounted at
//! `photos`. A mount is a small file in the home space, under `.mounts/`.
//!
//! The node and the people who use it through the gateway act with the node
//! key. Each app acts with its own key, which comes from the node key. Every
//! write, by anyone, is checked against the grants in its space.
//!
//! A node may be one of the nodes of an identity (a person, named by a
//! `did:fileos` DID). The identity space is mounted at [`IDENTITY_AT`]. It has
//! the DID log and a certificate for each node. It is owned by the signing
//! key of the identity, so only the node that holds that key can add or take
//! away nodes.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::{self, Write as _};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use fileos_core::{Error, Hash, PublicKey, SecretKey};
use fileos_runtime::{AppInstance, ChangeKind, FsError, HttpApp, Runtime};
use fileos_space::{
    Act, Content, Contents as _, Effect, Event, File, Id, Msg, Permission, Resource, Retention,
    Space, Step,
};
use fileos_store::Store;

pub use fileos_space::Principal;

mod app;
mod auth;
mod identity;
mod net;

#[doc(inline)]
pub use app::{Installed, app_id, pack_app};
#[doc(inline)]
pub use identity::{IDENTITY_AT, Identity, parse_principal};
#[doc(inline)]
pub use net::NetConfig;

/// The prefix of the files in the home space that say what is mounted where.
const MOUNTS: &str = ".mounts/";

/// The prefix of the files in the home space that say what each app sees:
/// `.views/<app>/<name>` holds the space, the prefix in it, and the grant.
const VIEWS: &str = ".views/";

/// Where the private data space of each app is mounted in the home space.
const APPDATA: &str = ".appdata/";

/// The file in the home space of an identity with the seed of app keys.
const APP_SEED: &str = ".app-seed";

/// Where the requests of apps for access wait for the person, in the home
/// space: `.requests/<id>`.
const REQUESTS: &str = ".requests/";

/// A request of an app for access, waiting for the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccessRequest {
    pub id: String,
    /// The ID in the app's manifest.
    pub app: String,
    /// What the app says it is for.
    pub purpose: String,
    /// What it asks for: exact permissions or templates, or `share` when
    /// it offers a file to another app.
    pub permissions: String,
    /// The name of the file it offers, for a share.
    pub file: Option<String>,
    /// Where that file is: its space and its ID.
    share: Option<(Hash, Id)>,
}

/// One mount in the view of an app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ViewMount {
    /// The name the app sees, with no `/` at the start: `photos`, or
    /// `granted/<handle>`.
    pub name: String,
    /// The space it stands for. The app does not see it.
    pub space: Hash,
    /// The name in that space it stands for. The app does not see it.
    pub prefix: String,
    /// The grant to the app's key that goes with it.
    pub grant: Option<Id>,
}

/// A node makes a new recipient key after this time (a policy, with a
/// default: 30 days).
const RECIPIENT_MAX_AGE: u64 = 30 * 24 * 60 * 60 * 1000;

/// After this many entries, a node makes a checkpoint of a space it owns and
/// compacts it. A policy, with a default.
const CHECKPOINT_AFTER: usize = 1000;

/// Node paths that changed, and how, for the apps that watch them.
type Changed = Mutex<VecDeque<(String, ChangeKind)>>;

/// One node. Cheap to clone; all clones are the same node.
#[derive(Clone)]
pub struct Node {
    shared: Arc<Shared>,
}

impl fmt::Debug for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Node({})", self.key())
    }
}

struct Shared {
    inner: Mutex<Inner>,
    apps: Mutex<Vec<AppInstance>>,
    /// The apps that give HTTP, by app ID: the name of their origin.
    http: Mutex<BTreeMap<String, HttpApp>>,
    /// Node paths that changed, for the apps that watch them.
    changed: Changed,
    /// The schedules of each app, by app ID.
    schedules: Mutex<BTreeMap<String, Vec<fileos_runtime::Schedule>>>,
    /// The most each app may ask for, by the ID in its manifest.
    asks: Mutex<BTreeMap<String, BTreeSet<Permission>>>,
    /// What each app takes from others, by the ID in its manifest.
    receives: Mutex<BTreeMap<String, Vec<fileos_runtime::Receive>>>,
    /// True once `start_schedules` ran: apps started later get their
    /// timers at once.
    scheduling: std::sync::atomic::AtomicBool,
    runtime: Runtime,
    key: PublicKey,
    /// Sign-in for people, once the gateway runs (it needs the port).
    auth: Mutex<Option<fileos_auth::Auth>>,
}

struct Inner {
    store: Store,
    /// The home space of this node itself. `home` is the home space of the
    /// identity instead, once the node has one: then mounts, views and apps
    /// are the same on every node of the person.
    node_home: Hash,
    clock: fn() -> u64,
    key: SecretKey,
    node: fileos_space::Node,
    home: Hash,
    kv: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
    net: Option<fileos_net::Commands>,
    net_dirty: bool,
    listening: Vec<String>,
    /// For each space: the entries kept on disk, and the checkpoint of the
    /// base that is on disk.
    saved: BTreeMap<Hash, (BTreeSet<Hash>, Option<Hash>)>,
    /// For each space: the newest versions of each name, as apps last heard.
    seen: BTreeMap<Hash, BTreeMap<String, Vec<Id>>>,
    /// Node paths that wait for content before apps hear about them.
    waiting: BTreeMap<Hash, Vec<(String, ChangeKind)>>,
    /// The last messages from apps and the kernel, newest last.
    logs: VecDeque<String>,
    /// The rotation key of the identity, when this node holds it.
    rotation_key: Option<SecretKey>,
    /// The signing key of the identity, when this node holds it.
    signing_key: Option<SecretKey>,
}

/// A file version ready to read in chunks.
#[derive(Debug)]
pub struct FileReader {
    reader: fileos_space::content::Reader,
    store: Store,
}

impl FileReader {
    /// The number of bytes in the file.
    pub fn len(&self) -> u64 {
        self.reader.content().size
    }

    /// Whether there are zero bytes in the file.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Gives each checked chunk to the caller, in order.
    ///
    /// # Errors
    ///
    /// When a piece cannot be read or the caller stops the read.
    pub fn read(&self, write: impl FnMut(Vec<u8>) -> Option<()>) -> Result<(), Error> {
        self.reader
            .read(&self.store, write)
            .ok_or_else(|| Error::unavailable("could not finish reading the file".into()))
    }
}

/// What a node is doing, for people.
#[derive(Clone, Debug)]
pub struct Status {
    pub key: PublicKey,
    pub home: Hash,
    pub listening: Vec<String>,
    pub peers: Vec<PublicKey>,
    pub spaces: Vec<SpaceStatus>,
    /// Spaces that are mounted but not here yet.
    pub wanted: Vec<Hash>,
    pub apps: Vec<String>,
    pub logs: Vec<String>,
    pub identity: Option<Identity>,
}

#[derive(Clone, Debug)]
pub struct SpaceStatus {
    pub id: Hash,
    pub owner: Principal,
    pub mounted_at: Vec<String>,
    pub entries: usize,
    /// Entries in the graph that have no effect.
    pub rejected: usize,
    /// The checkpoint the space was compacted at, if any.
    pub base: Option<Hash>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().expect("node state was left incomplete by a panic")
}

/// Sources for the time of file changes and for new keys.
#[derive(Clone, Copy, Debug)]
pub struct Sources {
    /// The time in milliseconds since 1970.
    pub clock: fn() -> u64,
    /// Fills a new seed from a source of random bytes.
    pub entropy: fn() -> Result<[u8; 32], Error>,
}

impl Default for Sources {
    fn default() -> Self {
        Self {
            clock: now,
            entropy: fileos_store::random_seed,
        }
    }
}

/// The time now, in milliseconds since 1970.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A node path without its first `/`, and with no `/` at the end.
fn clean(path: &str) -> String {
    path.trim_matches('/').to_string()
}

/// The token that local clients give to the gateway of the node in `dir`.
///
/// # Errors
///
/// When the data directory cannot be read.
pub fn token(dir: &Path) -> Result<String, Error> {
    Store::open(dir)?.token()
}

/// Writes the port of the running gateway into `dir`, or takes it away.
///
/// With `None`, the files go. Next to it is `gateway.curl`, a curl config file that `fileos curl`
/// uses: it sends the token, and sends every connection to the gateway, so
/// the token never leaves this machine.
///
/// # Errors
///
/// When a file cannot be written.
pub fn write_gateway_addr(dir: &Path, port: Option<u16>) -> Result<(), Error> {
    let addr = dir.join("gateway.addr");
    let curl = dir.join("gateway.curl");
    let Some(port) = port else {
        let _ = std::fs::remove_file(&curl);
        return std::fs::remove_file(&addr).or(Ok(()));
    };
    let token = Store::open(dir)?.token()?;
    fileos_store::write_atomic(
        &curl,
        format!(
            "# Made by `fileos node run` for `fileos curl`. It goes when the node stops.\n\
             header = \"Authorization: Bearer {token}\"\n\
             connect-to = \"::127.0.0.1:{port}\"\n"
        )
        .as_bytes(),
    )?;
    fileos_store::write_atomic(&addr, format!("127.0.0.1:{port}").as_bytes())
}

/// The permissions of the templates, for giving access to others.
const READ: [Permission; 5] = [
    Permission::FilesList,
    Permission::FilesRead,
    Permission::FilesHistory,
    Permission::FilesWatch,
    Permission::DataKeep,
];
const WRITE: [Permission; 3] = [
    Permission::FilesCreate,
    Permission::FilesChange,
    Permission::FilesDelete,
];
const MANAGE: [Permission; 6] = [
    Permission::AccessView,
    Permission::AccessGive,
    Permission::AccessTake,
    Permission::RulesSet,
    Permission::ShareSend,
    Permission::SpaceCheckpoint,
];

/// Reads permissions: exact names such as `files.read`, or the templates
/// `read`, `read-write` (or `write`), `manage` and `relay`, with `,`
/// between them.
///
/// # Errors
///
/// When a name is not a permission or a template.
pub fn parse_permissions(list: &str) -> Result<Vec<Permission>, Error> {
    let mut out = BTreeSet::new();
    for name in list.split(',').map(str::trim) {
        match name {
            "read" => out.extend(READ),
            "write" | "read-write" | "append" => out.extend(READ.into_iter().chain(WRITE)),
            "manage" | "all" => out.extend(READ.into_iter().chain(WRITE).chain(MANAGE)),
            "relay" | "store" => {
                out.insert(Permission::DataKeep);
            }
            other => {
                let permission = other
                    .parse::<Permission>()
                    .map_err(|e| e.context("give a permission or template"))?;
                out.insert(permission);
            }
        }
    }
    Ok(out.into_iter().collect())
}

impl Node {
    /// Makes a new node in `dir`, with its home space.
    ///
    /// # Errors
    ///
    /// When the directory already has a node.
    pub fn init(dir: impl AsRef<Path>) -> Result<PublicKey, Error> {
        Self::init_with_sources(dir.as_ref(), Sources::default())
    }

    /// Makes a node with a caller-supplied clock and source of random bytes.
    ///
    /// # Errors
    ///
    /// When the node already exists or its data files cannot be made.
    pub fn init_with_sources(dir: &Path, sources: Sources) -> Result<PublicKey, Error> {
        let store = Store::open_with_entropy(dir, sources.entropy)?;
        if store.home()?.is_some() {
            return Err(Error::invalid(format!(
                "{} already has a node",
                dir.display()
            )));
        }
        let key = store.key()?;
        let seed = store.seed()?;
        let (space, genesis) = Space::create(
            Principal::Key(key.public()),
            &key,
            (sources.clock)(),
            u64::from_le_bytes(seed[..8].try_into().unwrap_or_default()),
        );
        store.append_entries(space.id(), &[genesis])?;
        store.set_home(space.id())?;
        Ok(key.public())
    }

    /// Opens the node in `dir`, and starts its apps.
    ///
    /// # Errors
    ///
    /// When there is no node in `dir`, or its data is broken.
    pub fn open(dir: impl AsRef<Path>) -> Result<Node, Error> {
        Self::open_with_sources(dir.as_ref(), Sources::default())
    }

    /// Opens a node with a caller-supplied clock and source of random bytes.
    ///
    /// # Errors
    ///
    /// When there is no node in the directory, or its data cannot be read.
    pub fn open_with_sources(dir: &Path, sources: Sources) -> Result<Node, Error> {
        let store = Store::open_with_entropy(dir, sources.entropy)?;
        let home = store.home()?.ok_or_else(|| {
            Error::invalid(format!(
                "{} has no node: run `fileos node init`",
                dir.display()
            ))
        })?;
        let key = store.key()?;
        let rotation_key = store.identity_key("rotation")?;
        let signing_key = store.identity_key("signing")?;
        let seed = store.seed()?;
        let mut node = fileos_space::Node::new(
            key.clone(),
            u64::from_le_bytes(seed[..8].try_into().unwrap_or_default()),
        );
        if let Some(signing) = &signing_key {
            node.add_key(signing.clone());
        }
        node.set_recipients(store.recipients()?);
        let mut saved = BTreeMap::new();
        for space in store.load_spaces()? {
            let hashes = space.iter().map(fileos_space::Entry::hash).collect();
            let base = space.base().map(|b| b.checkpoint.hash());
            saved.insert(space.id(), (hashes, base));
            node.insert(space);
        }
        let mut inner = Inner {
            store,
            clock: sources.clock,
            key: key.clone(),
            node,
            home,
            node_home: home,
            kv: BTreeMap::new(),
            net: None,
            net_dirty: false,
            listening: Vec::new(),
            saved,
            seen: BTreeMap::new(),
            waiting: BTreeMap::new(),
            logs: VecDeque::new(),
            rotation_key,
            signing_key,
        };
        // Follow the mounted spaces that are not here yet.
        for (_, id) in inner.mounts() {
            let _ = inner.node.follow(id);
        }
        for id in inner.node.spaces().map(Space::id).collect::<Vec<_>>() {
            inner.note(id, &Mutex::new(VecDeque::new()));
        }
        inner.update_proof(&Mutex::new(VecDeque::new()))?;
        inner.rotate_if_due(&Mutex::new(VecDeque::new()))?;
        let node = Node {
            shared: Arc::new(Shared {
                inner: Mutex::new(inner),
                apps: Mutex::new(Vec::new()),
                http: Mutex::new(BTreeMap::new()),
                changed: Mutex::new(VecDeque::new()),
                schedules: Mutex::new(BTreeMap::new()),
                asks: Mutex::new(BTreeMap::new()),
                receives: Mutex::new(BTreeMap::new()),
                scheduling: std::sync::atomic::AtomicBool::new(false),
                runtime: Runtime::new()?,
                key: key.public(),
                auth: Mutex::new(None),
            }),
        };
        node.start_apps();
        Ok(node)
    }

    /// The key of this node. It is also its libp2p identity.
    pub fn key(&self) -> PublicKey {
        self.shared.key
    }

    fn inner(&self) -> MutexGuard<'_, Inner> {
        lock(&self.shared.inner)
    }

    /// Makes a new space, owned by this node, and mounts it at `at`. Gives
    /// its ID.
    ///
    /// # Errors
    ///
    /// When something is mounted at `at` already.
    pub fn create(&self, at: &str) -> Result<Hash, Error> {
        let mut inner = self.inner();
        let time = (inner.clock)();
        let (id, step) = inner.node.create(time);
        inner.process(step, &self.shared.changed)?;
        inner.mount(id, at, &self.shared.changed)?;
        Ok(id)
    }

    /// Mounts a space at `at`, and starts to get it from peers.
    ///
    /// # Errors
    ///
    /// When something is mounted at `at` already.
    pub fn mount(&self, space: Hash, at: &str) -> Result<(), Error> {
        let mut inner = self.inner();
        inner.mount(space, at, &self.shared.changed)?;
        let step = inner.node.follow(space);
        inner.process(step, &self.shared.changed)
    }

    /// Puts a file: a new version when the name is there, a new file when
    /// it is not.
    ///
    /// # Errors
    ///
    /// When the path is not valid, or the node has no grant for it.
    pub fn put(&self, path: &str, data: &[u8]) -> Result<(), Error> {
        let result = self.inner().put(None, path, data, &self.shared.changed);
        self.pump();
        result
    }

    /// Adds bytes to the end of a file, as a new version.
    ///
    /// # Errors
    ///
    /// When the path is not valid, or the node has no grant for it.
    pub fn append(&self, path: &str, data: &[u8]) -> Result<(), Error> {
        let result = self.inner().append(None, path, data, &self.shared.changed);
        self.pump();
        result
    }

    /// Deletes a file, or takes a mount away.
    ///
    /// # Errors
    ///
    /// When the path is not there, or the node has no grant for it.
    pub fn delete(&self, path: &str) -> Result<(), Error> {
        let result = self.inner().delete(None, path, &self.shared.changed);
        self.pump();
        result
    }

    /// The bytes of a file.
    ///
    /// # Errors
    ///
    /// When the path is not there, or the node has no grant for it.
    pub fn read(&self, path: &str) -> Result<Vec<u8>, Error> {
        let mut bytes = Vec::new();
        self.open_file(path)?.read(|chunk| {
            bytes.try_reserve(chunk.len()).ok()?;
            bytes.extend_from_slice(&chunk);
            Some(())
        })?;
        Ok(bytes)
    }

    /// Opens a file for a read in chunks.
    ///
    /// # Errors
    ///
    /// When the file is missing, cannot be read, or has pieces still to come.
    pub fn open_file(&self, path: &str) -> Result<FileReader, Error> {
        let inner = self.inner();
        let (space, name) = inner.resolve(path);
        let reader = inner.reader_in(self.key(), space, &name)?;
        if !fileos_space::content::complete(&reader.content(), &inner.store) {
            return Err(Error::unavailable(
                "the file is not on this node yet".into(),
            ));
        }
        Ok(FileReader {
            reader,
            store: inner.store.clone(),
        })
    }

    /// The names directly under a path.
    ///
    /// # Errors
    ///
    /// When the space is not here, or the node has no grant for it.
    pub fn list(&self, path: &str) -> Result<Vec<String>, Error> {
        let key = self.key();
        self.inner().list(key, path).map_err(Error::from)
    }

    /// Gives `to` permissions on `path`: on the file when there is a file
    /// with that name (the grant follows it when it is renamed), or else on
    /// every name under it. Gives the ID of the grant.
    ///
    /// # Errors
    ///
    /// When the node may not give them.
    pub fn grant(
        &self,
        path: &str,
        to: PublicKey,
        permissions: &[Permission],
    ) -> Result<Id, Error> {
        self.grant_to(path, Principal::Key(to), permissions)
    }

    /// Gives a principal permissions on `path`, as [`Node::grant`] does. With
    /// [`Principal::Identity`], every node of that person gets them.
    ///
    /// # Errors
    ///
    /// When the node may not give them.
    pub fn grant_to(
        &self,
        path: &str,
        to: Principal,
        permissions: &[Permission],
    ) -> Result<Id, Error> {
        let result = self
            .inner()
            .grant(None, path, to, permissions, &self.shared.changed);
        self.pump();
        result
    }

    /// Takes back a grant in the space that has `path`.
    ///
    /// # Errors
    ///
    /// When the node may not take it back.
    pub fn take(&self, path: &str, grant: Id) -> Result<(), Error> {
        let mut inner = self.inner();
        let (space, _) = inner.resolve(path);
        inner.write(None, space, vec![Act::Take { grant }], &self.shared.changed)?;
        drop(inner);
        self.pump();
        Ok(())
    }

    /// The grants in place in the space that has `path`, as text: one line
    /// each.
    pub fn grants(&self, path: &str) -> Vec<String> {
        let inner = self.inner();
        let (space, _) = inner.resolve(path);
        let Some(s) = inner.node.space(space) else {
            return Vec::new();
        };
        s.grants()
            .into_iter()
            .map(|g| {
                let g_ = &g.given;
                let permissions = g_
                    .permissions
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "{} {} -> {} {} on {}{}",
                    g_.id,
                    g_.effect,
                    g_.to,
                    permissions,
                    g_.on,
                    if g.in_effect { "" } else { " (not in effect)" }
                )
            })
            .collect()
    }

    /// Makes a checkpoint of the space that has `path`, and compacts it.
    ///
    /// # Errors
    ///
    /// When the node may not make checkpoints of it.
    pub fn checkpoint(&self, path: &str) -> Result<(), Error> {
        let mut inner = self.inner();
        let (space, _) = inner.resolve(path);
        inner.checkpoint(space, &self.shared.changed)
    }

    /// What this node is doing.
    pub fn status(&self) -> Status {
        let apps = lock(&self.shared.apps)
            .iter()
            .map(|a| a.manifest().id.clone())
            .collect();
        let mut inner = self.inner();
        let mounts = inner.mounts();
        let spaces = inner
            .node
            .spaces()
            .map(|s| SpaceStatus {
                id: s.id(),
                owner: s.owner(),
                mounted_at: mounts
                    .iter()
                    .filter(|(_, id)| *id == s.id())
                    .map(|(at, _)| format!("/{at}"))
                    .collect(),
                entries: s.len(),
                rejected: s.rejected().len(),
                base: s.base().map(|b| b.checkpoint.hash()),
            })
            .collect();
        let peers = inner.node.peers().copied().collect();
        let wanted = inner.node.wanted().copied().collect();
        let logs = inner.logs.drain(..).collect();
        let identity = inner.identity(self.key());
        Status {
            key: self.key(),
            home: inner.home,
            listening: inner.listening.clone(),
            peers,
            spaces,
            wanted,
            apps,
            logs,
            identity,
        }
    }
}

/// A grant ID as text: the hash of its entry and its index.
fn grant_text(id: Id) -> String {
    format!("{}:{}", id.entry, id.index)
}

/// Reads a grant ID written as `<entry hash>:<index>`.
///
/// # Errors
///
/// When the text is not a grant ID.
pub fn parse_grant(text: &str) -> Result<Id, Error> {
    text.parse()
        .map_err(|error: Error| error.context("bad grant ID"))
}

/// The newest version of a file that has content: when there are more, the
/// one made last (by its time, then its ID).
fn newest(file: &File) -> Option<Content> {
    newest_version(file).and_then(|v| v.content)
}

fn newest_version(file: &File) -> Option<&fileos_space::Version> {
    file.versions
        .iter()
        .filter(|v| v.content.is_some())
        .max_by_key(|v| (v.time, v.id))
}

impl Inner {
    /// Every mount: (the name it is mounted at, without `/`, and the space).
    fn mounts(&self) -> Vec<(String, Hash)> {
        let Some(home) = self.node.space(self.home) else {
            return Vec::new();
        };
        home.files()
            .into_iter()
            .filter_map(|f| {
                let v = f.versions.iter().find(|v| v.name.starts_with(MOUNTS))?;
                let at = v.name[MOUNTS.len()..].to_string();
                let bytes = self.bytes(self.home, &f)?;
                let id = String::from_utf8(bytes).ok()?.trim().parse().ok()?;
                Some((at, id))
            })
            .collect()
    }

    /// The space and the name in it of a node path. Names are in the home
    /// space when no mount has them.
    fn resolve(&self, path: &str) -> (Hash, String) {
        let path = clean(path);
        let mut best: Option<(String, Hash)> = None;
        for (at, id) in self.mounts() {
            let fits = path == at || path.starts_with(&format!("{at}/"));
            if fits && best.as_ref().is_none_or(|(b, _)| at.len() > b.len()) {
                best = Some((at, id));
            }
        }
        match best {
            Some((at, id)) => (id, path[at.len()..].trim_start_matches('/').to_string()),
            None => (self.home, path),
        }
    }

    fn mount(&mut self, space: Hash, at: &str, changed: &Changed) -> Result<(), Error> {
        let at = clean(at);
        if at.is_empty() || self.mounts().iter().any(|(m, _)| *m == at) {
            return Err(Error::invalid(format!(
                "something is mounted at /{at} already"
            )));
        }
        let home = self.home;
        self.put_name(
            None,
            home,
            &format!("{MOUNTS}{at}"),
            space.to_string().as_bytes(),
            changed,
        )
    }

    /// Writes acts into a space, as `author` (the node when `None`).
    fn write(
        &mut self,
        author: Option<&SecretKey>,
        space: Hash,
        acts: Vec<Act>,
        changed: &Changed,
    ) -> Result<Id, Error> {
        let author = author.cloned().unwrap_or_else(|| self.key.clone());
        let (entry, step) = self.node.write_as(space, &author, (self.clock)(), acts)?;
        self.process(step, changed)?;
        Ok(Id {
            entry: entry.hash(),
            index: 0,
        })
    }

    /// The bytes of the newest version of a file, if this node can open
    /// them and has all of the content.
    fn bytes(&self, space: Hash, file: &File) -> Option<Vec<u8>> {
        let v = newest_version(file)?;
        self.node.open(space, file.id, v, &self.store)
    }

    fn put(
        &mut self,
        author: Option<&SecretKey>,
        path: &str,
        data: &[u8],
        changed: &Changed,
    ) -> Result<(), Error> {
        let (space, name) = self.resolve(path);
        self.put_name(author, space, &name, data, changed)
    }

    fn put_name(
        &mut self,
        author: Option<&SecretKey>,
        space: Hash,
        name: &str,
        data: &[u8],
        changed: &Changed,
    ) -> Result<(), Error> {
        let s = self
            .node
            .space(space)
            .ok_or_else(|| Error::unavailable(format!("space {space} is not here yet")))?;
        let file = s.find(name).into_iter().next();
        let (content, key, pieces) =
            self.node
                .seal(space, name, file.as_ref().map(|f| f.id), data)?;
        for (hash, bytes) in pieces {
            self.store.put_piece(hash, &bytes)?;
        }
        let act = match file {
            Some(file) => Act::Change {
                file: file.id,
                parents: file.versions.iter().map(|v| v.id).collect(),
                name: name.to_string(),
                content: Some(content),
                key,
            },
            None => Act::Create {
                name: name.to_string(),
                content,
                key,
            },
        };
        self.write(author, space, vec![act], changed).map(drop)
    }

    fn append(
        &mut self,
        author: Option<&SecretKey>,
        path: &str,
        data: &[u8],
        changed: &Changed,
    ) -> Result<(), Error> {
        let who = author.map_or_else(|| self.key.public(), SecretKey::public);
        let (space, name) = self.resolve(path);
        let mut bytes = match self.read_in(who, space, &name) {
            Ok(bytes) => bytes,
            Err(FsError::NotFound) => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        bytes.extend_from_slice(data);
        self.put(author, path, &bytes, changed)
    }

    fn delete(
        &mut self,
        author: Option<&SecretKey>,
        path: &str,
        changed: &Changed,
    ) -> Result<(), Error> {
        let at = clean(path);
        if author.is_none() && self.mounts().iter().any(|(m, _)| *m == at) {
            let home = self.home;
            return self.delete_name(None, home, &format!("{MOUNTS}{at}"), changed);
        }
        let (space, name) = self.resolve(path);
        self.delete_name(author, space, &name, changed)
    }

    fn delete_name(
        &mut self,
        author: Option<&SecretKey>,
        space: Hash,
        name: &str,
        changed: &Changed,
    ) -> Result<(), Error> {
        let s = self
            .node
            .space(space)
            .ok_or_else(|| Error::unavailable(format!("space {space} is not here yet")))?;
        let file = s
            .find(name)
            .into_iter()
            .next()
            .ok_or_else(|| Error::not_found(format!("{name:?} is not there")))?;
        let act = Act::Change {
            file: file.id,
            parents: file.versions.iter().map(|v| v.id).collect(),
            name: name.to_string(),
            content: None,
            key: None,
        };
        self.write(author, space, vec![act], changed).map(drop)
    }

    /// What a grant on `name` in `space` is on: the file with that name, or
    /// every name under it.
    fn resource(s: &Space, name: &str) -> Resource {
        match s.find(name).first() {
            Some(file) => Resource::File(file.id),
            None if name.is_empty() => Resource::Prefix(String::new()),
            None => Resource::Prefix(format!("{name}/")),
        }
    }

    fn grant(
        &mut self,
        author: Option<&SecretKey>,
        path: &str,
        to: Principal,
        permissions: &[Permission],
        changed: &Changed,
    ) -> Result<Id, Error> {
        let (space, name) = self.resolve(path);
        let s = self
            .node
            .space(space)
            .ok_or_else(|| Error::unavailable(format!("space {space} is not here yet")))?;
        let on = Self::resource(s, &name);
        self.write(
            author,
            space,
            vec![Act::Give {
                to,
                effect: Effect::Allow,
                permissions: permissions.to_vec(),
                on,
            }],
            changed,
        )
    }

    fn checkpoint(&mut self, space: Hash, changed: &Changed) -> Result<(), Error> {
        let step = self
            .node
            .checkpoint(space, (self.clock)(), Retention::default())?;
        self.process(step, changed)?;
        let (_, step) = self.node.compact(space);
        self.apply(step, changed)?;
        self.save(space)
    }

    /// Says if `who` may do `permission` on `name` in `space`.
    fn may(s: &Space, who: PublicKey, permission: Permission, name: &str) -> bool {
        let on = Self::resource(s, name);
        s.may(Principal::Key(who), permission, &on)
    }

    fn reader_in(
        &self,
        who: PublicKey,
        space: Hash,
        name: &str,
    ) -> Result<fileos_space::content::Reader, FsError> {
        let s = self.node.space(space).ok_or(FsError::NotFound)?;
        let file = s.find(name).into_iter().next().ok_or(FsError::NotFound)?;
        if !Self::may(s, who, Permission::FilesRead, name) {
            return Err(FsError::Denied(format!("{who} may not read {name}")));
        }
        let version = newest_version(&file).ok_or(FsError::NotFound)?;
        self.node
            .reader(space, file.id, version)
            .ok_or_else(|| FsError::Unavailable("this node has no key for the file yet".into()))
    }

    fn read_in(&self, who: PublicKey, space: Hash, name: &str) -> Result<Vec<u8>, FsError> {
        self.reader_in(who, space, name)?
            .join(&self.store)
            .ok_or_else(|| FsError::Unavailable("the file is not on this node yet".into()))
    }

    fn list(&self, who: PublicKey, path: &str) -> Result<Vec<String>, FsError> {
        let (space, name) = self.resolve(path);
        let mut names = self.list_in(who, space, &name)?;
        if space == self.home {
            names.retain(|n| ![".mounts", ".views", ".appdata", ".requests"].contains(&n.as_str()));
            let here = clean(path);
            for (at, _) in self.mounts() {
                let rest = if here.is_empty() {
                    Some(at.as_str())
                } else {
                    at.strip_prefix(&format!("{here}/"))
                };
                if let Some(first) = rest.and_then(|r| r.split('/').next())
                    && !first.starts_with('.')
                {
                    names.push(first.to_string());
                }
            }
            names.sort();
            names.dedup();
        }
        Ok(names)
    }

    fn list_in(&self, who: PublicKey, space: Hash, name: &str) -> Result<Vec<String>, FsError> {
        let path = name;
        let s = self.node.space(space).ok_or(FsError::NotFound)?;
        let prefix = if name.is_empty() {
            String::new()
        } else {
            format!("{name}/")
        };
        if !s.may(
            Principal::Key(who),
            Permission::FilesList,
            &Resource::Prefix(prefix.clone()),
        ) && !s.may(
            Principal::Key(who),
            Permission::FilesRead,
            &Resource::Prefix(prefix.clone()),
        ) {
            return Err(FsError::Denied(format!("{who} may not list {path}")));
        }
        let names: BTreeSet<String> = s
            .files()
            .iter()
            .flat_map(|f| f.versions.iter().map(|v| v.name.clone()))
            .filter_map(|n| {
                let rest = n.strip_prefix(&prefix)?;
                Some(rest.split('/').next()?.to_string())
            })
            .filter(|n| !n.is_empty())
            .collect();
        Ok(names.into_iter().collect())
    }

    /// The requests of apps that wait for the person.
    fn files_under(&self, path: &str) -> Vec<String> {
        let (space, name) = self.resolve(path);
        let Some(s) = self.node.space(space) else {
            return Vec::new();
        };
        let base = clean(path);
        s.files()
            .iter()
            .flat_map(|f| f.versions.iter().map(|v| v.name.clone()))
            .filter(|n| name.is_empty() || *n == name || n.starts_with(&format!("{name}/")))
            .map(|n| {
                let rest = n[name.len()..].trim_start_matches('/');
                if rest.is_empty() {
                    format!("/{base}")
                } else if base.is_empty() {
                    format!("/{rest}")
                } else {
                    format!("/{base}/{rest}")
                }
            })
            .collect()
    }

    /// The node paths where a name of a space can be seen.
    fn node_paths(&self, space: Hash, name: &str) -> Vec<String> {
        if space == self.home {
            return vec![format!("/{name}")];
        }
        self.mounts()
            .into_iter()
            .filter(|(_, id)| *id == space)
            .map(|(at, _)| format!("/{at}/{name}"))
            .collect()
    }

    /// Keeps the entries of a space that are not on disk yet. After
    /// compaction, writes the space again.
    fn save(&mut self, space: Hash) -> Result<(), Error> {
        let Some(s) = self.node.space(space) else {
            return Ok(());
        };
        let base = s.base().map(|b| b.checkpoint.hash());
        let saved = self.saved.entry(space).or_default();
        if saved.1 != base {
            self.store.save_space(s)?;
            *saved = (s.iter().map(fileos_space::Entry::hash).collect(), base);
            return Ok(());
        }
        let new: Vec<fileos_space::Entry> = s
            .iter()
            .filter(|e| !saved.0.contains(&e.hash()))
            .cloned()
            .collect();
        self.store.append_entries(space, &new)?;
        saved.0.extend(new.iter().map(fileos_space::Entry::hash));
        Ok(())
    }

    /// Finds the names in a space whose newest versions changed since apps
    /// last heard, and notes their node paths (or waits for their content).
    fn note(&mut self, space: Hash, changed: &Changed) {
        let Some(s) = self.node.space(space) else {
            return;
        };
        let mut now: BTreeMap<String, Vec<Id>> = BTreeMap::new();
        let mut contents: BTreeMap<String, Option<Content>> = BTreeMap::new();
        for f in s.files() {
            for v in &f.versions {
                now.entry(v.name.clone()).or_default().push(v.id);
            }
            if let Some(name) = f.versions.first().map(|v| v.name.clone()) {
                contents.insert(name, newest(&f));
            }
        }
        let before = self.seen.remove(&space).unwrap_or_default();
        let names: BTreeSet<&String> = now
            .iter()
            .filter(|(n, ids)| before.get(*n) != Some(*ids))
            .map(|(n, _)| n)
            .chain(before.keys().filter(|n| !now.contains_key(*n)))
            .collect();
        let mut ready = Vec::new();
        for name in names {
            let kind = if !now.contains_key(name) {
                ChangeKind::Deleted
            } else if before.contains_key(name) {
                ChangeKind::Changed
            } else {
                ChangeKind::Made
            };
            let at: Vec<(String, ChangeKind)> = self
                .node_paths(space, name)
                .into_iter()
                .map(|p| (p, kind))
                .collect();
            match contents.get(name).copied().flatten() {
                Some(c) if !fileos_space::content::complete(&c, &self.store) => {
                    self.waiting.entry(c.hash).or_default().extend(at);
                }
                _ => ready.extend(at),
            }
        }
        self.seen.insert(space, now);
        lock(changed).extend(ready);
    }

    /// Keeps what a step made, sends its messages, and notes changed paths.
    fn process(&mut self, step: Step, changed: &Changed) -> Result<(), Error> {
        self.rotate_if_due(changed)?;
        for space in self.apply(step, changed)? {
            self.maybe_checkpoint(space, changed);
        }
        Ok(())
    }

    fn apply(&mut self, step: Step, changed: &Changed) -> Result<BTreeSet<Hash>, Error> {
        let mut spaces = BTreeSet::new();
        let mut out = step.out;
        let mut proof_changed = false;
        let mut home_changed = false;
        let mut mounts_changed = false;
        for event in step.events {
            match event {
                Event::Changed(space) => {
                    self.save(space)?;
                    self.note(space, changed);
                    spaces.insert(space);
                    proof_changed |= self.identity_space() == Some(space);
                    // The home space of the identity came or changed, or a
                    // mount may be readable now.
                    home_changed |= space != self.home && self.identity_home() == Some(space);
                    mounts_changed |= space == self.home;
                }
                Event::Content { from, data } => {
                    let hash = Hash::of(&data);
                    self.store.put_piece(hash, &data)?;
                    out.extend(self.node.content_saved(from, &self.store).out);
                    // A list of chunks or the last chunk came: the paths that
                    // waited for content that is now complete can go.
                    let done: Vec<Hash> = self
                        .waiting
                        .keys()
                        .copied()
                        .filter(|h| {
                            self.store.has(*h)
                                && self.node.spaces().any(|s| {
                                    s.contents().iter().any(|c| {
                                        c.hash == *h
                                            && fileos_space::content::complete(c, &self.store)
                                    })
                                })
                        })
                        .collect();
                    for h in done {
                        let paths = self.waiting.remove(&h).unwrap_or_default();
                        lock(changed).extend(paths);
                    }
                    // The certificate of this node, or the file of a mount,
                    // may be what just came.
                    proof_changed = true;
                    mounts_changed = true;
                }
                Event::Again { space, entries } => {
                    tracing::info!(name: "node.space.rebased", %space, entries, "offline changes were written again");
                    self.log(format!(
                        "{entries} changes made at the same time as a checkpoint of space \
                         {space} were written again on top"
                    ));
                }
            }
        }
        if proof_changed {
            self.update_proof(changed)?;
        }
        if home_changed {
            self.use_identity_home(changed)?;
        }
        if mounts_changed {
            self.follow_mounts(changed)?;
        }
        if let Some(net) = self.net.clone() {
            for (to, msg) in out {
                let content = match &msg {
                    Msg::WantContent(hash) => Some(*hash),
                    _ => None,
                };
                if net
                    .send(fileos_net::Command::Send {
                        to,
                        msg: Box::new(msg),
                    })
                    .is_err()
                {
                    self.send_failed(to, content);
                }
            }
        }
        Ok(spaces)
    }

    fn send_failed(&mut self, to: PublicKey, content: Option<Hash>) {
        if let Some(hash) = content {
            self.node.content_failed(to, hash);
        } else {
            self.net_dirty = true;
        }
    }

    /// The default policy: a node makes a new recipient key (the key it gets
    /// keys with) when it has none or the newest is older than
    /// [`RECIPIENT_MAX_AGE`], keeps it on disk first, and keeps only the one
    /// before it, so the older secrets are destroyed.
    fn rotate_if_due(&mut self, changed: &Changed) -> Result<(), Error> {
        let now = (self.clock)();
        let due = self
            .node
            .recipient_made()
            .is_none_or(|made| now.saturating_sub(made) >= RECIPIENT_MAX_AGE);
        if !due {
            return Ok(());
        }
        let step = self.node.rotate_recipient(now);
        self.store.set_recipients(&self.node.recipients())?;
        self.apply(step, changed).map(drop)
    }

    /// The default policy: a node makes a checkpoint of a space it owns when
    /// the space has many entries, and every node compacts when it can.
    fn maybe_checkpoint(&mut self, space: Hash, changed: &Changed) {
        let Some(s) = self.node.space(space) else {
            return;
        };
        if s.len() < CHECKPOINT_AFTER {
            return;
        }
        let mine = s.owner() == Principal::Key(self.key.public());
        let result = if mine {
            self.checkpoint(space, changed)
        } else {
            let (compacted, step) = self.node.compact(space);
            self.apply(step, changed)
                .and_then(|_| if compacted { self.save(space) } else { Ok(()) })
        };
        if let Err(e) = result {
            tracing::warn!(name: "node.space.checkpoint.failed", %space, error = %e, "space could not be compacted");
        }
    }

    fn log(&mut self, message: String) {
        self.logs.push_back(message);
        while self.logs.len() > 100 {
            self.logs.pop_front();
        }
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        let _ = writeln!(out, "node     {}", self.key);
        let _ = writeln!(out, "home     {}", self.home);
        match &self.identity {
            Some(Identity { did: Some(did), .. }) => {
                let _ = writeln!(out, "identity {did}");
            }
            Some(Identity { space, .. }) => {
                let _ = writeln!(out, "identity (waiting for its space {space})");
            }
            None => {}
        }
        for addr in &self.listening {
            let _ = writeln!(out, "listen   {addr}");
        }
        for peer in &self.peers {
            let _ = writeln!(out, "peer     {peer}");
        }
        for space in &self.spaces {
            let at = if space.mounted_at.is_empty() {
                "-".to_string()
            } else {
                space.mounted_at.join(", ")
            };
            let _ = writeln!(
                out,
                "space    {} at {at}, {} entries",
                space.id, space.entries
            );
            if space.owner != Principal::Key(self.key) {
                let _ = writeln!(out, "           owned by {:?}", space.owner);
            }
            if space.rejected > 0 {
                let _ = writeln!(out, "           {} entries have no effect", space.rejected);
            }
            if let Some(base) = space.base {
                let _ = writeln!(out, "           compacted at {base}");
            }
        }
        for id in &self.wanted {
            let _ = writeln!(out, "waiting  for space {id} from peers");
        }
        // Apps and their messages are not part of the status of the
        // kernel. The app spaces are in /apps, as any space.
        f.write_str(&out)
    }
}
