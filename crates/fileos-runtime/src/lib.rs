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
use wasmtime::component::{Component, HasSelf, Linker, ResourceTable, TypedFunc};
use wasmtime::{Config, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

mod http;
mod pack;

pub use http::{Answer, HttpApp};

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

use bindings::app::fileos::app::{fs as wit_fs, kv as wit_kv, log as wit_log};

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

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Watch {
    pub path: String,
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
        let bad = || Error(format!("not a time for a schedule: {:?}", self.every));
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
            return Err(Error(format!(
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
}

/// The WIT version this runtime gives.
pub const WIT_VERSION: &str = "fileos:app@0.1.0";
const LEGACY_WIT_VERSION: &str = "fos:app@0.1.0";

impl Manifest {
    fn wit_interface(&self, name: &str) -> String {
        let namespace = if self.requires == LEGACY_WIT_VERSION {
            "fos"
        } else {
            "fileos"
        };
        format!("{namespace}:app/{name}@0.1.0")
    }
}

/// An `.app` folder: its manifest and its files, by path.
#[derive(Clone, Debug)]
pub struct Bundle {
    pub manifest: Manifest,
    pub files: BTreeMap<String, Vec<u8>>,
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
            .ok_or_else(|| Error("an .app folder needs a manifest.toml".into()))?;
        let text = std::str::from_utf8(text)
            .map_err(|e| Error(format!("manifest.toml is not UTF-8: {e}")))?;
        let manifest: Manifest =
            toml::from_str(text).map_err(|e| Error(format!("manifest.toml: {e}")))?;
        if !matches!(manifest.requires.as_str(), WIT_VERSION | LEGACY_WIT_VERSION) {
            return Err(Error(format!(
                "{} needs {}, but this runtime gives {WIT_VERSION}",
                manifest.id, manifest.requires
            )));
        }
        if !files.contains_key(&format!("/{}", manifest.main)) {
            return Err(Error(format!(
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
                return Err(Error(format!(
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
        walk(dir, "", &mut files).map_err(|e| Error(format!("{}: {e}", dir.display())))?;
        Self::from_files(files)
    }

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
        Ok(Self { engine })
    }

    fn component(&self, bundle: &Bundle, kind: Kind) -> Result<Component, Error> {
        if bundle.manifest.kind != kind {
            return Err(Error(
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
    pub fn app(&self, bundle: &Bundle, host: Arc<dyn AppHost>) -> Result<AppInstance, Error> {
        let component = self.component(bundle, Kind::App)?;
        let mut store = Store::new(&self.engine, AppCtx::new(&bundle.manifest.id, host));
        store.limiter(|ctx| &mut ctx.limits);
        store.set_fuel(APP_FUEL).map_err(wasm_error)?;
        let mut linker = Linker::new(&self.engine);
        add_app_to_linker(&mut linker, &bundle.manifest)?;
        wasmtime_wasi_http::p2::add_to_linker_sync(&mut linker).map_err(wasm_error)?;
        let events = bundle.manifest.wit_interface("events");
        let on_change = component
            .get_export_index(None, &events)
            .and_then(|events| component.get_export_index(Some(&events), "on-change"))
            .ok_or_else(|| Error(format!("{events} needs an on-change export")))?;
        let instance = linker
            .instantiate(&mut store, &component)
            .map_err(wasm_error)?;
        wasmtime_wasi_http::p2::bindings::sync::Proxy::new(&mut store, &instance)
            .map_err(wasm_error)?;
        let on_change = instance
            .get_typed_func(&mut store, on_change)
            .map_err(wasm_error)?;
        Ok(AppInstance {
            manifest: bundle.manifest.clone(),
            store,
            on_change,
        })
    }
}

fn wasm_error(e: wasmtime::Error) -> Error {
    Error(format!("{e:#}"))
}

/// An error from the file system, as an app sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsError {
    Denied(String),
    NotFound,
    Invalid(String),
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
}

struct AppCtx {
    app: String,
    host: Arc<dyn AppHost>,
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
    fn new(app: &str, host: Arc<dyn AppHost>) -> Self {
        Self {
            app: app.to_string(),
            host,
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
fn add_app_to_linker(linker: &mut Linker<AppCtx>, manifest: &Manifest) -> Result<(), Error> {
    // The old and new names give apps the same host calls and grant checks.
    let mut fs = linker
        .instance(&manifest.wit_interface("fs"))
        .map_err(wasm_error)?;
    wit_fs::add_to_linker_instance::<AppCtx, HasSelf<AppCtx>>(&mut fs, |c| c)
        .map_err(wasm_error)?;
    let mut kv = linker
        .instance(&manifest.wit_interface("kv"))
        .map_err(wasm_error)?;
    wit_kv::add_to_linker_instance::<AppCtx, HasSelf<AppCtx>>(&mut kv, |c| c)
        .map_err(wasm_error)?;
    let mut log = linker
        .instance(&manifest.wit_interface("log"))
        .map_err(wasm_error)?;
    wit_log::add_to_linker_instance::<AppCtx, HasSelf<AppCtx>>(&mut log, |c| c)
        .map_err(wasm_error)?;
    Ok(())
}

fn to_wit(e: FsError) -> wit_fs::Error {
    match e {
        FsError::Denied(m) => wit_fs::Error::Denied(m),
        FsError::NotFound => wit_fs::Error::NotFound,
        FsError::Invalid(m) => wit_fs::Error::Invalid(m),
    }
}

impl wit_fs::Host for AppCtx {
    fn read(&mut self, path: String) -> Result<Vec<u8>, wit_fs::Error> {
        self.host.read(&self.app, &path).map_err(to_wit)
    }

    fn write(&mut self, path: String, data: Vec<u8>) -> Result<(), wit_fs::Error> {
        self.host.write(&self.app, &path, data).map_err(to_wit)
    }

    fn append(&mut self, path: String, data: Vec<u8>) -> Result<(), wit_fs::Error> {
        self.host.append(&self.app, &path, data).map_err(to_wit)
    }

    fn delete(&mut self, path: String) -> Result<(), wit_fs::Error> {
        self.host.delete(&self.app, &path).map_err(to_wit)
    }

    fn list_dir(&mut self, path: String) -> Result<Vec<String>, wit_fs::Error> {
        self.host.list(&self.app, &path).map_err(to_wit)
    }
}

impl wit_kv::Host for AppCtx {
    fn get(&mut self, key: String) -> Option<Vec<u8>> {
        self.host.kv_get(&self.app, &key)
    }

    fn set(&mut self, key: String, value: Vec<u8>) {
        self.host.kv_set(&self.app, &key, value);
    }

    fn delete(&mut self, key: String) {
        self.host.kv_delete(&self.app, &key);
    }

    fn keys(&mut self, prefix: String) -> Vec<String> {
        self.host.kv_keys(&self.app, &prefix)
    }
}

impl wit_log::Host for AppCtx {
    fn info(&mut self, message: String) {
        self.host.log(&self.app, &message);
    }
}

/// A running app.
pub struct AppInstance {
    manifest: Manifest,
    store: Store<AppCtx>,
    on_change: TypedFunc<(String,), ()>,
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

    /// Says if the app watches `path`.
    pub fn watches(&self, path: &str) -> bool {
        self.manifest.watch.iter().any(|w| {
            w.path == "/"
                || path == w.path
                || path
                    .strip_prefix(&w.path)
                    .is_some_and(|r| r.starts_with('/'))
        })
    }

    /// Tells the app that a path changed.
    ///
    /// # Errors
    ///
    /// When the app traps or runs out of fuel or memory.
    pub fn on_change(&mut self, path: &str) -> Result<(), Error> {
        self.store.set_fuel(APP_FUEL).map_err(wasm_error)?;
        self.on_change
            .call(&mut self.store, (path.to_owned(),))
            .map_err(wasm_error)
    }
}

#[cfg(test)]
mod tests;
