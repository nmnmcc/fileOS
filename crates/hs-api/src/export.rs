// SPDX-License-Identifier: AGPL-3.0-only

//! `GET /api/export`: the whole data directory as a tar archive, streamed.
//!
//! The archive holds the truth (config, identity log, space logs, blobs) and
//! leaves out what must not travel or can be rebuilt: `keys/` and the index.
//! It is for keeping and for reading with other tools; 0.1.0 does not restore
//! a node from it. `hs ctl restore` brings a node back from a replica.

use std::io::{BufWriter, Read as _, Write};
use std::path::{Path, PathBuf};

use bytes::Bytes;
use http::{StatusCode, header};
use tokio::sync::mpsc;

use crate::util::{Result, set_header};
use crate::{Api, bitwarden};
use hs_net::server::{Body, Response};

const CHUNK: usize = 256 * 1024;

fn skipped(rel: &Path) -> bool {
    let s = rel.to_string_lossy();
    s == "keys"
        || s.starts_with("keys/")
        || s.starts_with("index.db")
        || s.split('/')
            .any(|name| name.starts_with(".tmp-") || name.starts_with(".space-"))
        || s.ends_with(".tmp")
        || s.ends_with(".lock")
}

/// Every regular file under `dir`, relative to it, sorted.
fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(dir.join(&rel)) else {
            continue;
        };
        for e in rd.flatten() {
            let r = rel.join(e.file_name());
            if skipped(&r) {
                continue;
            }
            match e.file_type() {
                Ok(t) if t.is_dir() => stack.push(r),
                Ok(t) if t.is_file() => out.push(r),
                _ => {}
            }
        }
    }
    out.sort();
    out
}

/// Write the archive of a data directory to `out`.
///
/// # Errors
///
/// Returns an I/O error if a file cannot be read or the archive cannot be written.
pub fn archive(dir: &Path, out: impl Write) -> std::io::Result<()> {
    let mut tar = tar::Builder::new(out);
    for rel in files(dir) {
        let Ok(f) = std::fs::File::open(dir.join(&rel)) else {
            continue;
        };
        let meta = f.metadata()?;
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Regular);
        h.set_mode(0o644);
        h.set_size(meta.len());
        h.set_mtime(
            meta.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs()),
        );
        // Exactly the size in the header, even if the file changes meanwhile.
        let data = f
            .take(meta.len())
            .chain(std::io::repeat(0))
            .take(meta.len());
        tar.append_data(&mut h, Path::new("homeserver").join(&rel), data)?;
    }
    tar.into_inner()?.flush()
}

/// Sends what is written as chunks over a channel.
struct ChannelWriter(mpsc::Sender<Bytes>);

impl Write for ChannelWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .blocking_send(Bytes::copy_from_slice(buf))
            .map_err(|_| std::io::Error::other("client went away"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// The export response.
///
/// # Errors
///
/// Returns a server error if the streaming response cannot be built.
pub fn response(api: &Api) -> Result {
    let dir = api.node.dir().to_path_buf();
    let (tx, rx) = mpsc::channel(8);
    tokio::task::spawn_blocking(move || {
        let _ = archive(&dir, BufWriter::with_capacity(CHUNK, ChannelWriter(tx)));
    });
    let mut r = Response::new(Body::Stream(rx));
    *r.status_mut() = StatusCode::OK;
    set_header(&mut r, header::CONTENT_TYPE, "application/x-tar");
    let date = bitwarden::iso(hs_node::now_ms());
    let file = format!("attachment; filename=\"homeserver-{}.tar\"", &date[..10]);
    set_header(&mut r, header::CONTENT_DISPOSITION, &file);
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_lists_files() {
        let dir = std::env::temp_dir().join(format!("hs-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let long = format!("blobs/{}/{}", "a".repeat(80), "b".repeat(90));
        std::fs::create_dir_all(dir.join(&long).parent().unwrap()).unwrap();
        std::fs::create_dir_all(dir.join("keys")).unwrap();
        std::fs::write(dir.join("config.json"), b"{}").unwrap();
        std::fs::write(dir.join(&long), b"blob").unwrap();
        std::fs::write(dir.join("keys/node"), b"secret").unwrap();
        let mut out = Vec::new();
        archive(&dir, &mut out).unwrap();
        let mut a = tar::Archive::new(&out[..]);
        let mut names: Vec<String> = a
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                format!("homeserver/{long}"),
                "homeserver/config.json".into()
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn skips_keys_and_index() {
        assert!(skipped(Path::new("keys/node")));
        assert!(skipped(Path::new("index.db-wal")));
        assert!(!skipped(Path::new("logs/ab/head")));
    }
}
