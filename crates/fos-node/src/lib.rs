// SPDX-License-Identifier: AGPL-3.0-only

//! One fileOS node.
//!
//! A node puts the parts together: the data directory ([`Store`]), the
//! spaces and sync ([`fos_space::Node`]), the apps, and the network.
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
//! `did:fos` DID). The identity space is mounted at [`IDENTITY_AT`]. It has
//! the DID log and a certificate for each node. It is owned by the signing
//! key of the identity, so only the node that holds that key can add or take
//! away nodes.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt::{self, Write as _};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{SystemTime, UNIX_EPOCH};

use fos_core::{Error, Hash, PublicKey, SecretKey};
use fos_id::{Link, NodeCert, Op as DidOp, Service, State};
use fos_runtime::{AppHost, AppInstance, Bundle, FsError, Kind, Runtime};
use fos_space::{
    Act, Content, Contents as _, Effect, Event, File, Id, Permission, Resource, Retention, Space,
    Step,
};
use fos_store::Store;
use tokio::sync::mpsc;

pub use fos_net::Config as NetConfig;
pub use fos_runtime::{Answer, HttpApp};
pub use fos_space::Principal;

/// Where the identity space is mounted in the home space.
pub const IDENTITY_AT: &str = "/identity";

/// The prefix of the files in the home space that say what is mounted where.
const MOUNTS: &str = ".mounts/";

/// The prefix of the files in the home space that say what each app sees:
/// `.views/<app>/<name>` holds the space, the prefix in it, and the grant.
const VIEWS: &str = ".views/";

/// Where the private data space of each app is mounted in the home space.
const APPDATA: &str = ".appdata/";

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

/// After this many entries, a node makes a checkpoint of a space it owns and
/// compacts it. A policy, with a default.
const CHECKPOINT_AFTER: usize = 1000;

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
    changed: Mutex<VecDeque<String>>,
    /// The schedules of each app, by app ID.
    schedules: Mutex<BTreeMap<String, Vec<fos_runtime::Schedule>>>,
    /// True once `start_schedules` ran: apps started later get their
    /// timers at once.
    scheduling: std::sync::atomic::AtomicBool,
    runtime: Runtime,
    key: PublicKey,
}

struct Inner {
    store: Store,
    key: SecretKey,
    node: fos_space::Node,
    home: Hash,
    kv: BTreeMap<String, BTreeMap<String, Vec<u8>>>,
    net: Option<mpsc::UnboundedSender<fos_net::Command>>,
    listening: Vec<String>,
    /// For each space: the entries kept on disk, and the checkpoint of the
    /// base that is on disk.
    saved: BTreeMap<Hash, (BTreeSet<Hash>, Option<Hash>)>,
    /// For each space: the newest versions of each name, as apps last heard.
    seen: BTreeMap<Hash, BTreeMap<String, Vec<Id>>>,
    /// Node paths that wait for content before apps hear about them.
    waiting: BTreeMap<Hash, Vec<String>>,
    /// The last messages from apps and the kernel, newest last.
    logs: VecDeque<String>,
    /// The rotation key of the identity, when this node holds it.
    rotation_key: Option<SecretKey>,
    /// The signing key of the identity, when this node holds it.
    signing_key: Option<SecretKey>,
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

/// The identity of a node.
#[derive(Clone, Debug)]
pub struct Identity {
    /// The DID of the identity. `None` while the node waits for the
    /// identity space from another node, or when its log is not valid.
    pub did: Option<String>,
    /// The identity space.
    pub space: Hash,
    /// The rotation keys, as `did:key`, the first one with the highest priority.
    pub rotation_keys: Vec<String>,
    /// Other names of the identity.
    pub also_known_as: Vec<String>,
    /// This node holds a rotation key of the identity.
    pub holds_rotation_key: bool,
    /// This node holds the signing key of the identity, so it can add nodes.
    pub holds_signing_key: bool,
    /// The nodes of the identity: the ones with a good certificate.
    pub nodes: Vec<PublicKey>,
    /// The link that people give to others, with the addresses of this node
    /// as hints.
    pub link: Option<String>,
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
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
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

/// Packs the `.app` folder `dir` into the bytes of an `.app` file: a zip
/// with no compression, with `manifest.toml` as it is and the other files in
/// `payload.tar.zst`.
///
/// # Errors
///
/// When the folder is not a good bundle.
pub fn pack_app(dir: &Path) -> Result<Vec<u8>, Error> {
    Bundle::read_dir(dir)?.pack()
}

/// The ID of an app: the host name of its origin, `<app-id>.localhost`.
///
/// It is the ID in the manifest (only lowercase letters, digits and `-`)
/// and the start of the ID of the app's space. So it does not change, and is
/// not used again by another app, which would get the data that the browser
/// keeps for the origin.
pub fn app_id(manifest_id: &str, space: Hash) -> String {
    let name: String = manifest_id
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .take(40)
        .collect();
    let name = name.trim_matches('-');
    let short = &space.to_string()[..10];
    if name.is_empty() {
        format!("app-{short}")
    } else {
        format!("{name}-{short}")
    }
}

/// An app that was installed.
#[derive(Clone, Debug)]
pub struct Installed {
    /// The key that the app acts with.
    pub key: PublicKey,
    /// The ID of the app: its origin is `<id>.localhost`.
    pub id: String,
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
/// With `None`, the files go. Next to it is `gateway.curl`, a curl config file that `fos curl`
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
    fos_store::write_atomic(
        &curl,
        format!(
            "# Made by `fos node run` for `fos curl`. It goes when the node stops.\n\
             header = \"Authorization: Bearer {token}\"\n\
             connect-to = \"::127.0.0.1:{port}\"\n"
        )
        .as_bytes(),
    )?;
    fos_store::write_atomic(&addr, format!("127.0.0.1:{port}").as_bytes())
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
                let permission = fos_core::decode::<Permission>(&fos_core::encode(&other))
                    .map_err(|e| Error(format!("not a permission or template: {other:?} ({e})")))?;
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
    pub fn init(dir: &Path) -> Result<PublicKey, Error> {
        let store = Store::open(dir)?;
        if store.home()?.is_some() {
            return Err(Error(format!("{} already has a node", dir.display())));
        }
        let key = store.key()?;
        let seed = fos_store::random_seed()?;
        let (space, genesis) = Space::create(
            Principal::Key(key.public()),
            &key,
            now(),
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
    pub fn open(dir: &Path) -> Result<Node, Error> {
        let store = Store::open(dir)?;
        let home = store.home()?.ok_or_else(|| {
            Error(format!(
                "{} has no node: run `fos node init`",
                dir.display()
            ))
        })?;
        let key = store.key()?;
        let rotation_key = store.identity_key("rotation")?;
        let signing_key = store.identity_key("signing")?;
        let seed = fos_store::random_seed()?;
        let mut node = fos_space::Node::new(
            key.clone(),
            u64::from_le_bytes(seed[..8].try_into().unwrap_or_default()),
        );
        let mut saved = BTreeMap::new();
        for space in store.load_spaces()? {
            let hashes = space.entries().iter().map(fos_space::Entry::hash).collect();
            let base = space.base().map(|b| b.checkpoint.hash());
            saved.insert(space.id(), (hashes, base));
            node.insert(space);
        }
        let mut inner = Inner {
            store,
            key: key.clone(),
            node,
            home,
            kv: BTreeMap::new(),
            net: None,
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
        let node = Node {
            shared: Arc::new(Shared {
                inner: Mutex::new(inner),
                apps: Mutex::new(Vec::new()),
                http: Mutex::new(BTreeMap::new()),
                changed: Mutex::new(VecDeque::new()),
                schedules: Mutex::new(BTreeMap::new()),
                scheduling: std::sync::atomic::AtomicBool::new(false),
                runtime: Runtime::new()?,
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

    /// Makes a new space, owned by this node, and mounts it at `at`. Gives
    /// its ID.
    ///
    /// # Errors
    ///
    /// When something is mounted at `at` already.
    pub fn create(&self, at: &str) -> Result<Hash, Error> {
        let mut inner = self.inner();
        let (id, step) = inner.node.create(now());
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
        let key = self.key();
        self.inner().read(key, path).map_err(fs_error)
    }

    /// The names directly under a path.
    ///
    /// # Errors
    ///
    /// When the space is not here, or the node has no grant for it.
    pub fn list(&self, path: &str) -> Result<Vec<String>, Error> {
        let key = self.key();
        self.inner().list(key, path).map_err(fs_error)
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
                format!(
                    "{} {:?} -> {:?} {:?} on {:?}{}",
                    grant_text(g_.id),
                    g_.effect,
                    principal_text(g_.to),
                    g_.permissions,
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

    /// Installs an app from an `.app` file (or an `.app` folder): puts it into a space mounted
    /// at `/apps/<name>`, gives its key the permissions in its manifest, and
    /// starts it. Gives the app's key and ID.
    ///
    /// # Errors
    ///
    /// When the bundle is not an app, a permission cannot be given, or the app
    /// does not start.
    pub fn install(&self, dir: &Path) -> Result<Installed, Error> {
        let bundle = Bundle::read(dir)?;
        if bundle.manifest.kind != Kind::App {
            return Err(Error(format!("{} is not an app", dir.display())));
        }
        let name = dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let at = format!("/apps/{name}");
        let app_key;
        let space;
        {
            let mut inner = self.inner();
            let app = bundle.manifest.id.clone();
            app_key = inner.app_key(&app);
            let (id, step) = inner.node.create(now());
            space = id;
            inner.process(step, &self.shared.changed)?;
            inner.mount(id, &at, &self.shared.changed)?;
            for (path, data) in &bundle.files {
                inner.put(None, &format!("{at}{path}"), data, &self.shared.changed)?;
            }
            // The private data of the app: a space that its key owns, so no
            // other app and no person has access to it unless given.
            if inner.app_data(&app).is_none() {
                let (data, step) = inner.node.create_as(&app_key, now());
                inner.process(step, &self.shared.changed)?;
                inner.mount(data, &format!("{APPDATA}{app}"), &self.shared.changed)?;
            }
            // Each permission in the manifest is mounted in the app's view at
            // the name the manifest gives.
            for permission in &bundle.manifest.permissions {
                inner.give_app(
                    &app,
                    &permission.path,
                    &parse_permissions(&permission.access)?,
                    &clean(&permission.path),
                    &self.shared.changed,
                )?;
            }
        }
        let id = self.start_app(&bundle, space)?;
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
        Ok(Installed {
            key: app_key.public(),
            id,
        })
    }

    /// Gives an installed app (by the ID in its manifest) permissions on a
    /// node path, and mounts it in the app's view at `name`, or at
    /// `granted/<handle>` with no name. A mount at the same name is taken
    /// back first. Gives the name in the app's view.
    ///
    /// # Errors
    ///
    /// When the path is not there, or the node may not give the permissions.
    pub fn app_grant(
        &self,
        app: &str,
        path: &str,
        permissions: &[Permission],
        name: Option<&str>,
    ) -> Result<String, Error> {
        let mut inner = self.inner();
        let name = if let Some(n) = name {
            clean(n)
        } else {
            let (space, prefix) = inner.resolve(path);
            let handle = Hash::of_value(&(app, space, &prefix)).to_string();
            format!("granted/{}", &handle[..12])
        };
        if inner.views(app).iter().any(|v| v.name == name) {
            inner.take_app(app, &name, &self.shared.changed)?;
        }
        inner.give_app(app, path, permissions, &name, &self.shared.changed)?;
        // The app hears of the files that are there already.
        let existing = inner.files_under(path);
        drop(inner);
        lock(&self.shared.changed).extend(existing);
        self.pump();
        Ok(format!("/{name}"))
    }

    /// What an app sees: the mounts in its view.
    pub fn app_view(&self, app: &str) -> Vec<ViewMount> {
        self.inner().views(app)
    }

    /// Takes a mount away from an app's view, and takes back its grant.
    ///
    /// # Errors
    ///
    /// When there is no such mount.
    pub fn app_take(&self, app: &str, name: &str) -> Result<(), Error> {
        let result = self
            .inner()
            .take_app(app, &clean(name), &self.shared.changed);
        self.pump();
        result
    }

    /// The app with this ID, if it gives HTTP.
    pub fn http_app(&self, id: &str) -> Option<HttpApp> {
        lock(&self.shared.http).get(id).cloned()
    }

    /// The IDs of the apps that give HTTP.
    pub fn http_apps(&self) -> Vec<String> {
        lock(&self.shared.http).keys().cloned().collect()
    }

    /// Makes a new identity. This node holds its rotation key and its
    /// signing key, and is its first node. Gives the DID.
    ///
    /// # Errors
    ///
    /// When this node already has an identity, or the keys cannot be kept.
    pub fn identity_new(&self) -> Result<String, Error> {
        let mut inner = self.inner();
        if let Some(space) = inner.identity_space() {
            return Err(Error(format!(
                "this node already has an identity, with the space {space}"
            )));
        }
        // Keys from an earlier try that did not end are used again.
        let rotation = inner.identity_key("rotation")?;
        let signing = inner.identity_key("signing")?;
        let (space, step) = inner.node.create_as(&signing, now());
        inner.process(step, &self.shared.changed)?;
        inner.mount(space, IDENTITY_AT, &self.shared.changed)?;
        let state = State {
            rotation_keys: vec![fos_id::did_key(rotation.public())],
            verification_methods: [(
                fos_id::SIGNING_METHOD.to_string(),
                fos_id::did_key(signing.public()),
            )]
            .into(),
            also_known_as: Vec::new(),
            services: [(
                fos_id::FOLDER_SERVICE.to_string(),
                Service {
                    kind: fos_id::FOLDER_SERVICE_TYPE.to_string(),
                    endpoint: space.to_string(),
                },
            )]
            .into(),
        };
        let genesis = DidOp::genesis(state, &rotation)?;
        let did = genesis.did();
        let path = format!("{IDENTITY_AT}{}", fos_id::op_path(&genesis));
        inner.put(
            Some(&signing),
            &path,
            &genesis.to_bytes(),
            &self.shared.changed,
        )?;
        inner.add_node(&did, &signing, self.key(), &self.shared.changed)?;
        drop(inner);
        self.pump();
        Ok(did)
    }

    /// Adds a node to the identity: puts its certificate into the identity
    /// space, and lets it read and keep the space. Only the node that holds
    /// the signing key of the identity can do this. Gives the ID of the
    /// identity space, which the other node joins.
    ///
    /// # Errors
    ///
    /// When this node does not hold the signing key of the identity.
    pub fn identity_add_node(&self, node: PublicKey) -> Result<Hash, Error> {
        let mut inner = self.inner();
        let (did, signing, space) = inner.held_identity()?;
        inner.add_node(&did, &signing, node, &self.shared.changed)?;
        drop(inner);
        self.pump();
        Ok(space)
    }

    /// Takes a node away from the identity: deletes its certificate and
    /// takes back its grant on the identity space.
    ///
    /// # Errors
    ///
    /// When this node does not hold the signing key of the identity, or the
    /// node is not one of its nodes.
    pub fn identity_remove_node(&self, node: PublicKey) -> Result<(), Error> {
        let mut inner = self.inner();
        let (did, signing, space) = inner.held_identity()?;
        let path = format!("{IDENTITY_AT}{}", fos_id::node_path(node));
        inner.delete(Some(&signing), &path, &self.shared.changed)?;
        // The revocation ends the node in every space that counted it as a
        // node of the identity.
        let revocation = fos_id::NodeRevocation::sign(&did, &signing, node);
        let path = format!("{IDENTITY_AT}{}", fos_id::revocation_path(node));
        inner.put(
            Some(&signing),
            &path,
            &revocation.to_bytes(),
            &self.shared.changed,
        )?;
        let grants: Vec<Id> = inner
            .node
            .space(space)
            .map(|s| {
                s.grants()
                    .into_iter()
                    .filter(|g| g.given.to == Principal::Key(node))
                    .map(|g| g.given.id)
                    .collect()
            })
            .unwrap_or_default();
        for grant in grants {
            inner.write(
                Some(&signing),
                space,
                vec![Act::Take { grant }],
                &self.shared.changed,
            )?;
        }
        drop(inner);
        self.pump();
        Ok(())
    }

    /// Makes this node one of the nodes of an identity, by the ID of its
    /// space. The node that holds the signing key must add this node first
    /// ([`Node::identity_add_node`]), or the space does not come.
    ///
    /// # Errors
    ///
    /// When this node already has an identity.
    pub fn identity_join(&self, space: Hash) -> Result<(), Error> {
        if let Some(have) = self.inner().identity_space() {
            return Err(Error(format!(
                "this node already has an identity, with the space {have}"
            )));
        }
        self.mount(space, IDENTITY_AT)
    }

    /// The identity of this node, if it has one.
    pub fn identity(&self) -> Option<Identity> {
        self.inner().identity(self.key())
    }

    /// The cache of an app on this node.
    pub fn kv(&self, app: &str) -> BTreeMap<String, Vec<u8>> {
        self.inner().kv(app).clone()
    }

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
        let addr = addr
            .parse()
            .map_err(|e| Error(format!("not a multiaddr: {e}")))?;
        let inner = self.inner();
        let net = inner
            .net
            .as_ref()
            .ok_or_else(|| Error("the network is not started".into()))?;
        net.send(fos_net::Command::Dial(addr))
            .map_err(|e| Error(format!("the network stopped: {e}")))
    }

    fn on_net(&self, event: fos_net::Event) {
        {
            let mut inner = self.inner();
            let step = match event {
                fos_net::Event::Listening(addr) => {
                    inner.listening.push(format!(
                        "{addr}/p2p/{}",
                        fos_net::peer_id(self.key())
                            .map(|p| p.to_string())
                            .unwrap_or_default()
                    ));
                    return;
                }
                fos_net::Event::Connected(peer) => inner.node.connected(peer),
                fos_net::Event::Disconnected(peer) => {
                    inner.node.disconnected(peer);
                    return;
                }
                fos_net::Event::Message { from, msg } => {
                    let Inner { node, store, .. } = &mut *inner;
                    node.receive(from, *msg, store)
                }
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
            inner
                .mounts()
                .into_iter()
                .filter(|(at, _)| at.starts_with("apps/"))
                .filter_map(|(_, id)| Some((inner.bundle(id)?, id)))
                .collect::<Vec<_>>()
        };
        for (bundle, space) in bundles {
            if let Err(e) = self.start_app(&bundle, space) {
                tracing::warn!("app {} did not start: {e}", bundle.manifest.id);
            }
        }
    }

    /// Starts an app that is in the space `space`. Gives its app ID.
    fn start_app(&self, bundle: &Bundle, space: Hash) -> Result<String, Error> {
        let host = Arc::new(Host {
            shared: Arc::downgrade(&self.shared),
        });
        let id = app_id(&bundle.manifest.id, space);
        let http = self
            .shared
            .runtime
            .http_app(bundle, Arc::<Host>::clone(&host))?;
        let app = self.shared.runtime.app(bundle, host)?;
        lock(&self.shared.apps).push(app);
        lock(&self.shared.http).insert(id.clone(), http);
        lock(&self.shared.schedules).insert(id.clone(), bundle.manifest.schedule.clone());
        if self
            .shared
            .scheduling
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            for schedule in &bundle.manifest.schedule {
                self.spawn_schedule(&id, schedule);
            }
        }
        Ok(id)
    }

    /// Runs one schedule of an app now: sends `POST
    /// /.well-known/fos/schedule/<name>` with `fos-event: schedule` to the
    /// app. Gives the status of its answer.
    ///
    /// # Errors
    ///
    /// When there is no such app or schedule, or the app fails.
    pub async fn run_schedule(&self, app: &str, name: &str) -> Result<u16, Error> {
        let known = lock(&self.shared.schedules)
            .get(app)
            .is_some_and(|list| list.iter().any(|s| s.name == name));
        let http = self
            .http_app(app)
            .filter(|_| known)
            .ok_or_else(|| Error(format!("the app {app} has no schedule {name:?}")))?;
        let answer = http
            .request(
                "POST",
                &format!("{}{name}", fos_runtime::SCHEDULE_PATH),
                &[("fos-event", "schedule")],
                Vec::new(),
            )
            .await?;
        Ok(answer.status)
    }

    /// Starts the timers of the schedules of every app. Call it once, inside
    /// a Tokio runtime. A schedule runs first one period after the start;
    /// times missed while the node was off are not made up, as with cron.
    pub fn start_schedules(&self) {
        self.shared
            .scheduling
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let all: Vec<(String, fos_runtime::Schedule)> = lock(&self.shared.schedules)
            .iter()
            .flat_map(|(id, list)| list.iter().map(|s| (id.clone(), s.clone())))
            .collect();
        for (id, schedule) in all {
            self.spawn_schedule(&id, &schedule);
        }
    }

    fn spawn_schedule(&self, id: &str, schedule: &fos_runtime::Schedule) {
        let Ok(seconds) = schedule.seconds() else {
            return;
        };
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let node = self.clone();
        let (id, name) = (id.to_string(), schedule.name.clone());
        handle.spawn(async move {
            let period = std::time::Duration::from_secs(seconds);
            let mut timer = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
            timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                timer.tick().await;
                match node.run_schedule(&id, &name).await {
                    Ok(status) if status < 400 => {}
                    Ok(status) => tracing::warn!("schedule {name} of {id}: status {status}"),
                    Err(e) => tracing::warn!("schedule {name} of {id}: {e}"),
                }
            }
        });
    }

    /// Tells the apps about changed paths, until there are no more.
    pub fn pump(&self) {
        pump(&self.shared);
    }
}

/// A principal as people write it: a key, `did:fos:…`, or `group:<id>`.
fn principal_text(p: Principal) -> String {
    match p {
        Principal::Key(k) => k.to_string(),
        Principal::Identity(d) => fos_id::did_of(d),
        Principal::Group(id) => format!("group:{}", grant_text(id)),
    }
}

/// Reads a principal: a node or app key in hex, or a `did:fos` DID for a
/// person and all their nodes.
///
/// # Errors
///
/// When the text is neither.
pub fn parse_principal(text: &str) -> Result<Principal, Error> {
    if text.starts_with("did:") {
        return fos_id::did_hash(text).map(Principal::Identity);
    }
    text.parse().map(Principal::Key)
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
    let (entry, index) = text
        .split_once(':')
        .ok_or_else(|| Error(format!("not a grant ID: {text:?}")))?;
    Ok(Id {
        entry: entry.parse()?,
        index: index
            .parse()
            .map_err(|e| Error(format!("not a grant ID: {text:?} ({e})")))?,
    })
}

fn pump(shared: &Shared) {
    loop {
        let Some(path) = lock(&shared.changed).pop_front() else {
            return;
        };
        // When the apps are busy, this is a write made by an app while it was
        // told of a change: the loop that holds them takes this path next.
        let mut apps = match shared.apps.try_lock() {
            Ok(apps) => apps,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                lock(&shared.changed).push_front(path);
                return;
            }
        };
        // Each app hears of the change at the paths of its own view.
        let seen: Vec<Vec<String>> = {
            let inner = lock(&shared.inner);
            let (space, name) = inner.resolve(&path);
            apps.iter()
                .map(|a| inner.view_paths(&a.manifest().id, space, &name))
                .collect()
        };
        for (app, paths) in apps.iter_mut().zip(seen) {
            let paths: Vec<String> = paths.into_iter().filter(|p| app.watches(p)).collect();
            for path in &paths {
                if let Err(e) = app.on_change(path) {
                    tracing::warn!("app {} failed on {path}: {e}", app.manifest().id);
                }
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

/// The newest version of a file that has content: when there are more, the
/// one made last (by its time, then its ID).
fn newest(file: &File) -> Option<Content> {
    file.versions
        .iter()
        .filter(|v| v.content.is_some())
        .max_by_key(|v| (v.time, v.id))
        .and_then(|v| v.content)
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
                let bytes = fos_space::content::join(&newest(&f)?, &self.store)?;
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

    fn mount(
        &mut self,
        space: Hash,
        at: &str,
        changed: &Mutex<VecDeque<String>>,
    ) -> Result<(), Error> {
        let at = clean(at);
        if at.is_empty() || self.mounts().iter().any(|(m, _)| *m == at) {
            return Err(Error(format!("something is mounted at /{at} already")));
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

    fn identity_space(&self) -> Option<Hash> {
        self.mounts()
            .into_iter()
            .find(|(at, _)| *at == clean(IDENTITY_AT))
            .map(|(_, id)| id)
    }

    /// Writes acts into a space, as `author` (the node when `None`).
    fn write(
        &mut self,
        author: Option<&SecretKey>,
        space: Hash,
        acts: Vec<Act>,
        changed: &Mutex<VecDeque<String>>,
    ) -> Result<Id, Error> {
        let author = author.cloned().unwrap_or_else(|| self.key.clone());
        let (entry, step) = self.node.write_as(space, &author, now(), acts)?;
        self.process(step, changed)?;
        Ok(Id {
            entry: entry.hash(),
            index: 0,
        })
    }

    /// Keeps content in the store, and gives what names it.
    fn keep(&self, data: &[u8]) -> Result<Content, Error> {
        let (content, pieces) = fos_space::content::split(data);
        for (hash, bytes) in pieces {
            self.store.put_piece(hash, &bytes)?;
        }
        Ok(content)
    }

    fn put(
        &mut self,
        author: Option<&SecretKey>,
        path: &str,
        data: &[u8],
        changed: &Mutex<VecDeque<String>>,
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
        changed: &Mutex<VecDeque<String>>,
    ) -> Result<(), Error> {
        let content = self.keep(data)?;
        let s = self
            .node
            .space(space)
            .ok_or_else(|| Error(format!("space {space} is not here yet")))?;
        let act = match s.find(name).first() {
            Some(file) => Act::Change {
                file: file.id,
                parents: file.versions.iter().map(|v| v.id).collect(),
                name: name.to_string(),
                content: Some(content),
            },
            None => Act::Create {
                name: name.to_string(),
                content,
            },
        };
        self.write(author, space, vec![act], changed).map(drop)
    }

    fn append(
        &mut self,
        author: Option<&SecretKey>,
        path: &str,
        data: &[u8],
        changed: &Mutex<VecDeque<String>>,
    ) -> Result<(), Error> {
        let who = author.map_or_else(|| self.key.public(), SecretKey::public);
        let mut bytes = match self.read(who, path) {
            Ok(bytes) => bytes,
            Err(FsError::NotFound) => Vec::new(),
            Err(e) => return Err(fs_error(e)),
        };
        bytes.extend_from_slice(data);
        self.put(author, path, &bytes, changed)
    }

    fn delete(
        &mut self,
        author: Option<&SecretKey>,
        path: &str,
        changed: &Mutex<VecDeque<String>>,
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
        changed: &Mutex<VecDeque<String>>,
    ) -> Result<(), Error> {
        let s = self
            .node
            .space(space)
            .ok_or_else(|| Error(format!("space {space} is not here yet")))?;
        let file = s
            .find(name)
            .into_iter()
            .next()
            .ok_or_else(|| Error(format!("{name:?} is not there")))?;
        let act = Act::Change {
            file: file.id,
            parents: file.versions.iter().map(|v| v.id).collect(),
            name: name.to_string(),
            content: None,
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
        changed: &Mutex<VecDeque<String>>,
    ) -> Result<Id, Error> {
        let (space, name) = self.resolve(path);
        let s = self
            .node
            .space(space)
            .ok_or_else(|| Error(format!("space {space} is not here yet")))?;
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

    fn checkpoint(&mut self, space: Hash, changed: &Mutex<VecDeque<String>>) -> Result<(), Error> {
        let step = self.node.checkpoint(space, now(), Retention::default())?;
        self.process(step, changed)?;
        let (_, step) = self.node.compact(space);
        self.process(step, changed)?;
        self.save(space)
    }

    /// Says if `who` may do `permission` on `name` in `space`.
    fn may(s: &Space, who: PublicKey, permission: Permission, name: &str) -> bool {
        let on = Self::resource(s, name);
        s.may(Principal::Key(who), permission, &on)
    }

    fn read(&self, who: PublicKey, path: &str) -> Result<Vec<u8>, FsError> {
        let (space, name) = self.resolve(path);
        self.read_in(who, space, &name)
    }

    fn read_in(&self, who: PublicKey, space: Hash, name: &str) -> Result<Vec<u8>, FsError> {
        let s = self.node.space(space).ok_or(FsError::NotFound)?;
        let file = s.find(name).into_iter().next().ok_or(FsError::NotFound)?;
        if !Self::may(s, who, Permission::FilesRead, name) {
            return Err(FsError::Denied(format!("{who} may not read {name}")));
        }
        let content = newest(&file).ok_or(FsError::NotFound)?;
        fos_space::content::join(&content, &self.store)
            .ok_or_else(|| FsError::Invalid("the file is not on this node yet".into()))
    }

    fn list(&self, who: PublicKey, path: &str) -> Result<Vec<String>, FsError> {
        let (space, name) = self.resolve(path);
        let mut names = self.list_in(who, space, &name)?;
        if space == self.home {
            names.retain(|n| ![".mounts", ".views", ".appdata"].contains(&n.as_str()));
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

    /// The private data space of an app, if it has one.
    fn app_data(&self, app: &str) -> Option<Hash> {
        let at = format!("{APPDATA}{app}");
        self.mounts()
            .into_iter()
            .find(|(m, _)| *m == at)
            .map(|(_, id)| id)
    }

    /// The mounts in the view of an app.
    fn views(&self, app: &str) -> Vec<ViewMount> {
        let prefix = format!("{VIEWS}{app}/");
        let Some(home) = self.node.space(self.home) else {
            return Vec::new();
        };
        let mut out: Vec<ViewMount> = home
            .files()
            .iter()
            .filter_map(|f| {
                let name = f
                    .versions
                    .iter()
                    .find_map(|v| v.name.strip_prefix(&prefix))?
                    .to_string();
                let bytes = fos_space::content::join(&newest(f)?, &self.store)?;
                let text = String::from_utf8(bytes).ok()?;
                let mut lines = text.split('\n');
                let space = lines.next()?.parse().ok()?;
                let prefix = lines.next().unwrap_or_default().to_string();
                let grant = lines.next().and_then(|g| parse_grant(g).ok());
                Some(ViewMount {
                    name,
                    space,
                    prefix,
                    grant,
                })
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    }

    /// The space and the name in it of a path in the view of an app. A path
    /// under no mount is in the app's private data space.
    fn resolve_app(&self, app: &str, path: &str) -> Result<(Hash, String), FsError> {
        let path = clean(path);
        let best = self
            .views(app)
            .into_iter()
            .filter(|v| path == v.name || path.starts_with(&format!("{}/", v.name)))
            .max_by_key(|v| v.name.len());
        if let Some(v) = best {
            let rest = path[v.name.len()..].trim_start_matches('/');
            let name = match (v.prefix.is_empty(), rest.is_empty()) {
                (true, _) => rest.to_string(),
                (false, true) => v.prefix.clone(),
                (false, false) => format!("{}/{rest}", v.prefix),
            };
            return Ok((v.space, name));
        }
        let data = self
            .app_data(app)
            .ok_or_else(|| FsError::Invalid("the app has no data space".into()))?;
        Ok((data, path))
    }

    /// The paths in the view of an app where a name of a space is seen.
    fn view_paths(&self, app: &str, space: Hash, name: &str) -> Vec<String> {
        let mut out = Vec::new();
        for v in self.views(app) {
            if v.space != space {
                continue;
            }
            let rest = if v.prefix.is_empty() {
                Some(name)
            } else if name == v.prefix {
                Some("")
            } else {
                name.strip_prefix(&format!("{}/", v.prefix))
            };
            if let Some(rest) = rest {
                out.push(if rest.is_empty() {
                    format!("/{}", v.name)
                } else {
                    format!("/{}/{rest}", v.name)
                });
            }
        }
        if self.app_data(app) == Some(space) {
            out.push(format!("/{name}"));
        }
        out
    }

    /// Gives the key of `app` permissions on a node path, and mounts it in
    /// the app's view at `name`.
    fn give_app(
        &mut self,
        app: &str,
        path: &str,
        permissions: &[Permission],
        name: &str,
        changed: &Mutex<VecDeque<String>>,
    ) -> Result<(), Error> {
        let key = self.app_key(app).public();
        let (space, prefix) = self.resolve(path);
        let grant = self.grant(None, path, Principal::Key(key), permissions, changed)?;
        let home = self.home;
        let text = format!("{space}\n{prefix}\n{}", grant_text(grant));
        self.put_name(
            None,
            home,
            &format!("{VIEWS}{app}/{name}"),
            text.as_bytes(),
            changed,
        )
    }

    /// Takes a mount away from the view of an app, and takes back its grant.
    fn take_app(
        &mut self,
        app: &str,
        name: &str,
        changed: &Mutex<VecDeque<String>>,
    ) -> Result<(), Error> {
        let v = self
            .views(app)
            .into_iter()
            .find(|v| v.name == name)
            .ok_or_else(|| Error(format!("the app {app} has no /{name}")))?;
        if let Some(grant) = v.grant
            && self
                .node
                .space(v.space)
                .is_some_and(|s| s.grants().iter().any(|g| g.given.id == grant))
        {
            self.write(None, v.space, vec![Act::Take { grant }], changed)?;
        }
        let home = self.home;
        self.delete_name(None, home, &format!("{VIEWS}{app}/{name}"), changed)
    }

    fn kv(&mut self, app: &str) -> &mut BTreeMap<String, Vec<u8>> {
        let store = &self.store;
        self.kv
            .entry(app.to_string())
            .or_insert_with(|| store.kv_load(app))
    }

    /// The node paths of every file under a node path.
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

    /// The files of an app space, as a bundle.
    fn bundle(&self, space: Hash) -> Option<Bundle> {
        let s = self.node.space(space)?;
        let files = s
            .files()
            .iter()
            .map(|f| {
                let name = &f.versions.first()?.name;
                let bytes = fos_space::content::join(&newest(f)?, &self.store)?;
                Some((format!("/{name}"), bytes))
            })
            .collect::<Option<_>>()?;
        Bundle::from_files(files).ok()
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
            *saved = (
                s.entries().iter().map(fos_space::Entry::hash).collect(),
                base,
            );
            return Ok(());
        }
        let new: Vec<fos_space::Entry> = s
            .entries()
            .into_iter()
            .filter(|e| !saved.0.contains(&e.hash()))
            .collect();
        self.store.append_entries(space, &new)?;
        saved.0.extend(new.iter().map(fos_space::Entry::hash));
        Ok(())
    }

    /// Finds the names in a space whose newest versions changed since apps
    /// last heard, and notes their node paths (or waits for their content).
    fn note(&mut self, space: Hash, changed: &Mutex<VecDeque<String>>) {
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
        let before = self.seen.insert(space, now.clone()).unwrap_or_default();
        let names: BTreeSet<&String> = now
            .iter()
            .filter(|(n, ids)| before.get(*n) != Some(*ids))
            .map(|(n, _)| n)
            .chain(before.keys().filter(|n| !now.contains_key(*n)))
            .collect();
        let mut ready = Vec::new();
        for name in names {
            let at = self.node_paths(space, name);
            match contents.get(name).copied().flatten() {
                Some(c) if !fos_space::content::complete(&c, &self.store) => {
                    self.waiting.entry(c.hash).or_default().extend(at);
                }
                _ => ready.extend(at),
            }
        }
        lock(changed).extend(ready);
    }

    /// Keeps what a step made, sends its messages, and notes changed paths.
    fn process(&mut self, step: Step, changed: &Mutex<VecDeque<String>>) -> Result<(), Error> {
        if let Some(net) = &self.net {
            for (to, msg) in step.out {
                let _ = net.send(fos_net::Command::Send {
                    to,
                    msg: Box::new(msg),
                });
            }
        }
        for event in step.events {
            match event {
                Event::Changed(space) => {
                    self.save(space)?;
                    self.note(space, changed);
                    self.maybe_checkpoint(space, changed);
                    if self.identity_space() == Some(space) {
                        self.update_proof(changed)?;
                    }
                }
                Event::Content(data) => {
                    let hash = Hash::of(&data);
                    self.store.put_piece(hash, &data)?;
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
                                        c.hash == *h && fos_space::content::complete(c, &self.store)
                                    })
                                })
                        })
                        .collect();
                    for h in done {
                        let paths = self.waiting.remove(&h).unwrap_or_default();
                        lock(changed).extend(paths);
                    }
                    // The certificate of this node may be what just came.
                    self.update_proof(changed)?;
                }
                Event::Again { space, entries } => {
                    self.log(format!(
                        "{entries} changes made at the same time as a checkpoint of space \
                         {space} were written again on top"
                    ));
                }
            }
        }
        Ok(())
    }

    /// The default policy: a node makes a checkpoint of a space it owns when
    /// the space has many entries, and every node compacts when it can.
    fn maybe_checkpoint(&mut self, space: Hash, changed: &Mutex<VecDeque<String>>) {
        let Some(s) = self.node.space(space) else {
            return;
        };
        if s.len() < CHECKPOINT_AFTER {
            return;
        }
        let mine = s.may(
            Principal::Key(self.key.public()),
            Permission::SpaceCheckpoint,
            &Resource::Prefix(String::new()),
        );
        let result = if mine {
            self.checkpoint(space, changed)
        } else {
            let (_, step) = self.node.compact(space);
            self.process(step, changed).and_then(|()| self.save(space))
        };
        if let Err(e) = result {
            tracing::warn!("space {space}: {e}");
        }
    }

    fn log(&mut self, message: String) {
        tracing::info!("{message}");
        self.logs.push_back(message);
        while self.logs.len() > 100 {
            self.logs.pop_front();
        }
    }

    /// A secret key of the identity on this node, by its use. It is made
    /// and kept when this node does not have it yet.
    fn identity_key(&mut self, usage: &str) -> Result<SecretKey, Error> {
        let held = match usage {
            "rotation" => &mut self.rotation_key,
            _ => &mut self.signing_key,
        };
        if let Some(key) = held {
            return Ok(key.clone());
        }
        let key = SecretKey::from_seed(&fos_store::random_seed()?);
        self.store.set_identity_key(usage, &key)?;
        *held = Some(key.clone());
        Ok(key)
    }

    /// The bytes of the files under `dir` in a space, by the rest of their
    /// name.
    fn files_in(&self, space: Hash, dir: &str) -> Vec<(String, Vec<u8>)> {
        let Some(s) = self.node.space(space) else {
            return Vec::new();
        };
        let prefix = format!("{}/", dir.trim_matches('/'));
        s.files()
            .iter()
            .filter_map(|f| {
                let name = f.versions.first()?.name.strip_prefix(&prefix)?.to_string();
                let bytes = fos_space::content::join(&newest(f)?, &self.store)?;
                Some((name, bytes))
            })
            .collect()
    }

    /// The operations of the DID log in the identity space.
    fn identity_ops(&self, space: Hash) -> Vec<DidOp> {
        self.files_in(space, fos_id::OPS)
            .into_iter()
            .filter_map(|(name, bytes)| {
                let op = DidOp::from_bytes(&bytes).ok()?;
                (op.hash().to_string() == name).then_some(op)
            })
            .collect()
    }

    /// The DID and the state of the identity whose log is in `space`: the
    /// one whose state names this space, and whose signing key owns it.
    fn resolve_identity(&self, space: Hash) -> Option<(String, State)> {
        let owner = self.node.space(space)?.owner();
        let ops = self.identity_ops(space);
        fos_id::dids(&ops).into_iter().find_map(|did| {
            let state = fos_id::resolve(&did, &ops).ok()?.state?;
            (state.folder() == Some(space)
                && state.signing_key().map(Principal::Key) == Some(owner))
            .then_some((did, state))
        })
    }

    /// The DID, the signing key and the space of the identity, when this
    /// node holds the signing key.
    fn held_identity(&self) -> Result<(String, SecretKey, Hash), Error> {
        let none = || Error("this node does not hold the signing key of an identity".into());
        let signing = self.signing_key.clone().ok_or_else(none)?;
        let space = self.identity_space().ok_or_else(none)?;
        let (did, state) = self.resolve_identity(space).ok_or_else(none)?;
        if state.signing_key() != Some(signing.public()) {
            return Err(none());
        }
        Ok((did, signing, space))
    }

    /// Puts the certificate of `node` into the identity space, and lets the
    /// node read and keep the space.
    fn add_node(
        &mut self,
        did: &str,
        signing: &SecretKey,
        node: PublicKey,
        changed: &Mutex<VecDeque<String>>,
    ) -> Result<(), Error> {
        let cert = NodeCert::sign(did, signing, node);
        let path = format!("{IDENTITY_AT}{}", fos_id::node_path(node));
        self.put(Some(signing), &path, &cert.to_bytes(), changed)?;
        if node == signing.public() {
            return Ok(());
        }
        self.grant(
            Some(signing),
            IDENTITY_AT,
            Principal::Key(node),
            &READ,
            changed,
        )
        .map(drop)
    }

    /// The identity of this node. Which nodes belong to it is a fact: the
    /// kernel checks the DID log and the certificates.
    fn identity(&self, me: PublicKey) -> Option<Identity> {
        let space = self.identity_space()?;
        let holds = |key: &Option<SecretKey>, keys: &[PublicKey]| {
            key.as_ref().is_some_and(|k| keys.contains(&k.public()))
        };
        let Some((did, state)) = self.resolve_identity(space) else {
            return Some(Identity {
                did: None,
                space,
                rotation_keys: Vec::new(),
                also_known_as: Vec::new(),
                holds_rotation_key: false,
                holds_signing_key: false,
                nodes: Vec::new(),
                link: None,
            });
        };
        let signing = state.signing_key();
        let nodes = self
            .files_in(space, fos_id::NODES)
            .into_iter()
            .filter_map(|(name, bytes)| {
                let cert = NodeCert::from_bytes(&bytes).ok()?;
                (cert.did == did
                    && signing.is_some_and(|key| cert.verify(key))
                    && cert.node.to_string() == name)
                    .then_some(cert.node)
            })
            .collect::<Vec<_>>();
        // Other nodes find this one by its addresses only if it is one of
        // the nodes of the identity.
        let hints = if nodes.contains(&me) {
            self.listening.clone()
        } else {
            Vec::new()
        };
        Some(Identity {
            link: Some(
                Link {
                    did: did.clone(),
                    hints,
                }
                .to_string(),
            ),
            did: Some(did),
            space,
            holds_rotation_key: holds(
                &self.rotation_key,
                &state.rotation_keys().unwrap_or_default(),
            ),
            holds_signing_key: holds(&self.signing_key, &signing.into_iter().collect::<Vec<_>>()),
            rotation_keys: state.rotation_keys,
            also_known_as: state.also_known_as,
            nodes,
        })
    }

    /// When this node is a node of its identity, tells the kernel, so that a
    /// grant to the identity is a grant to this node too.
    fn update_proof(&mut self, changed: &Mutex<VecDeque<String>>) -> Result<(), Error> {
        let Some(space) = self.identity_space() else {
            return Ok(());
        };
        let revocations: Vec<fos_core::Bytes> = self
            .files_in(space, fos_id::REVOKED)
            .into_iter()
            .map(|(_, bytes)| fos_core::Bytes(bytes))
            .collect();
        if !revocations.is_empty() {
            let step = self.node.add_revocations(revocations);
            self.process(step, changed)?;
        }
        let me = self.key.public().to_string();
        let ops: Vec<fos_core::Bytes> = self
            .files_in(space, fos_id::OPS)
            .into_iter()
            .map(|(_, bytes)| fos_core::Bytes(bytes))
            .collect();
        let Some(cert) = self
            .files_in(space, fos_id::NODES)
            .into_iter()
            .find(|(name, _)| *name == me)
            .map(|(_, bytes)| fos_core::Bytes(bytes))
        else {
            return Ok(());
        };
        match self.node.set_identity(ops, cert) {
            Ok(step) => self.process(step, changed),
            // Not a good certificate (yet, or any more): nothing to show.
            Err(_) => Ok(()),
        }
    }

    fn app_key(&self, app: &str) -> SecretKey {
        self.key.derive(&format!("app:{app}"))
    }
}

/// What an app may call. Every call is checked against the grants of the
/// app's key.
struct Host {
    shared: Weak<Shared>,
}

impl Host {
    fn with<T>(&self, f: impl FnOnce(&mut Inner, &Mutex<VecDeque<String>>) -> T) -> Option<T> {
        let shared = self.shared.upgrade()?;
        let out = f(&mut lock(&shared.inner), &shared.changed);
        // Apps hear of what an app wrote.
        pump(&shared);
        Some(out)
    }

    fn stopped<T>() -> Result<T, FsError> {
        Err(FsError::Invalid("the node stopped".into()))
    }
}

impl AppHost for Host {
    fn read(&self, app: &str, path: &str) -> Result<Vec<u8>, FsError> {
        self.with(|inner, _| {
            let key = inner.app_key(app).public();
            let (space, name) = inner.resolve_app(app, path)?;
            inner.read_in(key, space, &name)
        })
        .unwrap_or_else(Host::stopped)
    }

    fn write(&self, app: &str, path: &str, data: Vec<u8>) -> Result<(), FsError> {
        self.with(|inner, changed| {
            let key = inner.app_key(app);
            let (space, name) = inner.resolve_app(app, path)?;
            inner
                .put_name(Some(&key), space, &name, &data, changed)
                .map_err(|e| FsError::Denied(e.0))
        })
        .unwrap_or_else(Host::stopped)
    }

    fn append(&self, app: &str, path: &str, data: Vec<u8>) -> Result<(), FsError> {
        self.with(|inner, changed| {
            let key = inner.app_key(app);
            let (space, name) = inner.resolve_app(app, path)?;
            let mut bytes = match inner.read_in(key.public(), space, &name) {
                Ok(bytes) => bytes,
                Err(FsError::NotFound) => Vec::new(),
                Err(e) => return Err(e),
            };
            bytes.extend_from_slice(&data);
            inner
                .put_name(Some(&key), space, &name, &bytes, changed)
                .map_err(|e| FsError::Denied(e.0))
        })
        .unwrap_or_else(Host::stopped)
    }

    fn delete(&self, app: &str, path: &str) -> Result<(), FsError> {
        self.with(|inner, changed| {
            let key = inner.app_key(app);
            let (space, name) = inner.resolve_app(app, path)?;
            inner
                .delete_name(Some(&key), space, &name, changed)
                .map_err(|e| FsError::Denied(e.0))
        })
        .unwrap_or_else(Host::stopped)
    }

    fn list(&self, app: &str, path: &str) -> Result<Vec<String>, FsError> {
        self.with(|inner, _| {
            let key = inner.app_key(app).public();
            let (space, name) = inner.resolve_app(app, path)?;
            let mut names = inner.list_in(key, space, &name)?;
            // Mounts of the view under this path are names too.
            let here = clean(path);
            for v in inner.views(app) {
                let rest = if here.is_empty() {
                    Some(v.name.as_str())
                } else {
                    v.name.strip_prefix(&format!("{here}/"))
                };
                if let Some(first) = rest.and_then(|r| r.split('/').next()) {
                    names.push(first.to_string());
                }
            }
            names.sort();
            names.dedup();
            Ok(names)
        })
        .unwrap_or_else(Host::stopped)
    }

    fn kv_get(&self, app: &str, key: &str) -> Option<Vec<u8>> {
        self.with(|inner, _| inner.kv(app).get(key).cloned())
            .flatten()
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
        self.with(|inner, _| {
            inner
                .kv(app)
                .keys()
                .filter(|k| k.starts_with(prefix))
                .cloned()
                .collect()
        })
        .unwrap_or_default()
    }

    fn log(&self, app: &str, message: &str) {
        self.with(|inner, _| inner.log(format!("[{app}] {message}")));
    }
}

impl fmt::Display for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut out = String::new();
        match &self.did {
            Some(did) => {
                let _ = writeln!(out, "did      {did}");
            }
            None => {
                let _ = writeln!(
                    out,
                    "did      (waiting for the identity space: the node that holds the \
                     signing key must add this node, and be connected)"
                );
            }
        }
        let _ = writeln!(out, "space    {}", self.space);
        if let Some(link) = &self.link {
            let _ = writeln!(out, "link     {link}");
        }
        for name in &self.also_known_as {
            let _ = writeln!(out, "aka      {name}");
        }
        for (i, key) in self.rotation_keys.iter().enumerate() {
            let _ = writeln!(out, "rotation {key} (priority {})", i + 1);
        }
        if self.holds_rotation_key {
            let _ = writeln!(out, "         this node holds a rotation key");
        }
        if self.holds_signing_key {
            let _ = writeln!(out, "         this node holds the signing key");
        }
        for node in &self.nodes {
            let _ = writeln!(out, "node     {node}");
        }
        f.write_str(&out)
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
