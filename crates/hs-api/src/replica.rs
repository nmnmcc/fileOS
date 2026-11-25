// SPDX-License-Identifier: AGPL-3.0-only

//! Replicas: an encrypted copy of the data directory somewhere else, and
//! getting it back.
//!
//! A replica holds every file of the data directory except the index, the
//! access log and the storage key, each sealed with the storage key under an
//! opaque name. A sealed manifest lists the files and their hashes; it is
//! published only after all of its objects are available. Each changed file
//! gets a fresh object name. Unchanged files keep their names across snapshots.
//! The current format reads version-one backups. Restoring updated backups
//! requires a version-two reader. Older homeserver stores can hold these
//! objects, but keep retired versions because they cannot compare manifests.
//!
//! The storage key is derived from the recovery code, so the recovery code
//! alone is enough to restore.
//!
//! Where a replica can live:
//!
//! - `/some/dir` or `dir:/some/dir`: a directory (a USB disk, a NAS mount).
//! - `s3://KEY:SECRET@host[:port]/bucket[/prefix]`: any S3-compatible
//!   store. Query options: `region=` (default `us-east-1`), `http=1`.
//! - `hs://KEY:SECRET@host[:port]/bucket[/prefix]`: another homeserver's
//!   S3 subset (see `s3.rs`). Query option: `http=1`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path};
use std::sync::Arc;
use std::time::Duration;

use hs_core::Hash;
use hs_net::client::Client;
use hs_objects::Dir;
use hs_objects::crypt::StorageKey;
use hs_objects::s3::{self, Credentials};
use http::{StatusCode, header};
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

use crate::Api;
use crate::types::ReplicaSync;

const MANIFEST: &str = "manifest";
const SYNC_EVERY: Duration = Duration::from_secs(3600);

/// A replica store.
#[derive(Debug)]
pub enum Store {
    /// A local directory.
    Dir(Dir),
    /// An S3 bucket, path-style.
    S3(S3),
}

/// An S3 bucket.
pub struct S3 {
    scheme: &'static str,
    host: String,
    base: String,
    bucket: String,
    prefix: String,
    creds: Credentials,
    client: Client,
    homeserver: bool,
}

impl std::fmt::Debug for S3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3").finish_non_exhaustive()
    }
}

fn now_s() -> u64 {
    hs_node::now_ms() / 1000
}

impl S3 {
    fn path(&self, key: &str) -> String {
        let mut p = format!("{}/{}", self.base, s3::uri_encode(&self.bucket, false));
        if !key.is_empty() {
            p.push('/');
            p.push_str(&s3::uri_encode(&format!("{}{key}", self.prefix), true));
        }
        p
    }

    async fn send(
        &self,
        method: &str,
        key: &str,
        body: Vec<u8>,
        headers: &[(String, String)],
    ) -> Result<http::Response<Vec<u8>>, String> {
        let path = self.path(key);
        let payload_hash = s3::sha256_hex(&body);
        let req = s3::Request {
            method,
            host: &self.host,
            path: &path,
            query: &[],
            payload_hash: &payload_hash,
            headers,
        };
        let signed = s3::sign(&self.creds, &req, now_s());
        let url = format!("{}://{}{path}", self.scheme, self.host);
        let mut b = http::Request::builder()
            .method(method)
            .uri(&url)
            .header(http::header::HOST, &self.host);
        for (k, v) in signed {
            b = b.header(k, v);
        }
        for (k, v) in headers {
            b = b.header(k, v);
        }
        let req = b.body(body).map_err(|e| e.to_string())?;
        self.client.send(req).await.map_err(err)
    }
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl Store {
    /// Parse a replica address (see the module docs).
    ///
    /// # Errors
    ///
    /// Returns an error for invalid S3 settings, credentials or a directory that cannot be opened.
    pub fn parse(spec: &str) -> Result<Store, String> {
        if let Some(p) = spec.strip_prefix("dir:") {
            return Dir::open(p).map(Store::Dir).map_err(err);
        }
        if spec.starts_with('/') || spec.starts_with('.') {
            return Dir::open(spec).map(Store::Dir).map_err(err);
        }
        let (kind, rest) = spec
            .split_once("://")
            .ok_or("expected a directory, s3:// or hs://")?;
        if kind != "s3" && kind != "hs" {
            return Err(format!("unknown replica kind {kind}"));
        }
        let (rest, query) = rest.split_once('?').unwrap_or((rest, ""));
        let q = crate::util::parse_form(query);
        let (userinfo, rest) = rest
            .rsplit_once('@')
            .ok_or("credentials missing: KEY:SECRET@host")?;
        let (key, secret) = userinfo
            .split_once(':')
            .ok_or("credentials must be KEY:SECRET")?;
        let (host, path) = rest.split_once('/').ok_or("bucket missing")?;
        let (bucket, prefix) = path.split_once('/').unwrap_or((path, ""));
        if bucket.is_empty() {
            return Err("bucket missing".into());
        }
        let prefix = match prefix.trim_matches('/') {
            "" => String::new(),
            p => format!("{p}/"),
        };
        let hs = kind == "hs";
        let region =
            crate::util::param(&q, "region").unwrap_or(if hs { "homeserver" } else { "us-east-1" });
        Ok(Store::S3(S3 {
            scheme: if crate::util::param(&q, "http") == Some("1") {
                "http"
            } else {
                "https"
            },
            host: host.to_string(),
            base: if hs { "/s3".into() } else { String::new() },
            bucket: bucket.to_string(),
            prefix,
            creds: Credentials {
                access_key: crate::util::percent_decode(key, false),
                secret_key: crate::util::percent_decode(secret, false),
                region: region.to_string(),
            },
            client: if hs {
                Client::accepting_self_signed()
            } else {
                Client::new()
            },
            homeserver: hs,
        }))
    }

    async fn get(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        match self {
            Store::Dir(d) => {
                let dir = d.clone();
                let name = name.to_owned();
                tokio::task::spawn_blocking(move || dir.get(&name))
                    .await
                    .map_err(err)?
                    .map_err(err)
            }
            Store::S3(s) => {
                let r = s.send("GET", name, Vec::new(), &[]).await?;
                match r.status().as_u16() {
                    200 => Ok(Some(r.into_body())),
                    404 => Ok(None),
                    c => Err(format!(
                        "GET {name}: {c} {}",
                        String::from_utf8_lossy(r.body())
                    )),
                }
            }
        }
    }

    async fn put(&self, name: &str, bytes: Vec<u8>) -> Result<(), String> {
        match self {
            Store::Dir(d) => {
                let dir = d.clone();
                let name = name.to_owned();
                tokio::task::spawn_blocking(move || dir.put_durable(&name, &bytes))
                    .await
                    .map_err(err)?
                    .map_err(err)
            }
            Store::S3(s) => {
                let r = s.send("PUT", name, bytes, &[]).await?;
                if r.status().is_success() {
                    Ok(())
                } else {
                    Err(format!(
                        "PUT {name}: {} {}",
                        r.status(),
                        String::from_utf8_lossy(r.body())
                    ))
                }
            }
        }
    }

    async fn delete(&self, name: &str) -> Result<(), String> {
        match self {
            Store::Dir(d) => {
                let dir = d.clone();
                let name = name.to_owned();
                tokio::task::spawn_blocking(move || dir.delete(&name))
                    .await
                    .map_err(err)?
                    .map_err(err)
            }
            Store::S3(s) => {
                let r = s.send("DELETE", name, Vec::new(), &[]).await?;
                if r.status().is_success() || r.status() == http::StatusCode::NOT_FOUND {
                    Ok(())
                } else {
                    Err(format!("DELETE {name}: {}", r.status()))
                }
            }
        }
    }

    async fn snapshot(&self, name: &str) -> Result<(Option<Vec<u8>>, Revision), String> {
        match self {
            Store::Dir(_) => {
                let bytes = self.get(name).await?;
                let revision = Revision::Directory(bytes.as_deref().map(Hash::of));
                Ok((bytes, revision))
            }
            Store::S3(s) => {
                let response = s.send("GET", name, Vec::new(), &[]).await?;
                let conditional =
                    !s.homeserver || response.headers().contains_key("hs-conditional-put");
                match response.status() {
                    StatusCode::OK => {
                        let revision = if conditional {
                            let tag = response
                                .headers()
                                .get(header::ETAG)
                                .and_then(|tag| tag.to_str().ok())
                                .ok_or("replica store omitted its manifest ETag")?;
                            Revision::Remote(Some(tag.to_owned()))
                        } else {
                            Revision::Legacy
                        };
                        Ok((Some(response.into_body()), revision))
                    }
                    StatusCode::NOT_FOUND => Ok((
                        None,
                        if conditional {
                            Revision::Remote(None)
                        } else {
                            Revision::Legacy
                        },
                    )),
                    status => Err(format!("read replica manifest: {status}")),
                }
            }
        }
    }

    async fn publish(&self, name: &str, bytes: Vec<u8>, expected: &Revision) -> Result<(), String> {
        let written = match (self, expected) {
            (Store::Dir(dir), Revision::Directory(expected)) => {
                let dir = dir.clone();
                let name = name.to_owned();
                let expected = *expected;
                tokio::task::spawn_blocking(move || dir.put_if(&name, &bytes, expected))
                    .await
                    .map_err(err)?
                    .map_err(err)?
            }
            (Store::S3(s), Revision::Remote(tag)) => {
                let condition = match tag {
                    Some(tag) => ("if-match".into(), tag.clone()),
                    None => ("if-none-match".into(), "*".into()),
                };
                let response = s.send("PUT", name, bytes, &[condition]).await?;
                match response.status() {
                    status if status.is_success() => true,
                    StatusCode::PRECONDITION_FAILED | StatusCode::CONFLICT => false,
                    status => return Err(format!("publish replica manifest: {status}")),
                }
            }
            // Older homeserver stores accept unconditional PUTs. Immutable objects
            // keep their snapshots restorable even when those writers overlap.
            (_, Revision::Legacy) => return self.put(name, bytes).await,
            _ => unreachable!("the replica revision comes from this store"),
        };
        if written {
            Ok(())
        } else {
            Err("another sync published a replica snapshot first; retry the sync".into())
        }
    }
}

#[derive(PartialEq, Eq)]
enum Revision {
    Directory(Option<Hash>),
    Remote(Option<String>),
    Legacy,
}

#[derive(Deserialize, Serialize)]
struct Manifest {
    version: u8,
    time: u64,
    files: BTreeMap<String, ReplicaFile>,
}

#[derive(Deserialize, Serialize)]
struct ReplicaFile {
    hash: String,
    size: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    generation: Option<String>,
}

impl ReplicaFile {
    fn object(&self, rel: &str) -> String {
        self.generation.as_ref().map_or_else(
            || rel.to_owned(),
            |generation| format!("snapshots/{generation}/{rel}"),
        )
    }
}

/// Whether a data-directory file belongs in a replica.
fn replicated(rel: &str) -> bool {
    !(rel.starts_with("index.db")
        || rel == "keys/storage"
        || rel == "access.log"
        || rel == "buckets"
        || rel.starts_with("buckets/")
        || rel
            .split('/')
            .any(|name| name.starts_with(".tmp-") || name.starts_with(".space-"))
        || rel.ends_with(".tmp")
        || rel.ends_with(".lock"))
}

#[derive(PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: std::time::SystemTime,
    #[cfg(unix)]
    inode: u64,
}

impl FileStamp {
    fn read(metadata: &std::fs::Metadata) -> std::io::Result<Self> {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt as _;

        Ok(Self {
            len: metadata.len(),
            modified: metadata.modified()?,
            #[cfg(unix)]
            inode: metadata.ino(),
        })
    }
}

async fn walk(dir: &Path) -> Result<BTreeMap<String, FileStamp>, String> {
    let mut out = BTreeMap::new();
    let mut stack = vec![String::new()];
    while let Some(relative) = stack.pop() {
        let path = dir.join(&relative);
        let mut entries = tokio::fs::read_dir(&path)
            .await
            .map_err(|error| format!("list {}: {error}", path.display()))?;
        while let Some(entry) = entries.next_entry().await.map_err(err)? {
            let name = entry.file_name();
            let name = name.to_str().ok_or("replica path is not UTF-8")?;
            if name.contains('\\') {
                return Err(format!("unsupported replica file name: {name}"));
            }
            let rel = if relative.is_empty() {
                name.to_owned()
            } else {
                format!("{relative}/{name}")
            };
            if !replicated(&rel) {
                continue;
            }
            let kind = entry.file_type().await.map_err(err)?;
            if kind.is_dir() {
                stack.push(rel);
            } else if kind.is_file() {
                let stamp = FileStamp::read(&entry.metadata().await.map_err(err)?).map_err(err)?;
                out.insert(rel, stamp);
            }
        }
    }
    Ok(out)
}

fn seal(key: &StorageKey, name: &str, bytes: &[u8]) -> Vec<u8> {
    key.seal(name, bytes, hs_node::random::<12>())
}

async fn read_manifest(
    store: &Store,
    key: &StorageKey,
) -> Result<(Option<Manifest>, Revision), String> {
    let (sealed, revision) = store.snapshot(&key.opaque_name(MANIFEST)).await?;
    let manifest = sealed
        .map(|sealed| {
            let plain = key
                .open(MANIFEST, &sealed)
                .ok_or("the manifest does not open with this key")?;
            let manifest: Manifest = serde_json::from_slice(&plain).map_err(err)?;
            if !matches!(manifest.version, 1 | 2) {
                return Err(format!(
                    "unsupported replica manifest version {}",
                    manifest.version
                ));
            }
            Ok(manifest)
        })
        .transpose()?;
    Ok((manifest, revision))
}

/// What a sync did.
#[derive(Debug, Default)]
pub struct Report {
    /// Files sent.
    pub sent: usize,
    /// Bytes sent (before encryption).
    pub bytes: u64,
    /// Files already there.
    pub unchanged: usize,
    /// Old objects deleted.
    pub deleted: usize,
}

/// Bring a replica up to date with a data directory.
///
/// # Errors
///
/// Returns an error if the source changes while syncing, any object cannot
/// be written or read, or the manifest has changed since the sync began.
pub async fn sync(dir: &Path, key: &StorageKey, store: &Store) -> Result<Report, String> {
    let generation = data_encoding::HEXLOWER.encode(&hs_node::random::<16>());
    sync_generation(dir, key, store, &generation).await
}

async fn sync_generation(
    dir: &Path,
    key: &StorageKey,
    store: &Store,
    generation: &str,
) -> Result<Report, String> {
    let (old, revision) = read_manifest(store, key).await?;
    let mut old_files = old.map_or_else(BTreeMap::new, |manifest| manifest.files);
    let mut files = BTreeMap::new();
    let mut retired = BTreeSet::new();
    let mut report = Report::default();
    let source = walk(dir).await?;
    for (rel, stamp) in &source {
        let bytes = tokio::fs::read(dir.join(rel))
            .await
            .map_err(|error| format!("read {rel}: {error}"))?;
        if bytes.len() as u64 != stamp.len {
            return Err(source_changed());
        }
        let hash = Hash::of(&bytes).to_hex();
        let previous = old_files.remove(rel);
        if let Some(previous) = previous.as_ref().filter(|previous| previous.hash == hash) {
            files.insert(
                rel.clone(),
                ReplicaFile {
                    hash,
                    size: previous.size,
                    generation: previous.generation.clone(),
                },
            );
            report.unchanged += 1;
            continue;
        }
        if let Some(previous) = previous {
            retired.insert(previous.object(rel));
        }
        let size = bytes.len() as u64;
        let file = ReplicaFile {
            hash,
            size,
            generation: Some(generation.to_owned()),
        };
        let object = file.object(rel);
        store
            .put(&key.opaque_name(&object), seal(key, &object, &bytes))
            .await?;
        report.sent += 1;
        report.bytes += size;
        files.insert(rel.clone(), file);
    }
    // A live node can rotate logs or replace config while a sync is uploading.
    // Check the same source tree before publishing any references to the new objects.
    if walk(dir).await? != source {
        return Err(source_changed());
    }
    let manifest = Manifest {
        version: 2,
        time: hs_node::now_ms(),
        files,
    };
    let plain = serde_json::to_vec(&manifest).map_err(err)?;
    store
        .publish(
            &key.opaque_name(MANIFEST),
            seal(key, MANIFEST, &plain),
            &revision,
        )
        .await?;

    // A conditional publication retires only names from its actual predecessor.
    // Unconditional stores cannot safely retire objects while another sync may use them.
    if revision != Revision::Legacy {
        retired.extend(old_files.into_iter().map(|(rel, file)| file.object(&rel)));
        for object in retired {
            let name = key.opaque_name(&object);
            match store.delete(&name).await {
                Ok(()) => report.deleted += 1,
                Err(error) => tracing::warn!(name: "replica.cleanup.failed", object = name, %error,
                    "Could not remove retired replica object {{object}}"),
            }
        }
    }
    Ok(report)
}

fn source_changed() -> String {
    "source files changed during replica sync; retry the sync".into()
}

async fn restore_files(
    store: &Store,
    key: &StorageKey,
    files: &BTreeMap<String, ReplicaFile>,
    dir: &Path,
) -> Result<usize, String> {
    for (rel, file) in files {
        if rel.contains('\\')
            || rel.split('/').any(|part| part.is_empty() || part == ".")
            || !Path::new(rel)
                .components()
                .all(|component| matches!(component, Component::Normal(_)))
        {
            return Err(format!("bad path in manifest: {rel}"));
        }
        let object = file.object(rel);
        let sealed = store
            .get(&key.opaque_name(&object))
            .await?
            .ok_or_else(|| format!("{rel} is missing from the replica"))?;
        let bytes = key
            .open(&object, &sealed)
            .ok_or_else(|| format!("{rel} does not open"))?;
        if bytes.len() as u64 != file.size || Hash::of(&bytes).to_hex() != file.hash {
            return Err(format!("{rel} does not match the manifest"));
        }
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(err)?;
        }
        tokio::fs::write(&path, &bytes).await.map_err(err)?;
    }
    Ok(files.len())
}

async fn stage_restore(
    store: &Store,
    key: &StorageKey,
    manifest: Option<Manifest>,
    parent: &Path,
) -> Result<(tempfile::TempDir, usize), String> {
    let manifest = manifest.ok_or("no replica here (no manifest)")?;
    let stage = tempfile::Builder::new()
        .prefix(".restore-")
        .tempdir_in(parent)
        .map_err(err)?;
    let count = restore_files(store, key, &manifest.files, stage.path()).await?;
    Ok((stage, count))
}

/// Restore a data directory from a replica. `dir` must be empty or absent.
/// Returns how many files were written.
///
/// # Errors
///
/// Returns an error if the destination is occupied, the manifest cannot be opened, or a file cannot
/// be restored.
pub async fn restore(store: &Store, storage_key: [u8; 32], dir: &Path) -> Result<usize, String> {
    match tokio::fs::read_dir(dir).await {
        Ok(mut entries) => {
            if entries.next_entry().await.map_err(err)?.is_some() {
                return Err(format!("{} is not empty", dir.display()));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(err(error)),
    }
    let parent = dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    tokio::fs::create_dir_all(parent).await.map_err(err)?;
    let key = StorageKey::new(storage_key);

    // A concurrent sync may retire objects while this reader is staging an old manifest.
    // Retry once from a fresh manifest without mixing the two snapshots.
    let (manifest, revision) = read_manifest(store, &key).await?;
    let (stage, count) = match stage_restore(store, &key, manifest, parent).await {
        Ok(staged) => staged,
        Err(error) => {
            let (manifest, current) = read_manifest(store, &key).await?;
            if current == revision {
                return Err(error);
            }
            stage_restore(store, &key, manifest, parent).await?
        }
    };
    let keys = stage.path().join("keys");
    tokio::fs::create_dir_all(&keys).await.map_err(err)?;
    tokio::fs::write(
        keys.join("storage"),
        data_encoding::HEXLOWER.encode(&storage_key),
    )
    .await
    .map_err(err)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mut entries = tokio::fs::read_dir(&keys).await.map_err(err)?;
        while let Some(entry) = entries.next_entry().await.map_err(err)? {
            tokio::fs::set_permissions(entry.path(), std::fs::Permissions::from_mode(0o600))
                .await
                .map_err(err)?;
        }
    }
    #[cfg(windows)]
    if dir.exists() {
        tokio::fs::remove_dir(dir).await.map_err(err)?;
    }
    tokio::fs::rename(stage.path(), dir).await.map_err(err)?;
    Ok(count)
}

/// The configured replicas (`replicas` in the config: a list of addresses).
pub fn configured(api: &Api) -> Vec<String> {
    api.node
        .config()
        .raw
        .get("replicas")
        .and_then(Json::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Sync every configured replica now. Returns one line per replica.
pub async fn sync_all(api: &Api) -> Vec<ReplicaSync> {
    let mut out = Vec::new();
    for spec in configured(api) {
        let shown = redact(&spec);
        let r = match Store::parse(&spec) {
            Ok(store) => sync(api.node.dir(), &api.node.storage_key(), &store).await,
            Err(e) => Err(e),
        };
        let count = |n: usize| Some(u64::try_from(n).unwrap_or(u64::MAX));
        out.push(match r {
            Ok(r) => ReplicaSync {
                replica: shown,
                ok: true,
                sent: count(r.sent),
                bytes: Some(r.bytes),
                unchanged: count(r.unchanged),
                deleted: count(r.deleted),
                error: None,
            },
            Err(e) => ReplicaSync {
                replica: shown,
                ok: false,
                sent: None,
                bytes: None,
                unchanged: None,
                deleted: None,
                error: Some(e),
            },
        });
    }
    out
}

/// A replica address with its secret hidden.
pub fn redact(spec: &str) -> String {
    match (spec.find("://"), spec.rfind('@')) {
        (Some(a), Some(b)) if b > a => {
            let user = &spec[a + 3..b];
            let key = user.split(':').next().unwrap_or("");
            format!("{}{key}:***{}", &spec[..a + 3], &spec[b..])
        }
        _ => spec.to_string(),
    }
}

/// Sync configured replicas every hour.
pub fn start(api: Arc<Api>) {
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(SYNC_EVERY).await;
            if !configured(&api).is_empty() {
                for line in sync_all(&api).await {
                    if line.ok {
                        tracing::info!(name: "replica.sync.completed", sent = line.sent,
                            bytes = line.bytes, deleted = line.deleted,
                            "Replica synchronized with {{sent}} files sent");
                    } else {
                        tracing::warn!(name: "replica.sync.failed", "Could not synchronize a replica");
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("hs-replica-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        p
    }

    #[test]
    fn addresses() {
        assert!(matches!(Store::parse("/tmp/x").unwrap(), Store::Dir(_)));
        let Store::S3(s) =
            Store::parse("hs://ak:s%2Fk@node.example:8443/backups/alice?http=1").unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (
                s.scheme,
                s.host.as_str(),
                s.bucket.as_str(),
                s.prefix.as_str()
            ),
            ("http", "node.example:8443", "backups", "alice/")
        );
        assert_eq!(s.creds.secret_key, "s/k");
        assert_eq!(s.path("ab/cd"), "/s3/backups/alice/ab/cd");
        Store::parse("ftp://x").unwrap_err();
        assert_eq!(redact("s3://ak:secret@h/b"), "s3://ak:***@h/b");
    }

    #[tokio::test]
    async fn sync_and_restore() {
        let src = tmp("src");
        std::fs::create_dir_all(src.join("logs/aa")).unwrap();
        std::fs::create_dir_all(src.join("keys")).unwrap();
        std::fs::write(src.join("config.json"), b"{}").unwrap();
        std::fs::write(src.join("logs/aa/head"), b"one").unwrap();
        std::fs::write(src.join("keys/node"), b"seed").unwrap();
        std::fs::write(src.join("keys/storage"), b"never").unwrap();
        std::fs::write(src.join("index.db"), b"derived").unwrap();
        let seed = [9u8; 32];
        let key = StorageKey::new(seed);
        let rep = tmp("rep");
        let store = Store::parse(rep.to_str().unwrap()).unwrap();
        let r = sync(&src, &key, &store).await.unwrap();
        assert_eq!((r.sent, r.unchanged), (3, 0));
        // Nothing readable on the replica.
        let Store::Dir(objects) = &store else {
            unreachable!("test uses a directory")
        };
        for name in objects.list("").unwrap() {
            let b = store.get(&name).await.unwrap().unwrap();
            assert!(!b.windows(3).any(|w| w == b"one" || w == b"see"));
        }
        std::fs::write(src.join("logs/aa/head"), b"two").unwrap();
        std::fs::remove_file(src.join("config.json")).unwrap();
        let r = sync(&src, &key, &store).await.unwrap();
        assert_eq!((r.sent, r.unchanged, r.deleted), (1, 1, 2));
        let dst = tmp("dst");
        restore(&store, [8u8; 32], &dst).await.unwrap_err();
        let _ = std::fs::remove_dir_all(&dst);
        assert_eq!(restore(&store, seed, &dst).await.unwrap(), 2);
        assert_eq!(std::fs::read(dst.join("logs/aa/head")).unwrap(), b"two");
        assert_eq!(std::fs::read(dst.join("keys/node")).unwrap(), b"seed");
        assert!(!dst.join("index.db").exists());
        for p in [src, rep, dst] {
            let _ = std::fs::remove_dir_all(p);
        }
    }

    #[tokio::test]
    async fn interrupted_upload_leaves_previous_snapshot_restorable() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let replica = root.path().join("replica");
        std::fs::create_dir(&source).unwrap();
        let seed = [42; 32];
        let key = StorageKey::new(seed);
        let store = Store::parse(replica.to_str().unwrap()).unwrap();
        std::fs::write(source.join("a"), b"v1").unwrap();
        sync_generation(&source, &key, &store, "first")
            .await
            .unwrap();

        std::fs::write(source.join("a"), b"v2").unwrap();
        std::fs::write(source.join("z"), b"new").unwrap();
        std::fs::create_dir_all(replica.join(key.opaque_name("snapshots/second/z"))).unwrap();
        let error = sync_generation(&source, &key, &store, "second")
            .await
            .unwrap_err();
        assert_ne!(error, "");
        assert!(replica.join(key.opaque_name("snapshots/second/a")).exists());
        let destination = root.path().join("restored");
        assert_eq!(restore(&store, seed, &destination).await.unwrap(), 1);
        assert_eq!(std::fs::read(destination.join("a")).unwrap(), b"v1");
        assert!(!destination.join("z").exists());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unreadable_file_does_not_retire_the_backup_object() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let file = source.join("config.json");
        std::fs::write(&file, b"{}").unwrap();
        let seed = [33; 32];
        let key = StorageKey::new(seed);
        let store = Store::parse(root.path().join("backup").to_str().unwrap()).unwrap();
        sync(&source, &key, &store).await.unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&file).is_ok() {
            // A privileged test runner can read mode-000 files.
            return;
        }
        let error = sync(&source, &key, &store).await.unwrap_err();
        assert!(error.contains("read config.json"), "{error}");
        let destination = root.path().join("restored");
        assert_eq!(restore(&store, seed, &destination).await.unwrap(), 1);
        assert_eq!(
            std::fs::read(destination.join("config.json")).unwrap(),
            b"{}"
        );
    }

    #[tokio::test]
    async fn restores_and_updates_a_legacy_manifest() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("a"), b"old format").unwrap();
        let seed = [3; 32];
        let key = StorageKey::new(seed);
        let store = Store::parse(root.path().join("backup").to_str().unwrap()).unwrap();
        let legacy = serde_json::json!({
            "version": 1, "time": 1,
            "files": { "a": { "hash": Hash::of(b"old format").to_hex(), "size": 10 } }
        });
        store
            .put(&key.opaque_name("a"), seal(&key, "a", b"old format"))
            .await
            .unwrap();
        store
            .put(
                &key.opaque_name(MANIFEST),
                seal(&key, MANIFEST, &serde_json::to_vec(&legacy).unwrap()),
            )
            .await
            .unwrap();
        let old = root.path().join("old");
        assert_eq!(restore(&store, seed, &old).await.unwrap(), 1);
        assert_eq!(std::fs::read(old.join("a")).unwrap(), b"old format");
        let report = sync(&source, &key, &store).await.unwrap();
        assert_eq!((report.sent, report.unchanged), (0, 1));
        std::fs::write(source.join("a"), b"new format").unwrap();
        sync(&source, &key, &store).await.unwrap();
        let new = root.path().join("new");
        assert_eq!(restore(&store, seed, &new).await.unwrap(), 1);
        assert_eq!(std::fs::read(new.join("a")).unwrap(), b"new format");
    }

    #[tokio::test]
    async fn only_one_publication_can_replace_the_same_directory_revision() {
        let root = tempfile::tempdir().unwrap();
        let first = Store::parse(root.path().to_str().unwrap()).unwrap();
        let second = Store::parse(root.path().to_str().unwrap()).unwrap();
        let revision = Revision::Directory(None);
        let (a, b) = tokio::join!(
            first.publish("manifest", b"first".to_vec(), &revision),
            second.publish("manifest", b"second".to_vec(), &revision),
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let expected = if a.is_ok() {
            b"first".as_slice()
        } else {
            b"second".as_slice()
        };
        assert_eq!(first.get("manifest").await.unwrap().unwrap(), expected);
    }
}
