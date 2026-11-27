// SPDX-License-Identifier: AGPL-3.0-only

//! The fileOS app runtime, on wasmtime.
//!
//! - A [`Bundle`] is an `.app` folder: a `manifest.toml`, components in `bin/`
//!   and other files. It can come from a directory on disk or from a fileOS
//!   folder.
//! - An [`AppInstance`] runs an app component. Every call it makes goes to an
//!   [`AppHost`], which checks it against the grants of the app's key. Each
//!   call into an app has limits on fuel and memory.
//! - An [`HttpApp`] gives an app the HTTP requests for its origin, with
//!   `wasi:http`. Each request gets a new instance, with limits on fuel,
//!   memory and time. Apps get only the WASI interfaces of `wasi:http/proxy`:
//!   no files, no sockets, and no HTTP requests of their own.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use fileos_core::{Error, Hash};
use serde::Deserialize;
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable};
use wasmtime::{Config, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

mod http;
mod pack;

#[doc(inline)]
pub use http::{Answer, Body, HttpApp};

mod bindings {
    pub mod app {
        wasmtime::component::bindgen!({
            path: "../../wit",
            world: "app",
            require_store_data_send: true,
            with: {
                "wasi:http": wasmtime_wasi_http::p2::bindings::http,
                "wasi:io": wasmtime_wasi::p2::bindings::io,
                "wasi:clocks": wasmtime_wasi::p2::bindings::clocks,
            },
        });
    }
}

use bindings::app::fileos::app::{
    access as wit_access, fs as wit_fs, kv as wit_kv, log as wit_log,
};

/// Fuel for one call into an app.
const APP_FUEL: u64 = 2_000_000_000;
/// Memory for one app instance.
const APP_MEMORY: usize = 256 << 20;

/// What kind of component a bundle has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    App,
}

/// One permission that an app asks for.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Permission {
    pub path: String,
    /// `read`, `write`, `append` or `read-write`.
    pub access: String,
    /// The key of the text that tells the user why, in `resources/locales/`.
    #[serde(default)]
    pub reason: String,
}

/// What happened to a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    /// It was not there, and now it is.
    Made,
    /// It has a new version.
    Changed,
    /// It was there, and now it is not.
    Deleted,
}

/// A path that an app watches, with options, as `fs.watch` and inotify have.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Watch {
    pub path: String,
    /// Also the paths under it. On when not given.
    #[serde(default = "yes")]
    pub recursive: bool,
    /// Only names that match one of these patterns (`*` is any text, `?` is
    /// one letter), such as `*.jpg`. All names when empty.
    #[serde(default)]
    pub filter: Vec<String>,
    /// Only these kinds of change. All kinds when empty.
    #[serde(default)]
    pub kinds: Vec<ChangeKind>,
}

fn yes() -> bool {
    true
}

impl Watch {
    /// Says if this watch wants to hear of `kind` at `path`.
    pub fn wants(&self, path: &str, kind: ChangeKind) -> bool {
        let rest = if self.path == "/" {
            path.strip_prefix('/')
        } else if path == self.path {
            Some("")
        } else {
            path.strip_prefix(&self.path)
                .and_then(|r| r.strip_prefix('/'))
        };
        let under = rest.is_some_and(|r| self.recursive || !r.contains('/'));
        let name = path.rsplit('/').next().unwrap_or(path);
        under
            && (self.kinds.is_empty() || self.kinds.contains(&kind))
            && (self.filter.is_empty() || self.filter.iter().any(|f| glob(f, name)))
    }
}

/// Says if `name` matches `pattern`: `*` is any text, `?` one letter.
pub fn glob(pattern: &str, name: &str) -> bool {
    let (p, n): (Vec<char>, Vec<char>) = (pattern.chars().collect(), name.chars().collect());
    let (mut pi, mut ni, mut star, mut mark) = (0, 0, None, 0);
    while ni < n.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ni;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ni = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

/// What an app may ask the person for at run time (see `fileos:app/access`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Ask {
    /// Exact permissions or templates, with commas: `read`, `files.read`.
    pub permissions: String,
    /// The key of the text that tells the person why, in `resources/locales/`.
    #[serde(default)]
    pub reason: String,
}

/// Files that an app takes when other apps share them (see
/// `fileos:app/access.share`).
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Receive {
    /// Name patterns, such as `*.jpg`. Any file when empty.
    #[serde(default)]
    pub filter: Vec<String>,
}

/// Where the kernel sends the request for a schedule of an app:
/// `POST /.well-known/fileos/schedule/<name>`, with `fileos-event: schedule`.
/// Requests from outside never reach paths under `/.well-known/fileos/`.
pub const SCHEDULE_PATH: &str = "/.well-known/fileos/schedule/";

/// Work that an app wants done from time to time, as Cloudflare's Cron
/// Triggers. The kernel sends a request to the app's own `wasi:http`
/// handler, so it needs nothing new in the app interface.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Schedule {
    pub name: String,
    /// How often: a number and `s`, `m`, `h` or `d`, at least one minute.
    pub every: String,
}

impl Schedule {
    /// How often, in seconds.
    ///
    /// # Errors
    ///
    /// When `every` is not a number and a unit, or is less than a minute.
    pub fn seconds(&self) -> Result<u64, Error> {
        let bad = || Error::invalid(format!("not a time for a schedule: {:?}", self.every));
        let unit = self.every.chars().last().ok_or_else(bad)?;
        let n: u64 = self.every[..self.every.len() - unit.len_utf8()]
            .parse()
            .ok()
            .ok_or_else(bad)?;
        let seconds = match unit {
            's' => n,
            'm' => n * 60,
            'h' => n * 3600,
            'd' => n * 86400,
            _ => return Err(bad()),
        };
        if seconds < 60 {
            return Err(Error::invalid(format!(
                "a schedule runs at most once a minute: {:?}",
                self.every
            )));
        }
        Ok(seconds)
    }
}

/// `manifest.toml` of an `.app` folder.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    pub id: String,
    pub name: String,
    pub kind: Kind,
    pub version: String,
    pub build: u64,
    pub requires: String,
    pub main: String,
    #[serde(default)]
    pub permissions: Vec<Permission>,
    #[serde(default)]
    pub watch: Vec<Watch>,
    #[serde(default)]
    pub schedule: Vec<Schedule>,
    /// The most the app may ask for at run time, through the picker.
    #[serde(default)]
    pub asks: Vec<Ask>,
    /// What other apps may share with this app.
    #[serde(default)]
    pub receives: Vec<Receive>,
}

/// The WIT version this runtime gives.
pub const WIT_VERSION: &str = "fileos:app@0.1.0";

/// An `.app` folder: its manifest and its files, by path.
#[derive(Clone, Debug)]
pub struct Bundle {
    manifest: Manifest,
    files: BTreeMap<String, Vec<u8>>,
}

impl Bundle {
    /// Reads a bundle from its files. Paths are like `"/manifest.toml"`.
    ///
    /// # Errors
    ///
    /// When there is no valid manifest, or the main component is missing.
    pub fn from_files(files: BTreeMap<String, Vec<u8>>) -> Result<Self, Error> {
        let text = files
            .get("/manifest.toml")
            .ok_or_else(|| Error::invalid("an .app folder needs a manifest.toml".into()))?;
        let text = std::str::from_utf8(text)
            .map_err(|e| Error::invalid(format!("manifest.toml is not UTF-8: {e}")))?;
        let manifest: Manifest =
            toml::from_str(text).map_err(|e| Error::invalid(format!("manifest.toml: {e}")))?;
        if manifest.requires != WIT_VERSION {
            return Err(Error::invalid(format!(
                "{} needs {}, but this runtime gives {WIT_VERSION}",
                manifest.id, manifest.requires
            )));
        }
        if !files.contains_key(&format!("/{}", manifest.main)) {
            return Err(Error::invalid(format!(
                "{} is not in the .app folder",
                manifest.main
            )));
        }
        for schedule in &manifest.schedule {
            schedule.seconds()?;
            if schedule.name.is_empty()
                || !schedule
                    .name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                return Err(Error::invalid(format!(
                    "a schedule name has letters, digits, - and _: {:?}",
                    schedule.name
                )));
            }
        }
        Ok(Self { manifest, files })
    }

    /// Reads a bundle from a directory on disk.
    ///
    /// # Errors
    ///
    /// When the directory cannot be read, or [`Bundle::from_files`] fails.
    pub fn read_dir(dir: &Path) -> Result<Self, Error> {
        fn walk(
            dir: &Path,
            prefix: &str,
            out: &mut BTreeMap<String, Vec<u8>>,
        ) -> std::io::Result<()> {
            for entry in std::fs::read_dir(dir)? {
                let entry = entry?;
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = format!("{prefix}/{name}");
                if entry.file_type()?.is_dir() {
                    walk(&entry.path(), &path, out)?;
                } else {
                    out.insert(path, std::fs::read(entry.path())?);
                }
            }
            Ok(())
        }
        let mut files = BTreeMap::new();
        walk(dir, "", &mut files).map_err(|e| Error::invalid(format!("{}: {e}", dir.display())))?;
        Self::from_files(files)
    }

    /// The manifest checked when this bundle was opened.
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// The paths and bytes of all files in the bundle.
    pub fn files(&self) -> impl Iterator<Item = (&str, &[u8])> {
        self.files
            .iter()
            .map(|(path, data)| (path.as_str(), data.as_slice()))
    }

    /// The main component named in the manifest.
    ///
    /// # Panics
    ///
    /// If the bundle's internal check that the main file exists is broken.
    pub fn main(&self) -> &[u8] {
        &self.files[&format!("/{}", self.manifest.main)]
    }

    /// The hash of everything in the bundle. It changes when any file changes.
    pub fn hash(&self) -> Hash {
        Hash::of_value(
            &self
                .files
                .iter()
                .map(|(p, d)| (p, Hash::of(d)))
                .collect::<Vec<_>>(),
        )
    }
}

/// One wasmtime engine for all components of a node.
#[derive(Clone)]
pub struct Runtime {
    engine: wasmtime::Engine,
    requests: Arc<tokio::sync::Semaphore>,
    host_workers: Arc<tokio::sync::Semaphore>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Runtime")
    }
}

impl Runtime {
    /// # Errors
    ///
    /// When wasmtime cannot start on this machine.
    pub fn new() -> Result<Self, Error> {
        let mut config = Config::new();
        config.consume_fuel(true);
        let engine = wasmtime::Engine::new(&config).map_err(wasm_error)?;
        Ok(Self {
            engine,
            // Each request has its own Wasm store. Bound their total cost.
            requests: Arc::new(tokio::sync::Semaphore::new(8)),
            // Some host calls wait for disk or the node's state lock.
            host_workers: Arc::new(tokio::sync::Semaphore::new(4)),
        })
    }

    fn component(&self, bundle: &Bundle, kind: Kind) -> Result<Component, Error> {
        if bundle.manifest.kind != kind {
            return Err(Error::invalid(
                format!("{} is not an {kind:?}", bundle.manifest.id).to_lowercase(),
            ));
        }
        Component::new(&self.engine, bundle.main()).map_err(wasm_error)
    }

    /// Starts an app. Its calls go to `host`.
    ///
    /// # Errors
    ///
    /// When the bundle is not an app, or its component does not start.
    pub fn app(&self, bundle: &Bundle, host: Host) -> Result<AppInstance, Error> {
        let component = self.component(bundle, Kind::App)?;
        let mut linker = Linker::new(&self.engine);
        add_fileos_to_linker(&mut linker)?;
        wasmtime_wasi_http::p2::add_to_linker_sync(&mut linker).map_err(wasm_error)?;
        let pre =
            bindings::app::AppPre::new(linker.instantiate_pre(&component).map_err(wasm_error)?)
                .map_err(wasm_error)?;
        let app = AppInstance {
            manifest: bundle.manifest.clone(),
            host,
            host_workers: Arc::clone(&self.host_workers),
            pre,
        };
        // Start it once now, so a broken app is found at install.
        app.instance()?;
        Ok(app)
    }
}

fn wasm_error(e: wasmtime::Error) -> Error {
    Error::internal(format!("{e:#}")).with_source(e.into_boxed_dyn_error())
}

/// An error from the file system, as an app sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsError {
    Denied(String),
    NotFound,
    Invalid(String),
    Unavailable(String),
}

impl std::fmt::Display for FsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Denied(message) => write!(f, "not allowed: {message}"),
            Self::NotFound => f.write_str("not found"),
            Self::Invalid(message) | Self::Unavailable(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for FsError {}

impl From<FsError> for Error {
    fn from(error: FsError) -> Self {
        let message = error.to_string();
        match error {
            FsError::Denied(_) => Self::denied(message),
            FsError::NotFound => Self::not_found(message),
            FsError::Invalid(_) => Self::invalid(message),
            FsError::Unavailable(_) => Self::unavailable(message),
        }
        .with_source(error)
    }
}

impl From<Error> for FsError {
    fn from(error: Error) -> Self {
        if error.is_denied() {
            Self::Denied(error.to_string())
        } else if error.is_not_found() {
            Self::NotFound
        } else if error.is_invalid() {
            Self::Invalid(error.to_string())
        } else {
            Self::Unavailable("the file system could not finish the request".into())
        }
    }
}

/// What an app may call. The node gives this; `app` is the ID of the app.
pub trait AppHost: Send + Sync {
    /// # Errors
    ///
    /// See [`FsError`].
    fn read(&self, app: &str, path: &str) -> Result<Vec<u8>, FsError>;
    /// # Errors
    ///
    /// See [`FsError`].
    fn write(&self, app: &str, path: &str, data: Vec<u8>) -> Result<(), FsError>;
    /// # Errors
    ///
    /// See [`FsError`].
    fn append(&self, app: &str, path: &str, data: Vec<u8>) -> Result<(), FsError>;
    /// # Errors
    ///
    /// See [`FsError`].
    fn delete(&self, app: &str, path: &str) -> Result<(), FsError>;
    /// # Errors
    ///
    /// See [`FsError`].
    fn list(&self, app: &str, path: &str) -> Result<Vec<String>, FsError>;
    fn kv_get(&self, app: &str, key: &str) -> Option<Vec<u8>>;
    fn kv_set(&self, app: &str, key: &str, value: Vec<u8>);
    fn kv_delete(&self, app: &str, key: &str);
    fn kv_keys(&self, app: &str, prefix: &str) -> Vec<String>;
    fn log(&self, app: &str, message: &str);
    /// Files a request of the app for access, for the person to answer.
    /// Gives its ID.
    ///
    /// # Errors
    ///
    /// When the app may not ask for this, or the node has no picker.
    fn ask(&self, app: &str, purpose: &str, permissions: &str) -> Result<String, String> {
        let _ = (app, purpose, permissions);
        Err("this node cannot ask the person".into())
    }
    /// Files an offer of a file by the app to another app, for the person
    /// to answer. Gives its ID.
    ///
    /// # Errors
    ///
    /// When the app may not share the file, or the node cannot ask.
    fn share(&self, app: &str, path: &str) -> Result<String, String> {
        let _ = (app, path);
        Err("this node cannot ask the person".into())
    }
}

/// A shared host that handles calls made by an app.
#[derive(Clone)]
pub struct Host(Arc<dyn AppHost>);

impl Host {
    /// Makes a host from its implementation.
    pub fn new(host: impl AppHost + 'static) -> Self {
        Self(Arc::new(host))
    }
}

impl<T: AppHost + 'static> From<Arc<T>> for Host {
    fn from(host: Arc<T>) -> Self {
        Self(host)
    }
}

impl std::fmt::Debug for Host {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Host")
    }
}

struct AppCtx {
    app: String,
    host: Host,
    host_workers: Arc<tokio::sync::Semaphore>,
    limits: StoreLimits,
    table: ResourceTable,
    wasi: WasiCtx,
    http: WasiHttpCtx,
    hooks: NoOutgoingHttp,
}

/// The `wasi:http` hooks of an app. An app may not send HTTP requests of its
/// own, so `send_request` gives an error, as it does with no hooks at all.
struct NoOutgoingHttp;

impl wasmtime_wasi_http::WasiHttpHooks for NoOutgoingHttp {
    fn send_request(
        &mut self,
        _request: hyper::Request<wasmtime_wasi_http::p2::body::HyperOutgoingBody>,
        _options: Option<wasmtime_wasi_http::RequestOptions>,
        _done: Box<dyn Future<Output = Result<(), wasmtime_wasi_http::Error>> + Send>,
    ) -> Box<dyn Future<Output = OutgoingResult> + Send> {
        Box::new(std::future::ready(Err(
            wasmtime_wasi_http::Error::HttpRequestDenied,
        )))
    }
}

/// What a `send_request` gives back: a response, and a future for the end
/// of its body.
type OutgoingResult = Result<
    (
        hyper::Response<wasmtime_wasi_http::p2::body::HyperIncomingBody>,
        Box<dyn Future<Output = Result<(), wasmtime_wasi_http::Error>> + Send>,
    ),
    wasmtime_wasi_http::Error,
>;

impl AppCtx {
    fn new(app: &str, host: Host, host_workers: Arc<tokio::sync::Semaphore>) -> Self {
        Self {
            app: app.to_string(),
            host,
            host_workers,
            limits: StoreLimitsBuilder::new().memory_size(APP_MEMORY).build(),
            table: ResourceTable::new(),
            // No files, no environment, no network, and nothing on the
            // standard streams of the node.
            wasi: WasiCtx::builder()
                .allow_tcp(false)
                .allow_udp(false)
                .allow_ip_name_lookup(false)
                .build(),
            http: WasiHttpCtx::new(),
            hooks: NoOutgoingHttp,
        }
    }
}

impl WasiView for AppCtx {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for AppCtx {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: &mut self.hooks,
        }
    }
}

/// Adds the fileOS interfaces of the `app` world to a linker.
fn add_fileos_to_linker(linker: &mut Linker<AppCtx>) -> Result<(), Error> {
    use bindings::app::fileos::app;
    app::fs::add_to_linker::<AppCtx, HasSelf<AppCtx>>(linker, |c| c).map_err(wasm_error)?;
    app::kv::add_to_linker::<AppCtx, HasSelf<AppCtx>>(linker, |c| c).map_err(wasm_error)?;
    app::log::add_to_linker::<AppCtx, HasSelf<AppCtx>>(linker, |c| c).map_err(wasm_error)?;
    app::access::add_to_linker::<AppCtx, HasSelf<AppCtx>>(linker, |c| c).map_err(wasm_error)?;
    Ok(())
}

fn to_wit(e: FsError) -> wit_fs::Error {
    match e {
        FsError::Denied(m) => wit_fs::Error::Denied(m),
        FsError::NotFound => wit_fs::Error::NotFound,
        FsError::Invalid(m) | FsError::Unavailable(m) => wit_fs::Error::Invalid(m),
    }
}

impl wit_fs::Host for AppCtx {
    fn read(&mut self, path: String) -> Result<Vec<u8>, wit_fs::Error> {
        self.host.0.read(&self.app, &path).map_err(to_wit)
    }

    fn write(&mut self, path: String, data: Vec<u8>) -> Result<(), wit_fs::Error> {
        self.host.0.write(&self.app, &path, data).map_err(to_wit)
    }

    fn append(&mut self, path: String, data: Vec<u8>) -> Result<(), wit_fs::Error> {
        self.host.0.append(&self.app, &path, data).map_err(to_wit)
    }

    fn delete(&mut self, path: String) -> Result<(), wit_fs::Error> {
        self.host.0.delete(&self.app, &path).map_err(to_wit)
    }

    fn list_dir(&mut self, path: String) -> Result<Vec<String>, wit_fs::Error> {
        self.host.0.list(&self.app, &path).map_err(to_wit)
    }
}

impl wit_access::Host for AppCtx {
    fn ask(&mut self, purpose: String, permissions: String) -> Result<String, String> {
        self.host.0.ask(&self.app, &purpose, &permissions)
    }

    fn share(&mut self, path: String) -> Result<String, String> {
        self.host.0.share(&self.app, &path)
    }
}

impl wit_kv::Host for AppCtx {
    fn get(&mut self, key: String) -> Option<Vec<u8>> {
        self.host.0.kv_get(&self.app, &key)
    }

    fn set(&mut self, key: String, value: Vec<u8>) {
        self.host.0.kv_set(&self.app, &key, value);
    }

    fn delete(&mut self, key: String) {
        self.host.0.kv_delete(&self.app, &key);
    }

    fn keys(&mut self, prefix: String) -> Vec<String> {
        self.host.0.kv_keys(&self.app, &prefix)
    }
}

impl wit_log::Host for AppCtx {
    fn info(&mut self, message: String) {
        self.host.0.log(&self.app, &message);
    }
}

/// An app, ready for events.
///
/// It is a worker: each event gets a new instance, with its own limits, and
/// nothing stays in memory between events. Data that must stay goes into
/// files, or into the app's cache.
pub struct AppInstance {
    manifest: Manifest,
    host: Host,
    host_workers: Arc<tokio::sync::Semaphore>,
    pre: bindings::app::AppPre<AppCtx>,
}

impl std::fmt::Debug for AppInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AppInstance({})", self.manifest.id)
    }
}

impl AppInstance {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    /// Says if the app watches `path` (for any kind of change).
    pub fn watches(&self, path: &str) -> bool {
        [ChangeKind::Made, ChangeKind::Changed, ChangeKind::Deleted]
            .into_iter()
            .any(|k| self.wants(path, k))
    }

    /// Says if one of the app's watches wants to hear of `kind` at `path`.
    pub fn wants(&self, path: &str, kind: ChangeKind) -> bool {
        self.manifest.watch.iter().any(|w| w.wants(path, kind))
    }

    /// Tells the app that a path changed.
    ///
    /// # Errors
    ///
    /// When the app traps or runs out of fuel or memory.
    pub fn on_change(&mut self, path: &str) -> Result<(), Error> {
        let (mut store, app) = self.instance()?;
        app.fileos_app_events()
            .call_on_change(&mut store, path)
            .map_err(wasm_error)
    }

    /// A new instance, with its own store and limits.
    fn instance(&self) -> Result<(Store<AppCtx>, bindings::app::App), Error> {
        let mut store = Store::new(
            self.pre.engine(),
            AppCtx::new(
                &self.manifest.id,
                self.host.clone(),
                Arc::clone(&self.host_workers),
            ),
        );
        store.limiter(|ctx| &mut ctx.limits);
        store.set_fuel(APP_FUEL).map_err(wasm_error)?;
        let app = self.pre.instantiate(&mut store).map_err(wasm_error)?;
        Ok((store, app))
    }
}
