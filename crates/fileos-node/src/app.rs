// SPDX-License-Identifier: AGPL-3.0-only

//! Installs and starts apps, and handles their calls to the node.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::{Arc, Weak};

use fileos_core::{Error, Hash, PublicKey, SecretKey};
use fileos_runtime::{AppHost, Bundle, ChangeKind, FsError, Host as RuntimeHost, HttpApp, Kind};
use fileos_space::{Act, Effect, Id, Permission, Principal, Resource};

use super::{
    APPDATA, AccessRequest, Changed, Inner, Node, READ, REQUESTS, Shared, VIEWS, ViewMount, clean,
    grant_text, lock, parse_grant, parse_permissions,
};

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
            let app = bundle.manifest().id.clone();
            app_key = inner.app_key(&app);
            inner.node.add_shared_key(app_key.clone());
            let now = (inner.clock)();
            let (id, step) = inner.node.create(now);
            space = id;
            inner.process(step, &self.shared.changed)?;
            inner.mount(id, &at, &self.shared.changed)?;
            for (path, data) in bundle.files() {
                inner.put(None, &format!("{at}{path}"), data, &self.shared.changed)?;
            }
            // Every node of the person runs the app: they may read and keep
            // its files.
            if let Some(person) = inner.identity_principal() {
                inner.grant(None, &at, person, &READ, &self.shared.changed)?;
            }
            // The private data of the app: a space that its key owns, so no
            // other app and no person has access to it unless given.
            if inner.app_data(&app).is_none() {
                let (data, step) = inner.node.create_as(&app_key, now);
                inner.process(step, &self.shared.changed)?;
                inner.mount(data, &format!("{APPDATA}{app}"), &self.shared.changed)?;
                // Every node of the person keeps it; only the app reads it.
                if let Some(person) = inner.identity_principal() {
                    inner.grant(
                        Some(&app_key),
                        &format!("/{APPDATA}{app}"),
                        person,
                        &[Permission::DataKeep],
                        &self.shared.changed,
                    )?;
                }
            }
            // Each permission in the manifest is mounted in the app's view at
            // the name the manifest gives.
            for permission in &bundle.manifest().permissions {
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
            for watch in &bundle.manifest().watch {
                existing.extend(inner.files_under(&watch.path));
            }
        }
        lock(&self.shared.changed).extend(existing.into_iter().map(|p| (p, ChangeKind::Made)));
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
        // The app hears of the new mount, and of the files that are there
        // already.
        let mut existing = vec![format!("/{}", clean(path))];
        existing.extend(inner.files_under(path));
        drop(inner);
        lock(&self.shared.changed).extend(existing.into_iter().map(|p| (p, ChangeKind::Made)));
        self.pump();
        Ok(format!("/{name}"))
    }

    /// What an app sees: the mounts in its view.
    pub fn app_view(&self, app: &str) -> Vec<ViewMount> {
        self.inner().views(app)
    }

    /// The requests of apps for access that wait for the person.
    pub fn requests(&self) -> Vec<AccessRequest> {
        self.inner().requests()
    }

    /// Answers a request of an app: gives it what it asked for on the node
    /// path the person picked, at `/granted/<handle>` in its view. Gives that
    /// path.
    ///
    /// # Errors
    ///
    /// When there is no such request, or the grant cannot be given.
    pub fn answer(&self, id: &str, path: &str) -> Result<String, Error> {
        let request = self
            .requests()
            .into_iter()
            .find(|r| r.id == id)
            .ok_or_else(|| Error::invalid(format!("no request {id:?}")))?;
        if let Some((space, file)) = request.share {
            // A share: `path` names the app to give the file to.
            let at = self.share_to(path, space, file)?;
            self.refuse(id)?;
            return Ok(at);
        }
        let at = self.app_grant(
            &request.app,
            path,
            &parse_permissions(&request.permissions)?,
            None,
        )?;
        self.refuse(id)?;
        Ok(at)
    }

    /// Gives the app `to` read access to one file, by its ID, mounted at
    /// `granted/<handle>/<name>` in its view.
    fn share_to(&self, to: &str, space: Hash, file: Id) -> Result<String, Error> {
        let mut inner = self.inner();
        let s = inner
            .node
            .space(space)
            .ok_or_else(|| Error::unavailable("the space of the file is not here".into()))?;
        let f = s
            .file(file)
            .filter(|f| !f.deleted())
            .ok_or_else(|| Error::not_found("the file is not there any more".into()))?;
        let name = f.versions[0].name.clone();
        let short = name.rsplit('/').next().unwrap_or(&name).to_string();
        let takes = lock(&self.shared.receives).get(to).is_some_and(|list| {
            list.iter().any(|r| {
                r.filter.is_empty() || r.filter.iter().any(|g| fileos_runtime::glob(g, &short))
            })
        });
        if !takes {
            return Err(Error::invalid(format!(
                "the app {to} does not take {short:?}"
            )));
        }
        let key = inner.app_key(to).public();
        let grant = inner.write(
            None,
            space,
            vec![Act::Give {
                to: Principal::Key(key),
                effect: Effect::Allow,
                permissions: vec![Permission::FilesRead],
                on: Resource::File(file),
            }],
            &self.shared.changed,
        )?;
        let handle = &grant.entry.to_string()[..12];
        let mount = format!("granted/{handle}/{short}");
        let home = inner.home;
        let text = format!("{space}\n{name}\n{}", grant_text(grant));
        inner.put_name(
            None,
            home,
            &format!("{VIEWS}{to}/{mount}"),
            text.as_bytes(),
            &self.shared.changed,
        )?;
        let at: Vec<String> = inner.node_paths(space, &name);
        drop(inner);
        lock(&self.shared.changed).extend(at.into_iter().map(|p| (p, ChangeKind::Made)));
        self.pump();
        Ok(format!("/{mount}"))
    }

    /// Says no to a request of an app: it goes away, and the app gets
    /// nothing.
    ///
    /// # Errors
    ///
    /// When there is no such request.
    pub fn refuse(&self, id: &str) -> Result<(), Error> {
        let mut inner = self.inner();
        let home = inner.home;
        inner.delete_name(None, home, &format!("{REQUESTS}{id}"), &self.shared.changed)
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
                tracing::warn!("app {} did not start: {e}", bundle.manifest().id);
            }
        }
    }

    /// Starts the apps in the home space that are not running yet, such as
    /// apps that another node of the person installed, once their files
    /// are here.
    pub(super) fn start_new_apps(&self) {
        let running: BTreeSet<String> = lock(&self.shared.http).keys().cloned().collect();
        let bundles = {
            let Ok(inner) = self.shared.inner.try_lock() else {
                return;
            };
            inner
                .mounts()
                .into_iter()
                .filter(|(at, _)| at.starts_with("apps/"))
                .filter_map(|(_, id)| Some((inner.bundle(id)?, id)))
                .filter(|(b, id)| !running.contains(&app_id(&b.manifest().id, *id)))
                .collect::<Vec<_>>()
        };
        for (bundle, space) in bundles {
            if let Err(e) = self.start_app(&bundle, space) {
                tracing::warn!("app {} did not start: {e}", bundle.manifest().id);
            }
        }
    }

    /// Starts an app that is in the space `space`. Gives its app ID.
    fn start_app(&self, bundle: &Bundle, space: Hash) -> Result<String, Error> {
        let key = self.inner().app_key(&bundle.manifest().id);
        self.inner().node.add_shared_key(key);
        let host = RuntimeHost::new(Host {
            shared: Arc::downgrade(&self.shared),
        });
        let id = app_id(&bundle.manifest().id, space);
        let http = self.shared.runtime.http_app(bundle, host.clone())?;
        let app = self.shared.runtime.app(bundle, host)?;
        // Another thread may have started it in the meantime.
        let mut running = lock(&self.shared.http);
        if running.contains_key(&id) {
            return Ok(id);
        }
        running.insert(id.clone(), http);
        drop(running);
        lock(&self.shared.apps).push(app);
        lock(&self.shared.schedules).insert(id.clone(), bundle.manifest().schedule.clone());
        let mut asks = BTreeSet::new();
        for ask in &bundle.manifest().asks {
            asks.extend(parse_permissions(&ask.permissions)?);
        }
        lock(&self.shared.asks).insert(bundle.manifest().id.clone(), asks);
        lock(&self.shared.receives).insert(
            bundle.manifest().id.clone(),
            bundle.manifest().receives.clone(),
        );
        if self
            .shared
            .scheduling
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            for schedule in &bundle.manifest().schedule {
                self.spawn_schedule(&id, schedule);
            }
        }
        Ok(id)
    }

    /// Runs one schedule of an app now: sends `POST
    /// /.well-known/fileos/schedule/<name>` with `fileos-event: schedule` to the
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
            .ok_or_else(|| Error::invalid(format!("the app {app} has no schedule {name:?}")))?;
        let answer = http
            .request(
                "POST",
                &format!("{}{name}", fileos_runtime::SCHEDULE_PATH),
                &[("fileos-event", "schedule")],
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
        let all: Vec<(String, fileos_runtime::Schedule)> = lock(&self.shared.schedules)
            .iter()
            .flat_map(|(id, list)| list.iter().map(|s| (id.clone(), s.clone())))
            .collect();
        for (id, schedule) in all {
            self.spawn_schedule(&id, &schedule);
        }
    }

    fn spawn_schedule(&self, id: &str, schedule: &fileos_runtime::Schedule) {
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
        self.start_new_apps();
        pump(&self.shared);
    }
}

fn pump(shared: &Shared) {
    loop {
        let Some((path, kind)) = lock(&shared.changed).pop_front() else {
            return;
        };
        // When the apps are busy, this is a write made by an app while it was
        // told of a change: the loop that holds them takes this path next.
        let mut apps = match shared.apps.try_lock() {
            Ok(apps) => apps,
            Err(std::sync::TryLockError::Poisoned(e)) => e.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                lock(&shared.changed).push_front((path, kind));
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
            let paths: Vec<String> = paths.into_iter().filter(|p| app.wants(p, kind)).collect();
            for path in &paths {
                if let Err(e) = app.on_change(path) {
                    tracing::warn!("app {} failed on {path}: {e}", app.manifest().id);
                }
            }
        }
    }
}

impl Inner {
    pub(super) fn kv(&mut self, app: &str) -> &mut BTreeMap<String, Vec<u8>> {
        let store = &self.store;
        self.kv
            .entry(app.to_string())
            .or_insert_with(|| store.kv_load(app))
    }

    /// The files of an app space, as a bundle.
    fn bundle(&self, space: Hash) -> Option<Bundle> {
        let s = self.node.space(space)?;
        let files = s
            .files()
            .iter()
            .map(|f| {
                let name = &f.versions.first()?.name;
                let bytes = self.bytes(space, f)?;
                Some((format!("/{name}"), bytes))
            })
            .collect::<Option<_>>()?;
        Bundle::from_files(files).ok()
    }

    pub(super) fn requests(&self) -> Vec<AccessRequest> {
        let Some(home) = self.node.space(self.home) else {
            return Vec::new();
        };
        home.files()
            .iter()
            .filter_map(|f| {
                let id = f
                    .versions
                    .iter()
                    .find_map(|v| v.name.strip_prefix(REQUESTS))?
                    .to_string();
                let text = String::from_utf8(self.bytes(self.home, f)?).ok()?;
                let mut lines = text.split('\n');
                let app = lines.next()?.to_string();
                let permissions = lines.next()?.to_string();
                let purpose = lines.next().unwrap_or_default().to_string();
                let share = match (lines.next(), lines.next()) {
                    (Some(space), Some(file)) => {
                        Some((space.parse().ok()?, parse_grant(file).ok()?))
                    }
                    _ => None,
                };
                let file = share.and_then(|(space, file)| {
                    let f = self.node.space(space)?.file(file)?;
                    Some(f.versions.first()?.name.clone())
                });
                Some(AccessRequest {
                    id,
                    app,
                    purpose,
                    permissions,
                    file,
                    share,
                })
            })
            .collect()
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
                let bytes = self.bytes(self.home, f)?;
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
    pub(super) fn view_paths(&self, app: &str, space: Hash, name: &str) -> Vec<String> {
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
        changed: &Changed,
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
    fn take_app(&mut self, app: &str, name: &str, changed: &Changed) -> Result<(), Error> {
        let v = self
            .views(app)
            .into_iter()
            .find(|v| v.name == name)
            .ok_or_else(|| Error::invalid(format!("the app {app} has no /{name}")))?;
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
    pub(super) fn app_key(&self, app: &str) -> SecretKey {
        self.apps_root().derive(&format!("app:{app}"))
    }
}

/// What an app may call. Every call is checked against the grants of the
/// app's key.
struct Host {
    shared: Weak<Shared>,
}

impl Host {
    fn with<T>(&self, f: impl FnOnce(&mut Inner, &Changed) -> T) -> Option<T> {
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
                .map_err(FsError::from)
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
                .map_err(FsError::from)
        })
        .unwrap_or_else(Host::stopped)
    }

    fn delete(&self, app: &str, path: &str) -> Result<(), FsError> {
        self.with(|inner, changed| {
            let key = inner.app_key(app);
            let (space, name) = inner.resolve_app(app, path)?;
            inner
                .delete_name(Some(&key), space, &name, changed)
                .map_err(FsError::from)
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

    fn ask(&self, app: &str, purpose: &str, permissions: &str) -> Result<String, String> {
        let shared = self.shared.upgrade().ok_or("the node stopped")?;
        let asked = parse_permissions(permissions).map_err(|e| e.to_string())?;
        let allowed = lock(&shared.asks).get(app).cloned().unwrap_or_default();
        if let Some(p) = asked.iter().find(|p| !allowed.contains(p)) {
            return Err(format!(
                "the manifest of {app} does not let it ask for {p:?}"
            ));
        }
        if purpose.len() > 500 || purpose.contains('\n') {
            return Err("a purpose is one line of at most 500 bytes".into());
        }
        let text = format!("{app}\n{permissions}\n{purpose}");
        self.with(|inner, changed| {
            let id = Hash::of_value(&(app, purpose, permissions, (inner.clock)())).to_string()
                [..12]
                .to_string();
            let home = inner.home;
            inner.put_name(
                None,
                home,
                &format!("{REQUESTS}{id}"),
                text.as_bytes(),
                changed,
            )?;
            Ok(id)
        })
        .ok_or("the node stopped")?
        .map_err(|error: Error| error.to_string())
    }

    fn share(&self, app: &str, path: &str) -> Result<String, String> {
        self.with(|inner, changed| {
            let key = inner.app_key(app).public();
            let (space, name) = inner.resolve_app(app, path).map_err(|e| format!("{e:?}"))?;
            let s = inner.node.space(space).ok_or("the file is not here")?;
            let file = s.find(&name).into_iter().next().ok_or("no such file")?;
            if !s.may(
                Principal::Key(key),
                Permission::ShareSend,
                &Resource::File(file.id),
            ) {
                return Err(format!("{app} may not share {path} (it needs share.send)"));
            }
            let id = Hash::of_value(&(app, space, file.id, (inner.clock)())).to_string()[..12]
                .to_string();
            let short = name.rsplit('/').next().unwrap_or(&name);
            let text = format!(
                "{app}\nshare\nShare {short}\n{space}\n{}",
                grant_text(file.id)
            );
            let home = inner.home;
            inner
                .put_name(
                    None,
                    home,
                    &format!("{REQUESTS}{id}"),
                    text.as_bytes(),
                    changed,
                )
                .map_err(|e| e.to_string())?;
            Ok(id)
        })
        .ok_or("the node stopped")?
    }
}
