// SPDX-License-Identifier: AGPL-3.0-only

//! Installs and starts apps, and handles their calls to the node.

use std::collections::{BTreeMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};

use fos_core::{Error, Hash, PublicKey, SecretKey};
use fos_runtime::{AppHost, Bundle, FsError, Host as RuntimeHost, HttpApp, Kind};
use fos_space::Principal;

use super::{Inner, Node, Shared, lock, newest, parse_permissions};

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

impl Node {
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
        if bundle.manifest().kind != Kind::App {
            return Err(Error::invalid(format!("{} is not an app", dir.display())));
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
            app_key = inner.app_key(&bundle.manifest().id);
            let time = (inner.clock)();
            let (id, step) = inner.node.create(time);
            space = id;
            inner.process(step, &self.shared.changed)?;
            inner.mount(id, &at, &self.shared.changed)?;
            for (path, data) in bundle.files() {
                inner.put(None, &format!("{at}{path}"), data, &self.shared.changed)?;
            }
            for permission in &bundle.manifest().permissions {
                inner.grant(
                    None,
                    &permission.path,
                    Principal::Key(app_key.public()),
                    &parse_permissions(&permission.access)?,
                    &self.shared.changed,
                )?;
            }
        }
        let id = self.start_app(&bundle, space)?;
        // Tell the new app about the files that are there already.
        let mut existing = Vec::new();
        {
            let inner = self.inner();
            for watch in &bundle.manifest().watch {
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

    /// The app with this ID, if it gives HTTP.
    pub fn http_app(&self, id: &str) -> Option<HttpApp> {
        lock(&self.shared.http).get(id).cloned()
    }

    /// The IDs of the apps that give HTTP.
    pub fn http_apps(&self) -> Vec<String> {
        lock(&self.shared.http).keys().cloned().collect()
    }

    /// The cache of an app on this node.
    pub fn kv(&self, app: &str) -> BTreeMap<String, Vec<u8>> {
        self.inner().kv(app).clone()
    }

    pub(super) fn start_apps(&self) {
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
                tracing::warn!(name: "node.app.start.failed", app = bundle.manifest().id, error = %e, "app did not start");
            }
        }
    }

    /// Starts an app that is in the space `space`. Gives its app ID.
    fn start_app(&self, bundle: &Bundle, space: Hash) -> Result<String, Error> {
        let host = RuntimeHost::new(Host {
            shared: Arc::downgrade(&self.shared),
        });
        let id = app_id(&bundle.manifest().id, space);
        let http = self.shared.runtime.http_app(bundle, host.clone())?;
        let app = self.shared.runtime.app(bundle, host)?;
        lock(&self.shared.apps).push(app);
        lock(&self.shared.http).insert(id.clone(), http);
        Ok(id)
    }

    /// Tells the apps about changed paths, until there are no more.
    pub fn pump(&self) {
        pump(&self.shared);
    }
}

fn pump(shared: &Shared) {
    loop {
        let Some(path) = lock(&shared.changed).pop_front() else {
            return;
        };
        let mut apps = lock(&shared.apps);
        for app in apps.iter_mut().filter(|a| a.watches(&path)) {
            if let Err(e) = app.on_change(&path) {
                tracing::warn!(name: "node.app.watch.failed", app = app.manifest().id, error = %e, "app could not handle a change");
            }
        }
    }
}

impl Inner {
    fn kv(&mut self, app: &str) -> &mut BTreeMap<String, Vec<u8>> {
        let store = &self.store;
        self.kv
            .entry(app.to_string())
            .or_insert_with(|| store.kv_load(app))
    }

    /// The node paths of every file under a node path.
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
        let mut inner = lock(&shared.inner);
        Some(f(&mut inner, &shared.changed))
    }

    fn stopped<T>() -> Result<T, FsError> {
        Err(FsError::Unavailable("the node stopped".into()))
    }
}

impl AppHost for Host {
    fn read(&self, app: &str, path: &str) -> Result<Vec<u8>, FsError> {
        self.with(|inner, _| {
            let key = inner.app_key(app).public();
            inner.read(key, path)
        })
        .unwrap_or_else(Host::stopped)
    }

    fn write(&self, app: &str, path: &str, data: Vec<u8>) -> Result<(), FsError> {
        self.with(|inner, changed| {
            let key = inner.app_key(app);
            inner
                .put(Some(&key), path, &data, changed)
                .map_err(FsError::from)
        })
        .unwrap_or_else(Host::stopped)
    }

    fn append(&self, app: &str, path: &str, data: Vec<u8>) -> Result<(), FsError> {
        self.with(|inner, changed| {
            let key = inner.app_key(app);
            inner
                .append(Some(&key), path, &data, changed)
                .map_err(FsError::from)
        })
        .unwrap_or_else(Host::stopped)
    }

    fn delete(&self, app: &str, path: &str) -> Result<(), FsError> {
        self.with(|inner, changed| {
            let key = inner.app_key(app);
            inner
                .delete(Some(&key), path, changed)
                .map_err(FsError::from)
        })
        .unwrap_or_else(Host::stopped)
    }

    fn list(&self, app: &str, path: &str) -> Result<Vec<String>, FsError> {
        self.with(|inner, _| {
            let key = inner.app_key(app).public();
            inner.list(key, path)
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
                tracing::warn!(name: "node.app.cache.failed", app, error = %e, "could not save app cache");
            }
        });
    }

    fn kv_delete(&self, app: &str, key: &str) {
        self.with(|inner, _| {
            if inner.kv(app).remove(key).is_some() {
                let map = inner.kv(app).clone();
                if let Err(e) = inner.store.kv_save(app, &map) {
                    tracing::warn!(name: "node.app.cache.failed", app, error = %e, "could not save app cache");
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
        tracing::info!(name: "node.app.message", app, message);
        self.with(|inner, _| inner.log(format!("[{app}] {message}")));
    }
}
