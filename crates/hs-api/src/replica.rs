// SPDX-License-Identifier: AGPL-3.0-only

//! Replicas: an encrypted copy of the data directory somewhere else, and
//! getting it back.
//!
//! A replica holds every file of the data directory except the index, the
//! access log and the storage key, each sealed with the storage key under an
//! opaque name. A sealed manifest lists the files and their hashes; it is
//! written last, so a replica is always either the old copy or the new one.
//! Files whose hash has not changed are not sent again, and objects the new
//! manifest no longer names are deleted.
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
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use hs_core::Hash;
use hs_net::client::Client;
use hs_objects::Dir;
use hs_objects::crypt::StorageKey;
use hs_objects::s3::{self, Credentials};
use serde_json::{Value as Json, json};

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
        query: &[(String, String)],
        body: Vec<u8>,
    ) -> Result<http::Response<Vec<u8>>, String> {
        let path = self.path(key);
        let payload_hash = s3::sha256_hex(&body);
        let req = s3::Request {
            method,
            host: &self.host,
            path: &path,
            query,
            payload_hash: &payload_hash,
            headers: &[],
        };
        let signed = s3::sign(&self.creds, &req, now_s());
        let mut q: Vec<String> = query
            .iter()
            .map(|(k, v)| format!("{}={}", s3::uri_encode(k, false), s3::uri_encode(v, false)))
            .collect();
        q.sort();
        let url = if q.is_empty() {
            format!("{}://{}{path}", self.scheme, self.host)
        } else {
            format!("{}://{}{path}?{}", self.scheme, self.host, q.join("&"))
        };
        let mut b = http::Request::builder()
            .method(method)
            .uri(&url)
            .header(http::header::HOST, &self.host);
        for (k, v) in signed {
            b = b.header(k, v);
        }
        let req = b.body(body).map_err(|e| e.to_string())?;
        self.client.send(req).await
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
        }))
    }

    async fn get(&self, name: &str) -> Result<Option<Vec<u8>>, String> {
        match self {
            Store::Dir(d) => d.get(name).map_err(err),
            Store::S3(s) => {
                let r = s.send("GET", name, &[], Vec::new()).await?;
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
            Store::Dir(d) => d.put(name, &bytes).map_err(err),
            Store::S3(s) => {
                let r = s.send("PUT", name, &[], bytes).await?;
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
            Store::Dir(d) => d.delete(name).map_err(err),
            Store::S3(s) => {
                let r = s.send("DELETE", name, &[], Vec::new()).await?;
                if r.status().is_success() || r.status() == http::StatusCode::NOT_FOUND {
                    Ok(())
                } else {
                    Err(format!("DELETE {name}: {}", r.status()))
                }
            }
        }
    }

    async fn list(&self) -> Result<Vec<String>, String> {
        match self {
            Store::Dir(d) => d.list("").map_err(err),
            Store::S3(s) => {
                let mut out = Vec::new();
                let mut token: Option<String> = None;
                loop {
                    let mut q = vec![
                        ("list-type".to_string(), "2".to_string()),
                        ("prefix".into(), s.prefix.clone()),
                    ];
                    if let Some(t) = &token {
                        q.push(("continuation-token".into(), t.clone()));
                    }
                    let r = s.send("GET", "", &q, Vec::new()).await?;
                    if !r.status().is_success() {
                        return Err(format!("list: {}", r.status()));
                    }
                    let (objects, next) = s3::parse_list(&String::from_utf8_lossy(r.body()));
                    out.extend(
                        objects
                            .into_iter()
                            .filter_map(|o| o.key.strip_prefix(&s.prefix).map(String::from)),
                    );
                    match next {
                        Some(n) => token = Some(n),
                        None => return Ok(out),
                    }
                }
            }
        }
    }
}

/// Whether a data-directory file belongs in a replica.
fn replicated(rel: &str) -> bool {
    !(rel.starts_with("index.db")
        || rel == "keys/storage"
        || rel == "access.log"
        || rel.starts_with("buckets/")
        || rel
            .split('/')
            .any(|name| name.starts_with(".tmp-") || name.starts_with(".space-"))
        || rel.ends_with(".tmp")
        || rel.ends_with(".lock"))
}

fn walk(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(p),
                Ok(t) if t.is_file() => {
                    if let Ok(rel) = p.strip_prefix(dir) {
                        let rel = rel.to_string_lossy().replace('\\', "/");
                        if replicated(&rel) {
                            out.push(rel);
                        }
                    }
                }
                _ => {}
            }
        }
    }
    out.sort();
    out
}

fn seal(key: &StorageKey, name: &str, bytes: &[u8]) -> Vec<u8> {
    key.seal(name, bytes, hs_node::random::<12>())
}

async fn read_manifest(store: &Store, key: &StorageKey) -> Result<Option<Json>, String> {
    let Some(sealed) = store.get(&key.opaque_name(MANIFEST)).await? else {
        return Ok(None);
    };
    let plain = key
        .open(MANIFEST, &sealed)
        .ok_or("the manifest does not open with this key")?;
    serde_json::from_slice(&plain).map(Some).map_err(err)
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
/// Returns an error if the manifest or an object cannot be written, read or verified.
pub async fn sync(dir: &Path, key: &StorageKey, store: &Store) -> Result<Report, String> {
    let old = read_manifest(store, key)
        .await?
        .unwrap_or_else(|| json!({ "files": {} }));
    let old_files = old.get("files").cloned().unwrap_or_default();
    let mut files = BTreeMap::new();
    let mut report = Report::default();
    for rel in walk(dir) {
        let Ok(bytes) = std::fs::read(dir.join(&rel)) else {
            continue;
        };
        let hash = Hash::of(&bytes).to_hex();
        let size = bytes.len() as u64;
        if old_files
            .pointer(&format!(
                "/{}/hash",
                rel.replace('~', "~0").replace('/', "~1")
            ))
            .and_then(Json::as_str)
            == Some(hash.as_str())
        {
            report.unchanged += 1;
        } else {
            store
                .put(&key.opaque_name(&rel), seal(key, &rel, &bytes))
                .await?;
            report.sent += 1;
            report.bytes += size;
        }
        files.insert(rel, json!({ "hash": hash, "size": size }));
    }
    let manifest = json!({ "version": 1, "time": hs_node::now_ms(), "files": files });
    let plain = serde_json::to_vec(&manifest).map_err(err)?;
    store
        .put(&key.opaque_name(MANIFEST), seal(key, MANIFEST, &plain))
        .await?;
    let keep: BTreeSet<String> = files
        .keys()
        .map(|r| key.opaque_name(r))
        .chain([key.opaque_name(MANIFEST)])
        .collect();
    for name in store.list().await? {
        if !keep.contains(&name) {
            store.delete(&name).await?;
            report.deleted += 1;
        }
    }
    Ok(report)
}

/// Restore a data directory from a replica. `dir` must be empty or absent.
/// Returns how many files were written.
///
/// # Errors
///
/// Returns an error if the destination is occupied, the manifest cannot be opened, or a file cannot
/// be restored.
pub async fn restore(store: &Store, storage_key: [u8; 32], dir: &Path) -> Result<usize, String> {
    if dir.exists() && std::fs::read_dir(dir).map_err(err)?.next().is_some() {
        return Err(format!("{} is not empty", dir.display()));
    }
    let key = StorageKey::new(storage_key);
    let manifest = read_manifest(store, &key)
        .await?
        .ok_or("no replica here (no manifest)")?;
    let files = manifest
        .get("files")
        .and_then(Json::as_object)
        .ok_or("the manifest lists no files")?;
    let mut n = 0;
    for (rel, meta) in files {
        if rel.split('/').any(|p| p == ".." || p.is_empty()) || rel.starts_with('/') {
            return Err(format!("bad path in manifest: {rel}"));
        }
        let want = meta.get("hash").and_then(Json::as_str).unwrap_or("");
        let sealed = store
            .get(&key.opaque_name(rel))
            .await?
            .ok_or_else(|| format!("{rel} is missing from the replica"))?;
        let bytes = key
            .open(rel, &sealed)
            .ok_or_else(|| format!("{rel} does not open"))?;
        if Hash::of(&bytes).to_hex() != want {
            return Err(format!("{rel} does not match the manifest"));
        }
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(err)?;
        }
        std::fs::write(&path, &bytes).map_err(err)?;
        n += 1;
    }
    let keys = dir.join("keys");
    std::fs::create_dir_all(&keys).map_err(err)?;
    std::fs::write(
        keys.join("storage"),
        data_encoding::HEXLOWER.encode(&storage_key),
    )
    .map_err(err)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for f in std::fs::read_dir(&keys).map_err(err)?.flatten() {
            let _ = std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o600));
        }
    }
    Ok(n)
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
        assert!(Store::parse("ftp://x").is_err());
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
        for name in store.list().await.unwrap() {
            let b = store.get(&name).await.unwrap().unwrap();
            assert!(!b.windows(3).any(|w| w == b"one" || w == b"see"));
        }
        std::fs::write(src.join("logs/aa/head"), b"two").unwrap();
        std::fs::remove_file(src.join("config.json")).unwrap();
        let r = sync(&src, &key, &store).await.unwrap();
        assert_eq!((r.sent, r.unchanged, r.deleted), (1, 1, 1));
        let dst = tmp("dst");
        assert!(restore(&store, [8u8; 32], &dst).await.is_err());
        let _ = std::fs::remove_dir_all(&dst);
        assert_eq!(restore(&store, seed, &dst).await.unwrap(), 2);
        assert_eq!(std::fs::read(dst.join("logs/aa/head")).unwrap(), b"two");
        assert_eq!(std::fs::read(dst.join("keys/node")).unwrap(), b"seed");
        assert!(!dst.join("index.db").exists());
        for p in [src, rep, dst] {
            let _ = std::fs::remove_dir_all(p);
        }
    }
}
