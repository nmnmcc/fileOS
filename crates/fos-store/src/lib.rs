// SPDX-License-Identifier: AGPL-3.0-only

//! fileOS on disk.
//!
//! One data directory has everything of one node:
//!
//! ```text
//! node.key                the seed of the node key (32 bytes)
//! root                    the ID of the root folder
//! engine/Engine.app/      the engine of this node (it does not sync)
//! blobs/<hash>            file contents, named by their hash
//! folders/<id>.log        the entries of each folder, one after the other
//! local/<app>.kv          the key-value store of each app (it does not sync)
//! gateway.token           the token for the local gateway
//! gateway.addr            the address the gateway listens on, while it runs
//! ```
//!
//! A copy of the directory is a backup.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use fos_core::{Bytes, Error, Hash, SecretKey, decode, encode};
use fos_fs::{Authz, Entry, Folder, Reject};

/// The data directory of one node.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

fn io_error(path: &Path, e: &io::Error) -> Error {
    Error(format!("{}: {e}", path.display()))
}

/// 32 random bytes from the operating system.
///
/// # Errors
///
/// When the operating system has no source of randomness.
pub fn random_seed() -> Result<[u8; 32], Error> {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).map_err(|e| Error(format!("no randomness: {e}")))?;
    Ok(seed)
}

impl Store {
    /// Opens a data directory, and makes it if it is not there.
    ///
    /// # Errors
    ///
    /// When the directories cannot be made.
    pub fn open(dir: &Path) -> Result<Self, Error> {
        for sub in ["blobs", "folders", "local", "engine"] {
            let path = dir.join(sub);
            fs::create_dir_all(&path).map_err(|e| io_error(&path, &e))?;
        }
        Ok(Self { dir: dir.to_path_buf() })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn read_or_make(&self, name: &str, make: impl FnOnce() -> Result<Vec<u8>, Error>) -> Result<Vec<u8>, Error> {
        let path = self.dir.join(name);
        match fs::read(&path) {
            Ok(bytes) => Ok(bytes),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let bytes = make()?;
                write_atomic(&path, &bytes)?;
                Ok(bytes)
            }
            Err(e) => Err(io_error(&path, &e)),
        }
    }

    /// The key of this node. It is made the first time.
    ///
    /// # Errors
    ///
    /// When the key file cannot be read or made, or is broken.
    pub fn key(&self) -> Result<SecretKey, Error> {
        let seed = self.read_or_make("node.key", || random_seed().map(Vec::from))?;
        let seed: [u8; 32] = seed.try_into().map_err(|_| Error("node.key is not 32 bytes".into()))?;
        Ok(SecretKey::from_seed(&seed))
    }

    /// The token that a local client gives to the gateway. It is made the first time.
    ///
    /// # Errors
    ///
    /// When the token file cannot be read or made.
    pub fn token(&self) -> Result<String, Error> {
        let bytes = self.read_or_make("gateway.token", || Ok(Hash(random_seed()?).to_string().into_bytes()))?;
        String::from_utf8(bytes).map_err(|_| Error("gateway.token is not text".into()))
    }

    /// The ID of the root folder, if there is one yet.
    ///
    /// # Errors
    ///
    /// When the file is there but broken.
    pub fn root(&self) -> Result<Option<Hash>, Error> {
        match fs::read_to_string(self.dir.join("root")) {
            Ok(text) => text.trim().parse().map(Some),
            Err(_) => Ok(None),
        }
    }

    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn set_root(&self, root: Hash) -> Result<(), Error> {
        write_atomic(&self.dir.join("root"), root.to_string().as_bytes())
    }

    /// The directory of the engine of this node.
    pub fn engine_dir(&self) -> PathBuf {
        self.dir.join("engine").join("Engine.app")
    }

    /// Keeps a blob. Gives its hash.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn put_blob(&self, data: &[u8]) -> Result<Hash, Error> {
        let hash = Hash::of(data);
        let path = self.blob_path(hash);
        if !path.exists() {
            write_atomic(&path, data)?;
        }
        Ok(hash)
    }

    fn blob_path(&self, hash: Hash) -> PathBuf {
        self.dir.join("blobs").join(hash.to_string())
    }

    /// Adds entries to the end of the log of a folder.
    ///
    /// # Errors
    ///
    /// When the log cannot be written.
    pub fn append_entries(&self, folder: Hash, entries: &[Entry]) -> Result<(), Error> {
        let path = self.dir.join("folders").join(format!("{folder}.log"));
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| io_error(&path, &e))?;
        let mut out = Vec::new();
        for entry in entries {
            let bytes = encode(entry);
            out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            out.extend_from_slice(&bytes);
        }
        file.write_all(&out).and_then(|()| file.sync_data()).map_err(|e| io_error(&path, &e))
    }

    /// Reads every folder from its log. The engine checks every entry again.
    ///
    /// # Errors
    ///
    /// When a log cannot be read, or is broken.
    pub fn load_folders(&self, authz: &mut dyn Authz) -> Result<Vec<Folder>, Error> {
        let dir = self.dir.join("folders");
        let mut folders = Vec::new();
        let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
            .map_err(|e| io_error(&dir, &e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "log"))
            .collect();
        paths.sort();
        for path in paths {
            let bytes = fs::read(&path).map_err(|e| io_error(&path, &e))?;
            let mut entries = read_entries(&bytes).map_err(|e| Error(format!("{}: {e}", path.display())))?;
            if entries.is_empty() {
                continue;
            }
            let mut folder = Folder::from_genesis(entries.remove(0), authz)?;
            for entry in entries {
                match folder.accept(entry, authz) {
                    Ok(()) | Err(Reject::Old) => {}
                    Err(e) => return Err(Error(format!("{}: entry not taken again: {e:?}", path.display()))),
                }
            }
            folders.push(folder);
        }
        Ok(folders)
    }

    /// The key-value store of an app.
    pub fn kv_load(&self, app: &str) -> BTreeMap<String, Vec<u8>> {
        let path = self.kv_path(app);
        fs::read(path)
            .ok()
            .and_then(|bytes| decode::<BTreeMap<String, Bytes>>(&bytes).ok())
            .map(|map| map.into_iter().map(|(k, v)| (k, v.0)).collect())
            .unwrap_or_default()
    }

    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn kv_save(&self, app: &str, map: &BTreeMap<String, Vec<u8>>) -> Result<(), Error> {
        let map: BTreeMap<&String, Bytes> = map.iter().map(|(k, v)| (k, Bytes(v.clone()))).collect();
        write_atomic(&self.kv_path(app), &encode(&map))
    }

    fn kv_path(&self, app: &str) -> PathBuf {
        let safe: String = app.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect();
        self.dir.join("local").join(format!("{safe}.kv"))
    }
}

impl fos_sync::Blobs for Store {
    fn has(&self, hash: Hash) -> bool {
        self.blob_path(hash).exists()
    }

    fn get(&self, hash: Hash) -> Option<Vec<u8>> {
        fs::read(self.blob_path(hash)).ok()
    }
}

fn read_entries(mut bytes: &[u8]) -> Result<Vec<Entry>, Error> {
    let mut entries = Vec::new();
    while !bytes.is_empty() {
        let (len, rest) = bytes.split_first_chunk::<4>().ok_or_else(|| Error("cut short".into()))?;
        let len = u32::from_be_bytes(*len) as usize;
        if rest.len() < len {
            // A write that stopped half way, for example at a power cut.
            // The entries before it are good.
            break;
        }
        entries.push(decode(&rest[..len])?);
        bytes = &rest[len..];
    }
    Ok(entries)
}

/// Writes a file so that it is never seen half written.
///
/// # Errors
///
/// When the file cannot be written.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), Error> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, data)
        .and_then(|()| fs::rename(&tmp, path))
        .map_err(|e| io_error(path, &e))
}

#[cfg(test)]
mod tests {
    use fos_fs::{Op, Open, SignedWrite};
    use fos_sync::Blobs as _;

    use super::*;

    #[test]
    fn keeps_everything() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let key = store.key().unwrap();
        assert_eq!(store.key().unwrap().public(), key.public());
        let token = store.token().unwrap();
        assert_eq!(store.token().unwrap(), token);

        let mut open = Open::default();
        let (mut folder, genesis) = Folder::create("f", &key, &mut open);
        let hash = store.put_blob(b"hello").unwrap();
        let op = Op::Put { path: "/a".into(), blob: hash, size: 5 };
        let entry = folder.order(SignedWrite::sign(Some(folder.id()), 1, op, &key), &key, &mut open).unwrap();
        store.append_entries(folder.id(), &[genesis]).unwrap();
        store.append_entries(folder.id(), &[entry]).unwrap();
        store.set_root(folder.id()).unwrap();

        let again = Store::open(dir.path()).unwrap();
        let folders = again.load_folders(&mut open).unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].head(), folder.head());
        assert_eq!(again.root().unwrap(), Some(folder.id()));
        assert_eq!(again.get(hash).as_deref(), Some(&b"hello"[..]));

        let mut kv = BTreeMap::new();
        kv.insert("k".to_string(), b"v".to_vec());
        again.kv_save("photos", &kv).unwrap();
        assert_eq!(again.kv_load("photos"), kv);
    }
}
