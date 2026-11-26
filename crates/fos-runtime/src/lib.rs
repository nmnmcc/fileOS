// SPDX-License-Identifier: AGPL-3.0-only

//! The fileOS app runtime, on wasmtime.
//!
//! - A [`Bundle`] is an `.app` folder: a `manifest.toml`, components in `bin/`
//!   and other files. It can come from a directory on disk or from a fileOS
//!   folder.
//! - An [`EngineInstance`] runs an engine component. It is the [`Authz`] of a
//!   node. The runtime puts no limits on it: it is the owner's choice.
//! - An [`AppInstance`] runs an app component. Every call it makes goes to an
//!   [`AppHost`], which asks the engine. Each call into an app has limits on
//!   fuel and memory.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use fos_core::{Error, Hash, PublicKey};
use fos_fs::{Ability, Authz, Decision};
use serde::Deserialize;
use wasmtime::component::{Component, HasSelf, Linker};
use wasmtime::{Config, Store, StoreLimits, StoreLimitsBuilder};

mod bindings {
    pub mod engine {
        wasmtime::component::bindgen!({ path: "../../wit", world: "engine" });
    }
    pub mod app {
        wasmtime::component::bindgen!({ path: "../../wit", world: "app" });
    }
}

use bindings::app::fos::app::{fs as wit_fs, kv as wit_kv, log as wit_log};
use bindings::engine::fos::app::types as wit;

/// Fuel for one call into an app.
const APP_FUEL: u64 = 2_000_000_000;
/// Memory for one app instance.
const APP_MEMORY: usize = 256 << 20;

/// What kind of component a bundle has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    App,
    Engine,
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
}

/// The WIT version this runtime gives.
pub const WIT_VERSION: &str = "fos:app@0.1.0";

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
        let text = std::str::from_utf8(text).map_err(|_| Error("manifest.toml is not UTF-8".into()))?;
        let manifest: Manifest = toml::from_str(text).map_err(|e| Error(format!("manifest.toml: {e}")))?;
        if manifest.requires != WIT_VERSION {
            return Err(Error(format!(
                "{} needs {}, but this runtime gives {WIT_VERSION}",
                manifest.id, manifest.requires
            )));
        }
        if !files.contains_key(&format!("/{}", manifest.main)) {
            return Err(Error(format!("{} is not in the .app folder", manifest.main)));
        }
        Ok(Self { manifest, files })
    }

    /// Reads a bundle from a directory on disk.
    ///
    /// # Errors
    ///
    /// When the directory cannot be read, or [`Bundle::from_files`] fails.
    pub fn read_dir(dir: &Path) -> Result<Self, Error> {
        fn walk(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Vec<u8>>) -> std::io::Result<()> {
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
        Hash::of_value(&self.files.iter().map(|(p, d)| (p, Hash::of(d))).collect::<Vec<_>>())
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
            return Err(Error(format!("{} is not an {kind:?}", bundle.manifest.id).to_lowercase()));
        }
        Component::new(&self.engine, bundle.main()).map_err(wasm_error)
    }

    /// Starts an engine.
    ///
    /// # Errors
    ///
    /// When the bundle is not an engine, or its component does not start.
    pub fn engine(&self, bundle: &Bundle) -> Result<EngineInstance, Error> {
        let component = self.component(bundle, Kind::Engine)?;
        let mut store = Store::new(&self.engine, ());
        // The engine is the owner's choice: no limits.
        store.set_fuel(u64::MAX).map_err(wasm_error)?;
        let linker = Linker::new(&self.engine);
        let bindings =
            bindings::engine::Engine::instantiate(&mut store, &component, &linker).map_err(wasm_error)?;
        Ok(EngineInstance { id: bundle.manifest.id.clone(), store, bindings })
    }

    /// Starts an app. Its calls go to `host`.
    ///
    /// # Errors
    ///
    /// When the bundle is not an app, or its component does not start.
    pub fn app(&self, bundle: &Bundle, host: Arc<dyn AppHost>) -> Result<AppInstance, Error> {
        let component = self.component(bundle, Kind::App)?;
        let limits = StoreLimitsBuilder::new().memory_size(APP_MEMORY).build();
        let ctx = AppCtx { app: bundle.manifest.id.clone(), host, limits };
        let mut store = Store::new(&self.engine, ctx);
        store.limiter(|ctx| &mut ctx.limits);
        store.set_fuel(APP_FUEL).map_err(wasm_error)?;
        let mut linker = Linker::new(&self.engine);
        bindings::app::App::add_to_linker::<AppCtx, HasSelf<AppCtx>>(&mut linker, |ctx| ctx)
            .map_err(wasm_error)?;
        let bindings = bindings::app::App::instantiate(&mut store, &component, &linker).map_err(wasm_error)?;
        Ok(AppInstance { manifest: bundle.manifest.clone(), store, bindings })
    }
}

fn wasm_error(e: wasmtime::Error) -> Error {
    Error(format!("{e:#}"))
}

/// A running engine.
pub struct EngineInstance {
    id: String,
    store: Store<()>,
    bindings: bindings::engine::Engine,
}

impl std::fmt::Debug for EngineInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EngineInstance({})", self.id)
    }
}

impl EngineInstance {
    fn authz(&mut self) -> (&mut Store<()>, &bindings::engine::exports::fos::app::authz::Guest) {
        (&mut self.store, self.bindings.fos_app_authz())
    }
}

fn keys(list: Vec<Vec<u8>>) -> Vec<PublicKey> {
    list.iter().filter_map(|k| PublicKey::try_from(k.as_slice()).ok()).collect()
}

/// A trap in the engine is a "no", so a broken engine never lets more in.
impl Authz for EngineInstance {
    fn init(&mut self, folder: Hash, creator: PublicKey) -> Vec<u8> {
        let (store, authz) = self.authz();
        authz.call_init(store, &folder.to_string(), &creator.0.to_vec()).unwrap_or_default()
    }

    fn apply(&mut self, state: &[u8], author: PublicKey, policy: &[u8]) -> Result<Vec<u8>, String> {
        let (store, authz) = self.authz();
        authz
            .call_apply(store, state, &author.0.to_vec(), policy)
            .unwrap_or_else(|e| Err(format!("the engine failed: {e:#}")))
    }

    fn check(&mut self, state: &[u8], principal: PublicKey, ability: Ability, folder: Hash, path: &str) -> Decision {
        let op = wit::Op {
            ability: to_wit_ability(ability),
            folder: folder.to_string(),
            path: path.to_string(),
        };
        let (store, authz) = self.authz();
        match authz.call_check(store, state, &principal.0.to_vec(), &op) {
            Ok(wit::Decision::Allow) => Decision::Allow,
            Ok(wit::Decision::Deny(reason)) => Decision::Deny(reason),
            Err(e) => Decision::Deny(format!("the engine failed: {e:#}")),
        }
    }

    fn readers(&mut self, state: &[u8]) -> Vec<PublicKey> {
        let (store, authz) = self.authz();
        authz.call_readers(store, state).map(keys).unwrap_or_default()
    }

    fn storers(&mut self, state: &[u8]) -> Vec<PublicKey> {
        let (store, authz) = self.authz();
        authz.call_storers(store, state).map(keys).unwrap_or_default()
    }

    fn grant(&mut self, to: PublicKey, abilities: &[Ability], path: &str) -> Vec<u8> {
        let abilities: Vec<wit::Ability> = abilities.iter().map(|a| to_wit_ability(*a)).collect();
        let (store, authz) = self.authz();
        authz.call_grant(store, &to.0.to_vec(), &abilities, path).unwrap_or_default()
    }
}

fn to_wit_ability(ability: Ability) -> wit::Ability {
    match ability {
        Ability::Read => wit::Ability::Read,
        Ability::Write => wit::Ability::Write,
        Ability::Append => wit::Ability::Append,
        Ability::Grant => wit::Ability::Grant,
        Ability::Store => wit::Ability::Store,
    }
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
    bindings: bindings::app::App,
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
            w.path == "/" || path == w.path || path.strip_prefix(&w.path).is_some_and(|r| r.starts_with('/'))
        })
    }

    /// Tells the app that a path changed.
    ///
    /// # Errors
    ///
    /// When the app traps or runs out of fuel or memory.
    pub fn on_change(&mut self, path: &str) -> Result<(), Error> {
        self.store.set_fuel(APP_FUEL).map_err(wasm_error)?;
        self.bindings.fos_app_events().call_on_change(&mut self.store, path).map_err(wasm_error)
    }
}

#[cfg(test)]
mod tests;
