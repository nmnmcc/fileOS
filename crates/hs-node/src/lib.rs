// SPDX-License-Identifier: AGPL-3.0-only

//! A homeserver node.
//!
//! Assembles identity, spaces, blobs, replicas, the derived index and the HTTP
//! API into one node. Browsers and phones are user agents that talk to a node
//! over HTTP; they do not run one.
//!
//! The node owns one data directory:
//!
//! ```text
//! config.json        who this node is: identity, private space, URL
//! identity           the identity log (signed by the personal key)
//! keys/node          the node key (seed, hex)
//! keys/storage       the key that encrypts replicas (hex)
//! logs/<space>/      head, sealed segments and the tail of each space's log
//! blobs/ab/cdef…     payloads and files, by BLAKE3 hash
//! index.db           the derived SQLite index; delete it to rebuild
//! access.log         structured HTTP access events
//! ```

mod auth;
mod logs;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

use hs_core::event::{Kind, Proposal};
use hs_core::{Hash, PublicKey};
use hs_identity::Keypair;
use hs_identity::{IdentityLog, IdentityState, LogKind};
use hs_objects::{Dir, blob_name};
use hs_space::{Applied, Refused, Role, Space};
use hs_store::{Change, Index, Opened};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::{Value as Json, json};

#[doc(inline)]
pub use auth::{Access, token_hash};
#[doc(inline)]
pub use logs::{SEGMENT_ENTRIES, SpaceLog};

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The components this node is built from, bottom first.
pub const COMPONENTS: &[&str] = &[
    hs_core::NAME,
    hs_identity::NAME,
    hs_space::NAME,
    hs_objects::NAME,
    hs_store::NAME,
    NAME,
];

/// The category of a node failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// The space refused the operation.
    Refused,
    /// The caller lacks permission.
    Forbidden,
    /// The requested object or space does not exist.
    NotFound,
    /// The resource does not match the required version.
    PreconditionFailed,
    /// The idempotency key already names a different operation.
    Conflict,
    /// Another node is the primary.
    NotPrimary {
        /// The affected space.
        space: Hash,
        /// The primary's address, if known.
        home: Option<String>,
    },
    /// An I/O, index or stored-data failure.
    Io,
}

/// A node failure and its diagnostic context.
#[derive(Debug)]
pub struct Error(Box<ErrorDetails>);

#[derive(Debug)]
struct ErrorDetails {
    kind: ErrorKind,
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
    backtrace: std::backtrace::Backtrace,
}

impl Error {
    fn new(kind: ErrorKind, message: String) -> Self {
        Self(Box::new(ErrorDetails {
            kind,
            message,
            source: None,
            backtrace: std::backtrace::Backtrace::capture(),
        }))
    }

    fn refused(message: String) -> Self {
        Self::new(ErrorKind::Refused, message)
    }
    fn forbidden(message: String) -> Self {
        Self::new(ErrorKind::Forbidden, message)
    }
    fn not_found(message: String) -> Self {
        Self::new(ErrorKind::NotFound, message)
    }
    fn io(message: String) -> Self {
        Self::new(ErrorKind::Io, message)
    }

    /// Report a resource version conflict.
    pub fn precondition_failed(message: &str) -> Self {
        Self::new(ErrorKind::PreconditionFailed, message.into())
    }

    /// Report that the requested resource does not exist.
    pub fn resource_missing() -> Self {
        Self::not_found("resource not found".into())
    }

    fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.0.source = Some(Box::new(source));
        self
    }

    /// Classify the failure without parsing its message.
    pub fn kind(&self) -> &ErrorKind {
        &self.0.kind
    }

    /// Read the failure's message.
    pub fn message(&self) -> &str {
        &self.0.message
    }

    /// Read the backtrace captured at the error boundary.
    pub fn backtrace(&self) -> &std::backtrace::Backtrace {
        &self.0.backtrace
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.0.kind {
            ErrorKind::Refused => write!(f, "refused: {}", self.0.message),
            ErrorKind::Forbidden => write!(f, "forbidden: {}", self.0.message),
            ErrorKind::NotFound => write!(f, "not found: {}", self.0.message),
            ErrorKind::NotPrimary { .. }
            | ErrorKind::PreconditionFailed
            | ErrorKind::Conflict
            | ErrorKind::Io => f.write_str(&self.0.message),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0
            .source
            .as_deref()
            .map(|source| source as &dyn std::error::Error)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::io(error.to_string()).with_source(error)
    }
}

impl From<hs_store::Error> for Error {
    fn from(error: hs_store::Error) -> Self {
        Self::io(format!("index: {error}")).with_source(error)
    }
}

impl From<Refused> for Error {
    fn from(error: Refused) -> Self {
        Self::refused(error.0.clone()).with_source(error)
    }
}

/// A result.
pub type Result<T, E = Error> = std::result::Result<T, E>;

async fn disk<T: Send + 'static>(work: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| Error::io(format!("disk worker failed: {error}")).with_source(error))?
}

fn put_blob(dir: &Dir, bytes: &[u8]) -> Result<Hash> {
    let hash = Hash::of(bytes);
    let name = blob_name(&hash);
    if dir.len(&name)?.is_none() {
        dir.put(&name, bytes)?;
    }
    Ok(hash)
}

fn read_blob(dir: &Dir, hash: &Hash) -> Result<Option<Vec<u8>>> {
    Ok(dir
        .get(&blob_name(hash))?
        .filter(|bytes| Hash::of(bytes) == *hash))
}

/// The result recorded in a space's log.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Committed {
    /// The sequence assigned by the primary.
    pub seq: u64,
    /// The hash of the signed entry.
    pub entry: Hash,
    /// The payload hash in that entry.
    pub payload: Option<Hash>,
    /// Whether this write just created the resource. Absent for older peers.
    pub created: Option<bool>,
}

/// The key and fingerprint of an idempotent API request.
#[derive(Debug)]
pub struct WriteId {
    /// The stable, scoped request key.
    pub nonce: Vec<u8>,
    /// The request method, target, conditions and body fingerprint.
    pub request: Hash,
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Random bytes from the operating system.
///
/// # Panics
///
/// Panics if the operating system cannot supply random bytes.
pub fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    // The system RNG does not fail on supported platforms.

    SystemRandom::new().fill(&mut b).expect("system random");
    b
}

fn hex(b: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(b)
}

fn unhex32(s: &str) -> Option<[u8; 32]> {
    Hash::from_hex(s.trim()).map(|h| h.0)
}

/// Something that happened, for live updates and federation.
#[derive(Clone, Debug)]
pub struct Event {
    /// The space.
    pub space: Hash,
    /// The entry's sequence number.
    pub seq: u64,
    /// The entry kind.
    pub kind: Kind,
    /// `type/id`.
    pub target: String,
    /// The signed entry.
    pub entry: Arc<Vec<u8>>,
    /// Whether this node ordered it (it is the primary).
    pub ordered_here: bool,
}

/// What the node knows about itself.
#[derive(Clone)]
pub struct Config {
    /// The person's identity.
    pub identity: PublicKey,
    /// Their private space.
    pub private: Hash,
    /// A display name.
    pub name: String,
    /// The public URL of this node, if it has one.
    pub url: Option<String>,
    /// Everything else in config.json.
    pub raw: Json,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config").finish_non_exhaustive()
    }
}

/// A running node.
pub struct Node {
    dir: PathBuf,
    /// The node key.
    pub key: Keypair,
    storage_key: [u8; 32],
    config: RwLock<Config>,
    identity: RwLock<IdentityLog>,
    /// Payloads and files.
    pub blobs: Dir,
    /// The derived index.
    pub index: Index,
    spaces: Arc<RwLock<BTreeMap<Hash, Arc<SpaceSlot>>>>,
    creating_space: Arc<AsyncMutex<()>>,
    events: tokio::sync::broadcast::Sender<Event>,
    // Held for the node's lifetime so two processes never append to the
    // same logs.
    lock: Arc<std::fs::File>,
}

impl std::fmt::Debug for Node {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Node").finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct SpaceSlot {
    writer: Arc<AsyncMutex<()>>,
    log: AsyncMutex<SpaceLog>,
}

impl SpaceSlot {
    fn new(log: SpaceLog) -> Self {
        Self {
            writer: Arc::new(AsyncMutex::new(())),
            log: AsyncMutex::new(log),
        }
    }
}

/// What `init` returns: the node and the personal recovery code, which is
/// shown once and never stored.
pub struct Initialized {
    /// The node.
    pub node: Node,
    /// The recovery code for the personal key.
    pub recovery_code: String,
    /// An owner token for the first browser or app.
    pub owner_token: String,
}

impl std::fmt::Debug for Initialized {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Initialized").finish_non_exhaustive()
    }
}

impl Node {
    /// Create a new data directory: a personal key (returned as a recovery
    /// code, not stored), a node key, the identity log and a private space.
    ///
    /// # Errors
    ///
    /// Returns an error if the node already exists or its keys, log or index cannot be initialized.
    pub async fn init(dir: &Path, name: &str, url: Option<&str>) -> Result<Initialized> {
        if dir.join("config.json").exists() {
            return Err(Error::refused(format!(
                "{} already holds a node",
                dir.display()
            )));
        }
        std::fs::create_dir_all(dir.join("keys"))?;
        let personal_seed: [u8; 32] = random();
        let personal = Keypair::from_seed(personal_seed);
        let node_key = Keypair::from_seed(random());
        let time = now_ms();
        let mut ilog = IdentityLog::create(&personal, time);
        ilog.append(
            &personal,
            time,
            LogKind::AddNode {
                key: node_key.public(),
            },
        )
        .map_err(|e| Error::io(e.to_string()))?;
        if let Some(u) = url {
            ilog.append(&personal, time, LogKind::Address { url: u.to_string() })
                .map_err(|e| Error::io(e.to_string()))?;
        }
        let storage = storage_key(&personal_seed);
        write_secret(&dir.join("keys/node"), &hex(node_key.seed()))?;
        write_secret(&dir.join("keys/storage"), &hex(&storage))?;
        std::fs::write(dir.join("identity"), ilog.encode())?;
        let identity = ilog.state().map_err(|e| Error::io(e.to_string()))?.id;

        // The private space, created by this node.
        let (signed, payload) = hs_space::genesis_proposal(
            &node_key,
            identity,
            json!({ "name": "private", "kind": "private" }),
            time,
            random::<16>().to_vec(),
        );
        let id = Space::id_of(&signed);
        let config = json!({
            "identity": identity.to_id(),
            "private": id.to_hex(),
            "name": name,
            "url": url,
        });
        std::fs::write(
            dir.join("config.json"),
            serde_json::to_vec_pretty(&config).unwrap_or_default(),
        )?;
        let node = Node::open(dir).await?;
        node.start_space(&signed, &payload, None).await?;
        let (owner_token, _) = node
            .create_token(Role::Owner, None, "owner (created by hs ctl init)", None)
            .await?;
        Ok(Initialized {
            node,
            recovery_code: hs_identity::recovery_code(&personal_seed),
            owner_token,
        })
    }

    /// Open an existing data directory, rebuilding the index if needed.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory is locked, stored data fails validation, or replay or
    /// indexing fails.
    pub async fn open(dir: &Path) -> Result<Node> {
        let lock = std::fs::File::create(dir.join("node.lock"))?;
        if lock.try_lock().is_err() {
            return Err(Error::refused(format!(
                "another hs process is using {}",
                dir.display()
            )));
        }
        let raw: Json = serde_json::from_slice(&std::fs::read(dir.join("config.json"))?)
            .map_err(|e| Error::io(format!("config.json: {e}")))?;
        let config = parse_config(&raw)?;
        let read_key = |name: &str| -> Result<[u8; 32]> {
            let s = std::fs::read_to_string(dir.join("keys").join(name))?;
            unhex32(&s).ok_or_else(|| Error::io(format!("keys/{name} is damaged")))
        };
        let key = Keypair::from_seed(read_key("node")?);
        let storage_key = read_key("storage")?;
        let identity = IdentityLog::decode(&std::fs::read(dir.join("identity"))?)
            .map_err(|e| Error::io(e.to_string()))?;
        identity
            .state()
            .map_err(|e| Error::io(format!("identity log: {e}")))?;
        let blobs = Dir::open(dir.join("blobs"))?;
        let (index, opened) = Index::open(&dir.join("index.db")).await?;
        let (events, _) = tokio::sync::broadcast::channel(1024);
        let node = Node {
            dir: dir.to_path_buf(),
            key,
            storage_key,
            config: RwLock::new(config),
            identity: RwLock::new(identity),
            blobs,
            index,
            spaces: Arc::new(RwLock::new(BTreeMap::new())),
            creating_space: Arc::new(AsyncMutex::new(())),
            events,
            lock: Arc::new(lock),
        };
        std::fs::create_dir_all(dir.join("logs"))?;
        let mut names: Vec<String> = std::fs::read_dir(dir.join("logs"))?
            .filter_map(std::result::Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        let mut erased = Vec::new();
        for name in names {
            let Some(id) = Hash::from_hex(&name) else {
                continue;
            };
            let payloads = |h: &Hash| node.payload(h);
            let (log, applied) = SpaceLog::load(&dir.join("logs").join(&name), id, &payloads)?;
            let space_hex = id.to_hex();
            let indexed = if opened == Opened::Rebuild {
                None
            } else {
                node.index.space_seq(&space_hex).await?
            };
            if indexed.is_none() {
                node.index
                    .set_space(&space_hex, log.space.name(), 0)
                    .await?;
            }
            for a in applied {
                if indexed.is_none_or(|s| a.seq > s) {
                    node.index_applied(&log.space, &a).await?;
                }
                erased.extend(a.erase);
            }
            node.spaces
                .write()
                .map_err(poisoned)?
                .insert(id, Arc::new(SpaceSlot::new(log)));
        }
        node.erase_blobs(&erased).await?;
        Ok(node)
    }

    /// The data directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The node's configuration.
    pub fn config(&self) -> Config {
        self.config
            .read()
            .map(|c| c.clone())
            .unwrap_or_else(|p| p.into_inner().clone())
    }

    /// Change a config.json field.
    ///
    /// # Errors
    ///
    /// Returns an error if the edited configuration is invalid or cannot be saved.
    pub fn set_config(&self, key: &str, value: Json) -> Result<()> {
        let mut c = self.config.write().map_err(poisoned)?;
        let mut raw = c.raw.clone();
        if let Some(o) = raw.as_object_mut() {
            o.insert(key.to_string(), value);
        }
        let new = parse_config(&raw)?;
        let bytes = serde_json::to_vec_pretty(&new.raw).map_err(std::io::Error::other)?;
        Dir::open(&self.dir)?.put("config.json", &bytes)?;
        *c = new;
        Ok(())
    }

    /// The key that encrypts this person's replicas.
    pub fn storage_key(&self) -> hs_objects::crypt::StorageKey {
        hs_objects::crypt::StorageKey::new(self.storage_key)
    }

    /// The identity's current state.
    ///
    /// # Panics
    ///
    /// Panics if the verified in-memory identity log violates its invariant.
    pub fn identity(&self) -> IdentityState {
        self.identity
            .read()
            .map(|l| l.state().expect("verified on open"))
            .unwrap_or_else(|p| p.into_inner().state().expect("verified on open"))
    }

    /// The identity log, encoded.
    pub fn identity_log(&self) -> Vec<u8> {
        self.identity.read().map(|l| l.encode()).unwrap_or_default()
    }

    /// Append to the identity log, signed with the personal key (from the
    /// recovery code). The key is used and dropped.
    ///
    /// # Errors
    ///
    /// Returns an error if the signing key or declaration is invalid or the new log cannot be
    /// saved.
    pub fn identity_append(&self, personal: &Keypair, kind: LogKind) -> Result<IdentityState> {
        let mut log = self.identity.write().map_err(poisoned)?;
        let mut updated = log.clone();
        updated
            .append(personal, now_ms(), kind)
            .map_err(|e| Error::refused(e.to_string()))?;
        let state = updated.state().map_err(|e| Error::io(e.to_string()))?;
        Dir::open(&self.dir)?.put("identity", &updated.encode())?;
        *log = updated;
        Ok(state)
    }

    /// Subscribe to new entries in any space.
    pub fn subscribe(&self) -> tokio::sync::broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    // ---- blobs ----

    /// Store bytes by hash.
    ///
    /// # Errors
    ///
    /// Returns an error if the blob directory cannot be read or the object cannot be written.
    pub fn put_blob(&self, bytes: &[u8]) -> Result<Hash> {
        put_blob(&self.blobs, bytes)
    }

    /// Store bytes without blocking an async worker.
    ///
    /// # Errors
    ///
    /// Returns an error if the object cannot be written.
    pub async fn put_blob_async(&self, bytes: impl AsRef<[u8]> + Send + 'static) -> Result<Hash> {
        let dir = self.blobs.clone();
        disk(move || put_blob(&dir, bytes.as_ref())).await
    }

    /// Read a blob, verifying its hash.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the object cannot be read. A missing or mismatched blob yields
    /// `None`.
    pub fn blob(&self, hash: &Hash) -> Result<Option<Vec<u8>>> {
        read_blob(&self.blobs, hash)
    }

    /// Read and verify a blob on a blocking worker.
    ///
    /// # Errors
    ///
    /// Returns an error if reading the blob fails.
    pub async fn blob_async(&self, hash: &Hash) -> Result<Option<Vec<u8>>> {
        let dir = self.blobs.clone();
        let hash = *hash;
        disk(move || read_blob(&dir, &hash)).await
    }

    /// Open a stored blob for metadata access, streaming or range reads.
    ///
    /// The file name identifies the blob. A caller requiring an integrity check
    /// must verify the bytes against the hash.
    ///
    /// # Errors
    ///
    /// Returns an error if opening the blob fails.
    pub async fn open_blob(&self, hash: &Hash) -> Result<Option<tokio::fs::File>> {
        let dir = self.blobs.clone();
        let name = blob_name(hash);
        disk(move || Ok(dir.open_file(&name)?.map(tokio::fs::File::from_std))).await
    }

    fn payload(&self, hash: &Hash) -> Option<Vec<u8>> {
        self.blob(hash).ok().flatten()
    }

    // ---- spaces ----

    /// The IDs of all spaces on this node.
    pub fn space_ids(&self) -> Vec<Hash> {
        self.spaces
            .read()
            .map(|s| s.keys().copied().collect())
            .unwrap_or_default()
    }

    fn space(&self, id: &Hash) -> Result<Arc<SpaceSlot>> {
        self.spaces
            .read()
            .map_err(poisoned)?
            .get(id)
            .cloned()
            .ok_or_else(|| Error::not_found(format!("space {id}")))
    }

    /// Look at a space's state.
    ///
    /// # Errors
    ///
    /// Returns an error if the space is missing or the node's space map is poisoned.
    pub async fn with_space<T>(&self, id: &Hash, f: impl FnOnce(&SpaceLog) -> T) -> Result<T> {
        let s = self.space(id)?;
        let g = s.log.lock().await;
        Ok(f(&g))
    }

    /// Remember the address of a space's primary.
    ///
    /// # Errors
    ///
    /// Returns an error if the space is missing or its address cannot be saved.
    pub async fn set_space_home(&self, id: &Hash, home: &str) -> Result<()> {
        self.space(id)?.log.lock().await.set_home(home)
    }

    /// Find a space by ID or unique ID prefix or name.
    ///
    /// # Errors
    ///
    /// Returns an error when the name is absent or ambiguous, or a space cannot be read.
    pub async fn find_space(&self, name: &str) -> Result<Hash> {
        if let Some(h) = Hash::from_hex(name) {
            return Ok(h);
        }
        let mut found = Vec::new();
        for id in self.space_ids() {
            let n = self.with_space(&id, |l| l.space.name().to_string()).await?;
            if n == name || id.to_hex().starts_with(name) {
                found.push(id);
            }
        }
        match found.as_slice() {
            [one] => Ok(*one),
            [] => Err(Error::not_found(format!("space {name}"))),
            _ => Err(Error::refused(format!(
                "{name} matches more than one space"
            ))),
        }
    }

    /// Create a space owned by this person, with this node as its primary.
    ///
    /// # Errors
    ///
    /// Returns an error if genesis validation, persistence or indexing fails.
    pub async fn create_space(&self, name: &str, info: Json) -> Result<Hash> {
        let mut info = if info.is_object() { info } else { json!({}) };
        if let Some(o) = info.as_object_mut() {
            o.insert("name".into(), Json::String(name.to_string()));
        }
        let identity = self.config().identity;
        let (signed, payload) = hs_space::genesis_proposal(
            &self.key,
            identity,
            info,
            now_ms(),
            random::<16>().to_vec(),
        );
        self.start_space(&signed, &payload, None).await
    }

    async fn start_space(
        &self,
        signed: &[u8],
        payload: &[u8],
        home: Option<String>,
    ) -> Result<Hash> {
        self.put_blob_async(payload.to_vec()).await?;
        let (space, entry, applied) = Space::genesis(&self.key, signed, payload, now_ms())?;
        let id = space.id;
        let creation = Arc::clone(&self.creating_space).lock_owned().await;
        let (slot, _creation) = self
            .install_log(
                space,
                home,
                (vec![entry], vec![applied]),
                Some(true),
                creation,
            )
            .await?;
        let _writer = slot.writer.lock().await;
        self.finish_pending(&slot).await?;
        Ok(id)
    }

    /// Start following a space from its entries in chronological order.
    ///
    /// # Errors
    ///
    /// Returns an error if the history is invalid or cannot be stored or indexed.
    pub async fn follow(
        &self,
        id: Hash,
        entries: &[Vec<u8>],
        payloads: &BTreeMap<Hash, Vec<u8>>,
        home: &str,
    ) -> Result<()> {
        for payload in payloads.values() {
            self.put_blob_async(payload.clone()).await?;
        }
        let creation = Arc::clone(&self.creating_space).lock_owned().await;
        if self.spaces.read().map_err(poisoned)?.contains_key(&id) {
            drop(creation);
            for e in entries {
                self.apply_remote(&id, e, None).await?;
            }
            return Ok(());
        }
        let blobs = self.blobs.clone();
        let entries = entries.to_vec();
        let (space, history, creation) = disk(move || {
            let lookup = |hash: &Hash| read_blob(&blobs, hash).ok().flatten();
            let (space, applied) = Space::replay(id, entries.iter().map(Vec::as_slice), &lookup)?;
            Ok((space, (entries, applied), creation))
        })
        .await?;
        let (slot, creation) = self
            .install_log(space, Some(home.to_string()), history, None, creation)
            .await?;
        let _writer = slot.writer.lock().await;
        drop(creation);
        self.finish_pending(&slot).await
    }

    async fn install_log(
        &self,
        space: Space,
        home: Option<String>,
        (entries, applied): (Vec<Vec<u8>>, Vec<Applied>),
        ordered_here: Option<bool>,
        creation: OwnedMutexGuard<()>,
    ) -> Result<(Arc<SpaceSlot>, OwnedMutexGuard<()>)> {
        let root = self.dir.join("logs");
        let spaces = Arc::clone(&self.spaces);
        let node_lock = Arc::clone(&self.lock);
        disk(move || {
            let _node_lock = node_lock;
            let id = space.id;
            let stage = tempfile::Builder::new()
                .prefix(".space-")
                .tempdir_in(&root)?;
            let mut log = SpaceLog::create(stage.path(), space, home)?;
            for entry in entries {
                log.append(&entry)?;
            }
            log.seal()?;
            for entry in applied {
                log.enqueue(entry, ordered_here);
            }
            let dir = root.join(id.to_hex());
            logs::sync_directory(stage.path())?;
            std::fs::rename(stage.path(), &dir)?;
            log.relocate(dir);
            let slot = Arc::new(SpaceSlot::new(log));
            spaces
                .write()
                .map_err(poisoned)?
                .insert(id, Arc::clone(&slot));
            logs::sync_directory(&root)?;
            Ok((slot, creation))
        })
        .await
    }

    /// Apply a remote entry and return the last committed sequence.
    ///
    /// Already-committed entries resume any outstanding maintenance.
    ///
    /// # Errors
    ///
    /// Returns an error if the entry is invalid or cannot be stored or indexed.
    pub async fn apply_remote(
        &self,
        id: &Hash,
        entry: &[u8],
        payload: Option<&[u8]>,
    ) -> Result<u64> {
        if let Some(p) = payload {
            self.put_blob_async(p.to_vec()).await?;
        }
        let slot = self.space(id)?;
        let writer = Arc::clone(&slot.writer).lock_owned().await;
        self.finish_pending(&slot).await?;
        let seq = hs_core::cose::Sign1::decode(entry)
            .ok()
            .and_then(|m| hs_core::event::Entry::decode(&m.payload).ok())
            .map(|e| e.seq)
            .ok_or_else(|| Error::refused("not an entry".into()))?;
        let entry = entry.to_vec();
        let working = Arc::clone(&slot);
        let dir = self.blobs.clone();
        let node_lock = Arc::clone(&self.lock);
        let (_writer, seq) = disk(move || {
            let _node_lock = node_lock;
            let mut log = working.log.blocking_lock();
            if seq > log.space.seq {
                log.apply(&entry, &|hash: &Hash| read_blob(&dir, hash).ok().flatten())?;
            }
            Ok((writer, log.space.seq))
        })
        .await?;
        self.finish_pending(&slot).await?;
        Ok(seq)
    }

    /// Order a signed proposal and return its sequence and entry hash.
    ///
    /// Repeating a committed author and nonce returns the original result.
    ///
    /// # Errors
    ///
    /// Returns an error if this node is not the primary, the proposal is invalid,
    /// or persistence or indexing fails. Retrying resumes pending maintenance.
    pub async fn submit(
        &self,
        id: &Hash,
        signed_proposal: &[u8],
        payload: Option<&[u8]>,
    ) -> Result<(u64, Hash)> {
        self.submit_checked(id, signed_proposal, payload, |_| Ok(()))
            .await
    }

    /// Submit a proposal with a version check at the primary's commit boundary.
    ///
    /// # Errors
    ///
    /// Returns the submission error or a failed version check.
    pub async fn submit_checked(
        &self,
        id: &Hash,
        signed_proposal: &[u8],
        payload: Option<&[u8]>,
        check: impl FnOnce(Option<Hash>) -> Result<()> + Send + 'static,
    ) -> Result<(u64, Hash)> {
        let committed = self.commit(id, signed_proposal, payload, check).await?;
        Ok((committed.seq, committed.entry))
    }

    /// Order a proposal and return the authoritative payload and creation status.
    ///
    /// # Errors
    ///
    /// Returns an error if the proposal conflicts with an existing commit, fails its
    /// version check, or cannot be validated, stored or indexed.
    pub async fn commit(
        &self,
        id: &Hash,
        signed_proposal: &[u8],
        payload: Option<&[u8]>,
        check: impl FnOnce(Option<Hash>) -> Result<()> + Send + 'static,
    ) -> Result<Committed> {
        if let Some(p) = payload {
            self.put_blob_async(p.to_vec()).await?;
        }
        let slot = self.space(id)?;
        let writer = Arc::clone(&slot.writer).lock_owned().await;
        self.finish_pending(&slot).await?;
        let working = Arc::clone(&slot);
        let dir = self.blobs.clone();
        let key = self.key.clone();
        let id = *id;
        let signed_proposal = signed_proposal.to_vec();
        // Keep the writer lock in the worker even if the request is cancelled.
        let node_lock = Arc::clone(&self.lock);
        let (_writer, out) = disk(move || {
            let _node_lock = node_lock;
            let mut log = working.log.blocking_lock();
            if log.space.primary != key.public() {
                return Err(Error::new(
                    ErrorKind::NotPrimary {
                        space: id,
                        home: log.home.clone(),
                    },
                    format!(
                        "not the primary of {id}; send to {}",
                        log.home.as_deref().unwrap_or("its primary")
                    ),
                ));
            }
            let lookup = |hash: &Hash| read_blob(&dir, hash).ok().flatten();
            let out = log.order(&key, &signed_proposal, &lookup, now_ms(), check)?;
            Ok((writer, out))
        })
        .await?;
        self.finish_pending(&slot).await?;
        Ok(out)
    }

    /// Read a committed result and verify that it matches the submitted proposal.
    ///
    /// # Errors
    ///
    /// Returns an error if the commit is missing, malformed or for a different request.
    pub async fn committed(&self, id: &Hash, seq: u64, signed: &[u8]) -> Result<Committed> {
        self.with_space(id, |log| log.committed(seq, signed))
            .await?
    }

    /// Find a prior commit for an authenticated API request on this node.
    ///
    /// # Errors
    ///
    /// Returns an error if the caller lacks write access or the key names a different request.
    pub async fn replay_request(
        &self,
        access: &Access,
        id: &Hash,
        target: &str,
        write: &WriteId,
    ) -> Result<Option<Committed>> {
        access.require(id, Role::Writer, target)?;
        let slot = self.space(id)?;
        let _writer = slot.writer.lock().await;
        self.finish_pending(&slot).await?;
        slot.log.lock().await.replay(self.key.public(), write)
    }

    // The slot's writer lock lets us release the log while removing blobs
    // without allowing another writer to overtake the pending index update.
    async fn finish_pending(&self, slot: &Arc<SpaceSlot>) -> Result<()> {
        let mut log = slot.log.lock().await;
        if log.pending.is_empty() {
            return Ok(());
        }
        if log.has_unsealed_segment() {
            drop(log);
            let working = Arc::clone(slot);
            let node_lock = Arc::clone(&self.lock);
            disk(move || {
                let _node_lock = node_lock;
                working.log.blocking_lock().seal()
            })
            .await?;
            log = slot.log.lock().await;
        }
        while let Some(pending) = log.pending.front() {
            if !pending.indexed {
                self.index_applied(&log.space, &pending.applied).await?;
                if let Some(pending) = log.pending.front_mut() {
                    pending.indexed = true;
                }
            }
            if let Some(erase) = log.pending.front().map(|p| &p.applied.erase)
                && !erase.is_empty()
            {
                let erase = erase.clone();
                drop(log);
                self.erase_blobs(&erase).await?;
                log = slot.log.lock().await;
            }
            if let Some(pending) = log.pending.pop_front()
                && let Some(ordered_here) = pending.ordered_here
                && let Some(entry) = usize::try_from(pending.applied.seq)
                    .ok()
                    .and_then(|seq| log.entries.get(seq))
            {
                self.emit(&log.space.id, &pending.applied, entry.clone(), ordered_here);
            }
        }
        Ok(())
    }

    /// Write on behalf of a caller: this node signs the proposal, recording
    /// the grant that allowed it. If this node is not the primary, the signed
    /// proposal is returned in [`ErrorKind::NotPrimary`]'s place by
    /// [`Node::propose`]; callers forward it.
    ///
    /// # Errors
    ///
    /// Returns an error if access is denied, another node is primary, or the operation cannot be
    /// completed.
    #[expect(
        clippy::too_many_arguments,
        reason = "the signature carries the signed proposal fields"
    )]
    pub async fn write(
        &self,
        access: &Access,
        id: &Hash,
        kind: Kind,
        target: &str,
        payload: Option<&[u8]>,
        media_type: &str,
        nonce: Option<Vec<u8>>,
    ) -> Result<(u64, Hash)> {
        let signed = self.propose(access, id, kind, target, payload, media_type, nonce)?;
        self.submit(id, &signed, payload).await
    }

    /// Check the caller may do this and build the signed proposal.
    ///
    /// # Errors
    ///
    /// Returns an error if the target is invalid or the caller lacks permission.
    #[expect(
        clippy::too_many_arguments,
        reason = "the signature carries the signed proposal fields"
    )]
    pub fn propose(
        &self,
        access: &Access,
        id: &Hash,
        kind: Kind,
        target: &str,
        payload: Option<&[u8]>,
        media_type: &str,
        nonce: Option<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        self.proposal(
            access,
            id,
            kind,
            target,
            (payload, media_type),
            (nonce, None),
        )
    }

    /// Sign a write with the fingerprint of its original API request.
    ///
    /// # Errors
    ///
    /// Returns an error if the target is invalid or the caller lacks permission.
    pub fn propose_request(
        &self,
        access: &Access,
        id: &Hash,
        kind: Kind,
        target: &str,
        content: (Option<&[u8]>, &str),
        write: Option<WriteId>,
    ) -> Result<Vec<u8>> {
        let (nonce, request) = write.map_or((None, None), |write| {
            (Some(write.nonce), Some(write.request))
        });
        self.proposal(access, id, kind, target, content, (nonce, request))
    }

    fn proposal(
        &self,
        access: &Access,
        id: &Hash,
        kind: Kind,
        target: &str,
        (payload, media_type): (Option<&[u8]>, &str),
        (nonce, request): (Option<Vec<u8>>, Option<Hash>),
    ) -> Result<Vec<u8>> {
        if !hs_core::valid_target(target) {
            return Err(Error::refused(format!("bad target {target:?}")));
        }
        let needed = match kind {
            Kind::Put | Kind::Delete | Kind::Erase => Role::Writer,
            Kind::Mls => Role::Reader,
            _ => Role::Owner,
        };
        access.require(id, needed, target)?;
        let nonce = nonce.unwrap_or_else(|| random::<16>().to_vec());
        let proposal = Proposal {
            space: *id,
            kind,
            target: target.to_owned(),
            payload: payload.map(Hash::of),
            media_type: media_type.to_owned(),
            time: now_ms(),
            nonce,
            grant: access.grant_id(),
            request,
        };
        Ok(self.key.sign1(proposal.encode()).encode())
    }

    /// The current version of a resource and its bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the space is missing or its blob cannot be read.
    pub async fn get(
        &self,
        id: &Hash,
        target: &str,
    ) -> Result<Option<(hs_space::Resource, Option<Vec<u8>>)>> {
        let r = self
            .with_space(id, |l| l.space.resources.get(target).cloned())
            .await?;
        let Some(r) = r else { return Ok(None) };
        let bytes = match r.payload {
            Some(h) => self.blob_async(&h).await?,
            None => None,
        };
        Ok(Some((r, bytes)))
    }

    /// Entries after `after` (exclusive), at most `limit`.
    ///
    /// # Errors
    ///
    /// Returns an error if the space is missing or its state cannot be read.
    pub async fn entries(
        &self,
        id: &Hash,
        after: Option<u64>,
        limit: usize,
    ) -> Result<Vec<Vec<u8>>> {
        self.with_space(id, |l| {
            let start = after.map_or(0, |a| {
                usize::try_from(a).unwrap_or(usize::MAX).saturating_add(1)
            });
            l.entries.iter().skip(start).take(limit).cloned().collect()
        })
        .await
    }

    /// Whether a payload belongs to a space (so its members may read it).
    pub async fn space_has_payload(&self, id: &Hash, hash: &Hash) -> bool {
        self.with_space(id, |l| l.payloads.contains(hash))
            .await
            .unwrap_or(false)
    }

    /// Delete erased payloads that no resource in any space still uses.
    async fn erase_blobs(&self, erase: &[Hash]) -> Result<()> {
        if erase.is_empty() {
            return Ok(());
        }
        let mut used = std::collections::HashSet::new();
        for id in self.space_ids() {
            self.with_space(&id, |l| {
                for r in l.space.resources.values() {
                    used.extend(r.history.iter().copied().chain(r.payload));
                }
            })
            .await?;
        }
        for hash in erase.iter().filter(|hash| !used.contains(*hash)) {
            self.blobs.delete(&blob_name(hash))?;
        }
        Ok(())
    }

    async fn index_applied(&self, space: &Space, a: &Applied) -> Result<()> {
        let space_hex = space.id.to_hex();
        let bytes = match a.proposal.payload {
            Some(hash) => self.blob_async(&hash).await?,
            None => None,
        };
        if a.proposal.kind == Kind::Genesis {
            self.index
                .set_space(&space_hex, space.name(), a.seq)
                .await?;
        }
        self.index
            .apply(&Change {
                space: &space_hex,
                seq: a.seq,
                time: a.time,
                kind: a.proposal.kind.as_str(),
                target: &a.proposal.target,
                payload: a.proposal.payload.map(|h| h.to_hex()),
                media_type: &a.proposal.media_type,
                bytes: bytes.as_deref(),
            })
            .await?;
        Ok(())
    }

    fn emit(&self, id: &Hash, a: &Applied, entry: Vec<u8>, ordered_here: bool) {
        let _ = self.events.send(Event {
            space: *id,
            seq: a.seq,
            kind: a.proposal.kind,
            target: a.proposal.target.clone(),
            entry: Arc::new(entry),
            ordered_here,
        });
    }

    /// Delete the index and rebuild it from the logs.
    ///
    /// # Errors
    ///
    /// Returns an error if a running node holds the directory lock or the index cannot be rebuilt.
    pub async fn rebuild_index(dir: &Path) -> Result<()> {
        let lock = std::fs::File::create(dir.join("node.lock"))?;
        if lock.try_lock().is_err() {
            return Err(Error::refused(format!(
                "another hs process is using {}",
                dir.display()
            )));
        }
        drop(lock);
        for s in ["", "-wal", "-shm"] {
            let _ = std::fs::remove_file(dir.join(format!("index.db{s}")));
        }
        Node::open(dir).await?.index.close().await;
        Ok(())
    }
}

/// Derive the storage key from the personal seed, so a recovery code is
/// enough to decrypt replicas.
pub fn storage_key(personal_seed: &[u8; 32]) -> [u8; 32] {
    blake3::derive_key("homeserver 2025 storage key v1", personal_seed)
}

fn parse_config(raw: &Json) -> Result<Config> {
    let get = |k: &str| raw.get(k).and_then(Json::as_str);
    Ok(Config {
        identity: get("identity")
            .and_then(PublicKey::from_id)
            .ok_or_else(|| Error::io("config.json: identity".into()))?,
        private: get("private")
            .and_then(Hash::from_hex)
            .ok_or_else(|| Error::io("config.json: private".into()))?,
        name: get("name").unwrap_or("").to_string(),
        url: get("url").map(String::from),
        raw: raw.clone(),
    })
}

fn write_secret(path: &Path, s: &str) -> Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600)
    };
    let mut f = opts.open(path)?;
    f.write_all(s.as_bytes())?;
    f.sync_all()?;
    Ok(())
}

fn poisoned<T>(_: T) -> Error {
    Error::io("lock poisoned".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn temp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("hs-node-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[tokio::test]
    async fn init_write_reopen_rebuild() {
        let dir = temp("a");
        let init = Node::init(&dir, "Ada", None).await.unwrap();
        let node = init.node;
        assert!(hs_identity::parse_recovery_code(&init.recovery_code).is_some());
        let owner = node.authenticate(&init.owner_token).await.unwrap();
        let notes = node.create_space("notes", json!({})).await.unwrap();
        let body = r#"{"title":"第一篇","text":"你好世界"}"#.as_bytes();
        let (seq, _) = node
            .write(
                &owner,
                &notes,
                Kind::Put,
                "note/1",
                Some(body),
                "application/json",
                None,
            )
            .await
            .unwrap();
        assert_eq!(seq, 1);
        // Idempotent with the same nonce.
        let n = Some(vec![7; 16]);
        let a = node
            .write(
                &owner,
                &notes,
                Kind::Put,
                "note/2",
                Some(b"{}"),
                "application/json",
                n.clone(),
            )
            .await
            .unwrap();
        let b = node
            .write(
                &owner,
                &notes,
                Kind::Put,
                "note/2",
                Some(b"{}"),
                "application/json",
                n,
            )
            .await
            .unwrap();
        assert_eq!(a.0, b.0);
        let spaces: Vec<String> = node.space_ids().iter().map(Hash::to_hex).collect();
        assert_eq!(
            node.index.search("世界", &spaces, 10).await.unwrap().len(),
            1
        );

        // A reader token cannot write.
        let (tok, _) = node
            .create_token(Role::Reader, Some(vec![notes]), "reader", None)
            .await
            .unwrap();
        let reader = node.authenticate(&tok).await.unwrap();
        node.write(
            &reader,
            &notes,
            Kind::Put,
            "note/3",
            Some(b"{}"),
            "application/json",
            None,
        )
        .await
        .unwrap_err();
        assert!(reader.can_read(&notes));
        assert!(!reader.can_read(&node.config().private));

        // Erase removes the payload.
        let h = Hash::of(body);
        node.write(&owner, &notes, Kind::Erase, "note/1", None, "", None)
            .await
            .unwrap();
        assert!(node.blob(&h).unwrap().is_none());
        let dump = node.index.dump().await.unwrap();
        node.index.close().await;
        drop(node);

        // Reopen: same state; rebuild: same index.
        let node = Node::open(&dir).await.unwrap();
        assert_eq!(node.with_space(&notes, |l| l.space.seq).await.unwrap(), 3);
        assert!(node.authenticate(&tok).await.is_some());
        node.index.close().await;
        drop(node);
        Node::rebuild_index(&dir).await.unwrap();
        let node = Node::open(&dir).await.unwrap();
        assert_eq!(node.index.dump().await.unwrap(), dump);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
