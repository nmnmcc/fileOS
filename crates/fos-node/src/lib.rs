// SPDX-License-Identifier: AGPL-3.0-only

//! One fileOS node.
//!
//! A node puts the parts together: the data directory ([`Store`]), the
//! folders and sync ([`Replica`]), the engine of the node, the apps, and the
//! network. Paths here are node paths: the first part is a mount in the root
//! folder, for example `/photos/2026/a.jpg`.
//!
//! The node and the people who use it through the gateway act with the node
//! key. Each app acts with its own key, which comes from the node key. Every
//! write, by anyone, goes through the engine.

use std::collections::{BTreeMap, VecDeque};
use std::fmt::{self, Write as _};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, Weak};

use fos_core::{Bytes, Error, Hash, PublicKey, SecretKey};
use fos_fs::{Ability, Authz, Decision, Fork, Node as FsNode, Op, resolve};
use fos_runtime::{AppHost, AppInstance, Bundle, EngineInstance, FsError, Kind, Runtime};
use fos_store::Store;
use fos_sync::{Blobs as _, Event, Replica, Step};
use tokio::sync::mpsc;

pub use fos_net::Config as NetConfig;

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
    /// Node paths that changed, for the apps that watch them.
    changed: Mutex<VecDeque<String>>,
    runtime: Runtime,
    key: PublicKey,
}

struct Inner {
    store: Store,
    key: SecretKey,
    replica: Replica,
    engine: EngineInstance,
    root: Hash,
    kv: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
    net: Option<mpsc::UnboundedSender<fos_net::Command>>,
    listening: Vec<String>,
    /// Node paths that wait for a blob before apps hear about them.
    waiting: BTreeMap<Hash, Vec<String>>,
    /// The last messages from apps, newest last.
    logs: VecDeque<String>,
}

/// What a node is doing, for people.
#[derive(Clone, Debug)]
pub struct Status {
    pub key: PublicKey,
    pub root: Hash,
    pub listening: Vec<String>,
    pub peers: Vec<PublicKey>,
    pub pending: usize,
    pub folders: Vec<FolderStatus>,
    pub apps: Vec<String>,
    pub logs: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct FolderStatus {
    pub id: Hash,
    pub name: String,
    pub mounted_at: Vec<String>,
    pub entries: u64,
    pub orderer: PublicKey,
    pub fork: Option<Fork>,
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn join(mount: &str, inner: &str) -> String {
    if inner == "/" { mount.to_string() } else { format!("{mount}{inner}") }
}

fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

/// The abilities that a permission in a manifest asks for.
fn abilities(access: &str) -> Result<Vec<Ability>, Error> {
    Ok(match access {
        "read" => vec![Ability::Read],
        "write" => vec![Ability::Write],
        "append" => vec![Ability::Append],
        "read-write" => vec![Ability::Read, Ability::Write, Ability::Append],
        _ => return Err(Error(format!("not an access: {access:?}"))),
    })
}

/// The token that local clients give to the gateway of the node in `dir`.
///
/// # Errors
///
/// When the data directory cannot be read.
pub fn token(dir: &Path) -> Result<String, Error> {
    Store::open(dir)?.token()
}

/// Writes the address of the running gateway into `dir`, or takes it away.
///
/// # Errors
///
/// When the file cannot be written.
pub fn write_gateway_addr(dir: &Path, addr: Option<std::net::SocketAddr>) -> Result<(), Error> {
    let path = dir.join("gateway.addr");
    match addr {
        Some(addr) => fos_store::write_atomic(&path, addr.to_string().as_bytes()),
        None => std::fs::remove_file(&path).or(Ok(())),
    }
}

/// Reads ability names, for example `read,write,store`.
///
/// # Errors
///
/// When a name is not an ability.
pub fn parse_abilities(list: &str) -> Result<Vec<Ability>, Error> {
    list.split(',')
        .map(|name| match name.trim() {
            "read" => Ok(vec![Ability::Read]),
            "write" => Ok(vec![Ability::Write]),
            "append" => Ok(vec![Ability::Append]),
            "grant" => Ok(vec![Ability::Grant]),
            "store" => Ok(vec![Ability::Store]),
            "all" => Ok(vec![Ability::Read, Ability::Write, Ability::Append, Ability::Grant, Ability::Store]),
            other => Err(Error(format!("not an ability: {other:?}"))),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|lists| lists.concat())
}

impl Node {
    /// Makes a new node in `dir`, with the engine in the `.app` folder `engine`.
    ///
    /// # Errors
    ///
    /// When the directory already has a node, or the engine does not start.
    pub fn init(dir: &Path, engine: &Path) -> Result<PublicKey, Error> {
        let store = Store::open(dir)?;
        if store.root()?.is_some() {
            return Err(Error(format!("{} already has a node", dir.display())));
        }
        let bundle = Bundle::read_dir(engine)?;
        if bundle.manifest.kind != Kind::Engine {
            return Err(Error(format!("{} is not an engine", engine.display())));
        }
        copy_dir(engine, &store.engine_dir()).map_err(|e| Error(format!("cannot copy the engine: {e}")))?;
        let key = store.key()?;
        let mut authz = Runtime::new()?.engine(&bundle)?;
        let mut replica = Replica::new(key.clone(), 0);
        let (root, step) = replica.create("root", &mut authz);
        for event in step.events {
            if let Event::Entries { folder, entries } = event {
                store.append_entries(folder, &entries)?;
            }
        }
        store.set_root(root)?;
        Ok(key.public())
    }

    /// Opens the node in `dir`, and starts its apps.
    ///
    /// # Errors
    ///
    /// When there is no node in `dir`, or its data or engine is broken.
    pub fn open(dir: &Path) -> Result<Node, Error> {
        let store = Store::open(dir)?;
        let root = store.root()?.ok_or_else(|| Error(format!("{} has no node: run `fos init`", dir.display())))?;
        let key = store.key()?;
        let runtime = Runtime::new()?;
        let mut engine = runtime.engine(&Bundle::read_dir(&store.engine_dir())?)?;
        let seed = fos_store::random_seed()?;
        let mut replica = Replica::new(key.clone(), u64::from_le_bytes(seed[..8].try_into().unwrap_or_default()));
        for folder in store.load_folders(&mut engine)? {
            replica.insert(folder);
        }
        let inner = Inner {
            store,
            key: key.clone(),
            replica,
            engine,
            root,
            kv: BTreeMap::new(),
            net: None,
            listening: Vec::new(),
            waiting: BTreeMap::new(),
            logs: VecDeque::new(),
        };
        let node = Node {
            shared: Arc::new(Shared {
                inner: Mutex::new(inner),
                apps: Mutex::new(Vec::new()),
                changed: Mutex::new(VecDeque::new()),
                runtime,
                key: key.public(),
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

    /// Makes a new folder, and mounts it at `at`. Gives its ID.
    ///
    /// # Errors
    ///
    /// When the mount is not allowed.
    pub fn create(&self, name: &str, at: &str) -> Result<Hash, Error> {
        let mut inner = self.inner();
        let Inner { replica, engine, .. } = &mut *inner;
        let (id, step) = replica.create(name, engine);
        inner.process(step, &self.shared.changed)?;
        inner.mount(id, at, &self.shared.changed)?;
        Ok(id)
    }

    /// Mounts a folder at `at`, and starts to get it from peers.
    ///
    /// # Errors
    ///
    /// When the mount is not allowed.
    pub fn mount(&self, folder: Hash, at: &str) -> Result<(), Error> {
        let mut inner = self.inner();
        inner.mount(folder, at, &self.shared.changed)?;
        let step = inner.replica.follow(folder);
        inner.process(step, &self.shared.changed)
    }

    /// Puts a file.
    ///
    /// # Errors
    ///
    /// When the path is not valid, or the engine says no.
    pub fn put(&self, path: &str, data: &[u8]) -> Result<(), Error> {
        let result = self.inner().put(None, path, data, &self.shared.changed);
        self.pump();
        result
    }

    /// Adds a record to the end of a stream.
    ///
    /// # Errors
    ///
    /// When the path is not valid, or the engine says no.
    pub fn append(&self, path: &str, data: &[u8]) -> Result<(), Error> {
        let result = self.inner().write_at(None, path, |p| Op::Append { path: p, data: Bytes(data.to_vec()) }, &self.shared.changed);
        self.pump();
        result
    }

    /// Takes a file or mount away.
    ///
    /// # Errors
    ///
    /// When the path is not there, or the engine says no.
    pub fn delete(&self, path: &str) -> Result<(), Error> {
        let result = self.inner().write_at(None, path, |p| Op::Delete { path: p }, &self.shared.changed);
        self.pump();
        result
    }

    /// The bytes of a file, or the records of a stream one after the other.
    ///
    /// # Errors
    ///
    /// When the path is not there, or the engine says no.
    pub fn read(&self, path: &str) -> Result<Vec<u8>, Error> {
        let key = self.key();
        self.inner().read(key, path).map_err(fs_error)
    }

    /// The names directly under a path.
    ///
    /// # Errors
    ///
    /// When the folder is not here, or the engine says no.
    pub fn list(&self, path: &str) -> Result<Vec<String>, Error> {
        let key = self.key();
        self.inner().list(key, path).map_err(fs_error)
    }

    /// Writes a policy entry into the folder that has `path`. The engine gives it meaning.
    ///
    /// # Errors
    ///
    /// When the engine does not take it.
    pub fn policy(&self, path: &str, policy: &[u8]) -> Result<(), Error> {
        self.inner().write_at(None, path, |_| Op::Policy(Bytes(policy.to_vec())), &self.shared.changed)
    }

    /// Gives `to` abilities on `path`, in the form the engine asks for.
    ///
    /// # Errors
    ///
    /// When the engine does not take it.
    pub fn grant(&self, path: &str, to: PublicKey, abilities: &[Ability]) -> Result<(), Error> {
        self.inner().grant(path, to, abilities, &self.shared.changed)
    }

    /// Installs an app from an `.app` directory: puts it into a folder at
    /// `/apps/<name>`, gives its key the permissions in its manifest, and
    /// starts it. Gives the app's key.
    ///
    /// # Errors
    ///
    /// When the bundle is not an app, a permission cannot be given, or the app
    /// does not start.
    pub fn install(&self, dir: &Path) -> Result<PublicKey, Error> {
        let bundle = Bundle::read_dir(dir)?;
        if bundle.manifest.kind != Kind::App {
            return Err(Error(format!("{} is not an app", dir.display())));
        }
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let at = format!("/apps/{name}");
        let app_key;
        {
            let mut inner = self.inner();
            app_key = inner.app_key(&bundle.manifest.id);
            if inner.resolve(&at).is_ok_and(|(f, p)| p == "/" && f != inner.root) {
                return Err(Error(format!("{at} is already there")));
            }
            let Inner { replica, engine, .. } = &mut *inner;
            let (id, step) = replica.create(&name, engine);
            inner.process(step, &self.shared.changed)?;
            inner.mount(id, &at, &self.shared.changed)?;
            for (path, data) in &bundle.files {
                inner.put(None, &format!("{at}{path}"), data, &self.shared.changed)?;
            }
            for permission in &bundle.manifest.permissions {
                inner.grant(&permission.path, app_key.public(), &abilities(&permission.access)?, &self.shared.changed)?;
            }
        }
        self.start_app(&bundle)?;
        // Tell the new app about the files that are there already.
        let mut existing = Vec::new();
        {
            let inner = self.inner();
            for watch in &bundle.manifest.watch {
                existing.extend(inner.files_under(&watch.path));
            }
        }
        lock(&self.shared.changed).extend(existing);
        self.pump();
        Ok(app_key.public())
    }

    /// The local key-value store of an app.
    pub fn kv(&self, app: &str) -> BTreeMap<String, Vec<u8>> {
        self.inner().kv(app).clone()
    }

    pub fn status(&self) -> Status {
        let apps = lock(&self.shared.apps).iter().map(|a| a.manifest().id.clone()).collect();
        let mut inner = self.inner();
        let root_mounts: Vec<(String, Hash)> = inner
            .replica
            .folder(inner.root)
            .map(|r| r.mounts().map(|(p, id)| (p.to_string(), id)).collect())
            .unwrap_or_default();
        let folders = inner
            .replica
            .folders()
            .map(|f| FolderStatus {
                id: f.id(),
                name: f.name().to_string(),
                mounted_at: root_mounts.iter().filter(|(_, id)| *id == f.id()).map(|(p, _)| p.clone()).collect(),
                entries: f.next_seq(),
                orderer: f.orderer(),
                fork: f.fork().cloned(),
            })
            .collect();
        let peers = inner.replica.peers().copied().collect();
        let pending = inner.replica.pending().len();
        let logs = inner.logs.drain(..).collect();
        Status { key: self.key(), root: inner.root, listening: inner.listening.clone(), peers, pending, folders, apps, logs }
    }

    /// Starts the network. Call it inside a Tokio runtime.
    ///
    /// # Errors
    ///
    /// When the network does not start.
    pub fn start_net(&self, config: NetConfig) -> Result<(), Error> {
        let seed = self.inner().key.seed();
        let (commands, mut events) = fos_net::start(seed, config)?;
        self.inner().net = Some(commands);
        let node = self.clone();
        tokio::spawn(async move {
            while let Some(event) = events.recv().await {
                node.on_net(event);
            }
        });
        Ok(())
    }

    /// Connects to another node.
    ///
    /// # Errors
    ///
    /// When the network is not started.
    pub fn dial(&self, addr: &str) -> Result<(), Error> {
        let addr = addr.parse().map_err(|e| Error(format!("not a multiaddr: {e}")))?;
        let inner = self.inner();
        let net = inner.net.as_ref().ok_or_else(|| Error("the network is not started".into()))?;
        net.send(fos_net::Command::Dial(addr)).map_err(|_| Error("the network stopped".into()))
    }

    fn on_net(&self, event: fos_net::Event) {
        {
            let mut inner = self.inner();
            let Inner { replica, engine, store, .. } = &mut *inner;
            let step = match event {
                fos_net::Event::Listening(addr) => {
                    inner.listening.push(format!("{addr}/p2p/{}", fos_net::peer_id(self.key()).map(|p| p.to_string()).unwrap_or_default()));
                    return;
                }
                fos_net::Event::Connected(peer) => replica.connected(peer, engine),
                fos_net::Event::Disconnected(peer) => {
                    replica.disconnected(peer);
                    return;
                }
                fos_net::Event::Message { from, msg } => replica.handle(from, msg, engine, store),
            };
            if let Err(e) = inner.process(step, &self.shared.changed) {
                tracing::warn!("{e}");
            }
        }
        self.pump();
    }

    fn start_apps(&self) {
        let bundles = {
            let inner = self.inner();
            let Some(root) = inner.replica.folder(inner.root) else { return };
            let mounts: Vec<Hash> =
                root.mounts().filter(|(p, _)| p.starts_with("/apps/")).map(|(_, id)| id).collect();
            mounts.into_iter().filter_map(|id| inner.bundle(id)).collect::<Vec<_>>()
        };
        for bundle in bundles {
            if let Err(e) = self.start_app(&bundle) {
                tracing::warn!("app {} did not start: {e}", bundle.manifest.id);
            }
        }
    }

    fn start_app(&self, bundle: &Bundle) -> Result<(), Error> {
        let host = Arc::new(Host { shared: Arc::downgrade(&self.shared) });
        let app = self.shared.runtime.app(bundle, host)?;
        lock(&self.shared.apps).push(app);
        Ok(())
    }

    /// Tells the apps about changed paths, until there are no more.
    pub fn pump(&self) {
        pump(&self.shared);
    }
}

fn pump(shared: &Shared) {
    loop {
        let Some(path) = lock(&shared.changed).pop_front() else { return };
        let mut apps = lock(&shared.apps);
        for app in apps.iter_mut().filter(|a| a.watches(&path)) {
            if let Err(e) = app.on_change(&path) {
                tracing::warn!("app {} failed on {path}: {e}", app.manifest().id);
            }
        }
    }
}

fn fs_error(e: FsError) -> Error {
    match e {
        FsError::Denied(m) => Error(format!("not allowed: {m}")),
        FsError::NotFound => Error("not found".into()),
        FsError::Invalid(m) => Error(m),
    }
}

impl Inner {
    fn app_key(&self, app: &str) -> SecretKey {
        self.key.derive(&format!("app:{app}"))
    }

    fn resolve(&self, path: &str) -> Result<(Hash, String), Error> {
        resolve(self.root, path, |id| self.replica.folder(id))
    }

    fn mount(&mut self, folder: Hash, at: &str, changed: &Mutex<VecDeque<String>>) -> Result<(), Error> {
        let key = self.key.clone();
        let step = self.replica.write(self.root, Op::Mount { path: at.into(), folder }, &key, &mut self.engine)?;
        self.process(step, changed)
    }

    /// Writes into the folder that has `path`, as `author` (the node when `None`).
    fn write_at(
        &mut self,
        author: Option<&SecretKey>,
        path: &str,
        op: impl FnOnce(String) -> Op,
        changed: &Mutex<VecDeque<String>>,
    ) -> Result<(), Error> {
        let (folder, inner) = self.resolve(path)?;
        let author = author.cloned().unwrap_or_else(|| self.key.clone());
        let step = self.replica.write(folder, op(inner), &author, &mut self.engine)?;
        self.process(step, changed)
    }

    fn put(&mut self, author: Option<&SecretKey>, path: &str, data: &[u8], changed: &Mutex<VecDeque<String>>) -> Result<(), Error> {
        let blob = self.store.put_blob(data)?;
        let size = data.len() as u64;
        self.write_at(author, path, |path| Op::Put { path, blob, size }, changed)
    }

    fn grant(&mut self, path: &str, to: PublicKey, abilities: &[Ability], changed: &Mutex<VecDeque<String>>) -> Result<(), Error> {
        let (_, inner) = self.resolve(path)?;
        let policy = self.engine.grant(to, abilities, &inner);
        self.write_at(None, path, |_| Op::Policy(Bytes(policy)), changed)
    }

    fn check(&mut self, principal: PublicKey, ability: Ability, path: &str) -> Result<(Hash, String), FsError> {
        let (folder, inner) = self.resolve(path).map_err(|e| FsError::Invalid(e.0))?;
        let f = self.replica.folder(folder).ok_or(FsError::NotFound)?;
        match f.check(&mut self.engine, principal, ability, &inner) {
            Decision::Allow => Ok((folder, inner)),
            Decision::Deny(reason) => Err(FsError::Denied(reason)),
        }
    }

    fn read(&mut self, principal: PublicKey, path: &str) -> Result<Vec<u8>, FsError> {
        let (folder, inner) = self.check(principal, Ability::Read, path)?;
        let f = self.replica.folder(folder).ok_or(FsError::NotFound)?;
        match f.get(&inner) {
            Some(FsNode::File(_)) => {
                let (hash, _) = f.file(&inner).ok_or(FsError::NotFound)?;
                self.store.get(hash).ok_or_else(|| FsError::Invalid("the file is not on this node yet".into()))
            }
            Some(FsNode::Stream(records)) => Ok(records.iter().flat_map(|r| r.data.iter().copied()).collect()),
            Some(FsNode::Mount(_)) | None => Err(FsError::NotFound),
        }
    }

    fn list(&mut self, principal: PublicKey, path: &str) -> Result<Vec<String>, FsError> {
        let (folder, inner) = self.check(principal, Ability::Read, path)?;
        Ok(self.replica.folder(folder).ok_or(FsError::NotFound)?.list(&inner))
    }

    fn kv(&mut self, app: &str) -> &mut BTreeMap<String, Vec<u8>> {
        let store = &self.store;
        self.kv.entry(app.to_string()).or_insert_with(|| store.kv_load(app))
    }

    /// The node paths of every file under a node path.
    fn files_under(&self, path: &str) -> Vec<String> {
        let Ok((folder, inner)) = self.resolve(path) else { return Vec::new() };
        let Some(f) = self.replica.folder(folder) else { return Vec::new() };
        let prefix = if inner == "/" { String::from("/") } else { format!("{inner}/") };
        f.files()
            .filter(|(p, _, _)| inner == "/" || *p == inner || p.starts_with(&prefix))
            .map(|(p, _, _)| join(path.trim_end_matches('/'), p))
            .collect()
    }

    /// The node paths where a path of a folder can be seen.
    fn node_paths(&self, folder: Hash, path: &str) -> Vec<String> {
        if folder == self.root {
            return vec![path.to_string()];
        }
        let Some(root) = self.replica.folder(self.root) else { return Vec::new() };
        root.mounts().filter(|(_, id)| *id == folder).map(|(at, _)| join(at, path)).collect()
    }

    /// The files of an app folder, as a bundle.
    fn bundle(&self, folder: Hash) -> Option<Bundle> {
        let f = self.replica.folder(folder)?;
        let files = f.files().map(|(p, h, _)| Some((p.to_string(), self.store.get(h)?))).collect::<Option<_>>()?;
        Bundle::from_files(files).ok()
    }

    /// Keeps what a step made, sends its messages, and notes changed paths.
    fn process(&mut self, step: Step, changed: &Mutex<VecDeque<String>>) -> Result<(), Error> {
        if let Some(net) = &self.net {
            for out in step.out {
                let _ = net.send(fos_net::Command::Send { to: out.to, msg: out.msg });
            }
        }
        let mut paths = Vec::new();
        for event in step.events {
            match event {
                Event::Added(_) => {}
                Event::Entries { folder, entries } => {
                    self.store.append_entries(folder, &entries)?;
                    for entry in &entries {
                        let (path, blob) = match &entry.write.write.op {
                            Op::Put { path, blob, .. } => (path, Some(*blob)),
                            Op::Delete { path } | Op::Append { path, .. } => (path, None),
                            _ => continue,
                        };
                        let at = self.node_paths(folder, path);
                        match blob {
                            Some(blob) if !self.store.has(blob) => self.waiting.entry(blob).or_default().extend(at),
                            _ => paths.extend(at),
                        }
                    }
                }
                Event::Forked { folder, fork } => {
                    tracing::warn!("folder {folder} forked at entry {}: {}", fork.seq, fork.reason);
                }
                Event::Refused { write, reason } => {
                    tracing::warn!("the ordering node did not take write {write}: {reason}");
                }
                Event::Blob(data) => {
                    let hash = self.store.put_blob(&data)?;
                    paths.extend(self.waiting.remove(&hash).unwrap_or_default());
                }
            }
        }
        lock(changed).extend(paths);
        Ok(())
    }
}

/// What an app may call. Every call checks with the engine, as the app's key.
struct Host {
    shared: Weak<Shared>,
}

impl Host {
    fn with<T>(&self, f: impl FnOnce(&mut Inner, &Mutex<VecDeque<String>>) -> T) -> Option<T> {
        let shared = self.shared.upgrade()?;
        let mut inner = lock(&shared.inner);
        Some(f(&mut inner, &shared.changed))
    }

    fn write(&self, app: &str, path: &str, op: impl FnOnce(String) -> Op) -> Result<(), FsError> {
        self.with(|inner, changed| {
            let key = inner.app_key(app);
            inner.write_at(Some(&key), path, op, changed).map_err(|e| FsError::Denied(e.0))
        })
        .unwrap_or(Err(FsError::Invalid("the node stopped".into())))
    }
}

impl AppHost for Host {
    fn read(&self, app: &str, path: &str) -> Result<Vec<u8>, FsError> {
        self.with(|inner, _| {
            let key = inner.app_key(app).public();
            inner.read(key, path)
        })
        .unwrap_or(Err(FsError::Invalid("the node stopped".into())))
    }

    fn write(&self, app: &str, path: &str, data: Vec<u8>) -> Result<(), FsError> {
        let blob = self
            .with(|inner, _| inner.store.put_blob(&data))
            .unwrap_or_else(|| Err(Error("the node stopped".into())))
            .map_err(|e| FsError::Invalid(e.0))?;
        let size = data.len() as u64;
        Host::write(self, app, path, |path| Op::Put { path, blob, size })
    }

    fn append(&self, app: &str, path: &str, data: Vec<u8>) -> Result<(), FsError> {
        Host::write(self, app, path, |path| Op::Append { path, data: Bytes(data) })
    }

    fn delete(&self, app: &str, path: &str) -> Result<(), FsError> {
        Host::write(self, app, path, |path| Op::Delete { path })
    }

    fn list(&self, app: &str, path: &str) -> Result<Vec<String>, FsError> {
        self.with(|inner, _| {
            let key = inner.app_key(app).public();
            inner.list(key, path)
        })
        .unwrap_or(Err(FsError::Invalid("the node stopped".into())))
    }

    fn kv_get(&self, app: &str, key: &str) -> Option<Vec<u8>> {
        self.with(|inner, _| inner.kv(app).get(key).cloned()).flatten()
    }

    fn kv_set(&self, app: &str, key: &str, value: Vec<u8>) {
        self.with(|inner, _| {
            inner.kv(app).insert(key.to_string(), value);
            let map = inner.kv(app).clone();
            if let Err(e) = inner.store.kv_save(app, &map) {
                tracing::warn!("{e}");
            }
        });
    }

    fn kv_delete(&self, app: &str, key: &str) {
        self.with(|inner, _| {
            if inner.kv(app).remove(key).is_some() {
                let map = inner.kv(app).clone();
                if let Err(e) = inner.store.kv_save(app, &map) {
                    tracing::warn!("{e}");
                }
            }
        });
    }

    fn kv_keys(&self, app: &str, prefix: &str) -> Vec<String> {
        self.with(|inner, _| inner.kv(app).keys().filter(|k| k.starts_with(prefix)).cloned().collect())
            .unwrap_or_default()
    }

    fn log(&self, app: &str, message: &str) {
        tracing::info!("[{app}] {message}");
        self.with(|inner, _| {
            inner.logs.push_back(format!("[{app}] {message}"));
            while inner.logs.len() > 100 {
                inner.logs.pop_front();
            }
        });
    }
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        let _ = writeln!(out, "node     {}", self.key);
        let _ = writeln!(out, "root     {}", self.root);
        for addr in &self.listening {
            let _ = writeln!(out, "listen   {addr}");
        }
        for peer in &self.peers {
            let _ = writeln!(out, "peer     {peer}");
        }
        let _ = writeln!(out, "waiting  {} writes for their ordering node", self.pending);
        for folder in &self.folders {
            let at = if folder.mounted_at.is_empty() { "-".to_string() } else { folder.mounted_at.join(", ") };
            let _ = writeln!(out, "folder   {} {:?} at {at}, {} entries", folder.id, folder.name, folder.entries);
            if folder.orderer != self.key {
                let _ = writeln!(out, "           ordered by {}", folder.orderer);
            }
            if let Some(fork) = &folder.fork {
                let _ = writeln!(out, "           FORK at entry {}: {}", fork.seq, fork.reason);
            }
        }
        for app in &self.apps {
            let _ = writeln!(out, "app      {app}");
        }
        for log in &self.logs {
            let _ = writeln!(out, "log      {log}");
        }
        f.write_str(&out)
    }
}
