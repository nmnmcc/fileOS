// SPDX-License-Identifier: AGPL-3.0-only

//! fileOS on disk.
//!
//! One data directory has everything of one node:
//!
//! ```text
//! node.key                the seed of the node key (32 bytes)
//! identity-*.key          the keys of an identity, on the node that holds them
//! recipients.key          the secrets this node gets keys with (the newest two)
//! home                    the ID of the home space of the node
//! blobs/<hash>            content: chunks and lists of chunks, by their hash
//! spaces/<id>.log         the entries of each space, one after the other
//! spaces/<id>.base        the snapshot at the last compaction, if any
//! local/<app>.kv          cache files of each app, on this node only
//! auth.cbor               sign-in: passkeys and refresh tokens (hashed)
//! gateway.token           the token for the local gateway
//! gateway.addr            the address the gateway listens on, while it runs
//! ```
//!
//! A copy of the directory is a backup.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use fileos_core::{Bytes, Error, Hash, SecretKey, decode, encode};
use fileos_space::{Contents, Entry, Snapshot, Space};

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
        for sub in ["blobs", "spaces", "local"] {
            let path = dir.join(sub);
            fs::create_dir_all(&path).map_err(|e| io_error(&path, &e))?;
        }
        Ok(Self {
            dir: dir.to_path_buf(),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn read_or_make(
        &self,
        name: &str,
        make: impl FnOnce() -> Result<Vec<u8>, Error>,
    ) -> Result<Vec<u8>, Error> {
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
        let seed: [u8; 32] = seed.try_into().map_err(|bytes: Vec<u8>| {
            Error(format!("node.key has {} bytes, not 32", bytes.len()))
        })?;
        Ok(SecretKey::from_seed(&seed))
    }

    /// A secret key of the identity that this node holds, by its use
    /// (`rotation` or `signing`), if this node holds it. Most nodes do not:
    /// the keys of an identity are on one device only.
    ///
    /// # Errors
    ///
    /// When the file is there but cannot be read, or is broken.
    pub fn identity_key(&self, usage: &str) -> Result<Option<SecretKey>, Error> {
        let name = format!("identity-{usage}.key");
        let path = self.dir.join(&name);
        let seed = match fs::read(&path) {
            Ok(seed) => seed,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(io_error(&path, &e)),
        };
        let seed: [u8; 32] = seed
            .try_into()
            .map_err(|bytes: Vec<u8>| Error(format!("{name} has {} bytes, not 32", bytes.len())))?;
        Ok(Some(SecretKey::from_seed(&seed)))
    }

    /// Keeps a secret key of an identity on this node, by its use.
    ///
    /// # Errors
    ///
    /// When this node already has a key for that use, or the file cannot be
    /// written.
    pub fn set_identity_key(&self, usage: &str, key: &SecretKey) -> Result<(), Error> {
        if self.identity_key(usage)?.is_some() {
            return Err(Error(format!(
                "this node already has an identity {usage} key"
            )));
        }
        write_atomic(&self.dir.join(format!("identity-{usage}.key")), &key.seed())
    }

    /// The recipient secrets of this node, with the time each was made.
    ///
    /// # Errors
    ///
    /// When the file is there but cannot be read, or is broken.
    pub fn recipients(&self) -> Result<Vec<(u64, [u8; 32])>, Error> {
        let path = self.dir.join("recipients.key");
        let bytes = match fs::read(&path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(io_error(&path, &e)),
        };
        let kept: Vec<(u64, Bytes)> = decode(&bytes)?;
        kept.into_iter()
            .map(|(t, k)| {
                let k: [u8; 32] = k.0.try_into().map_err(|bytes: Vec<u8>| {
                    Error(format!(
                        "recipients.key has a secret of {} bytes, not 32",
                        bytes.len()
                    ))
                })?;
                Ok((t, k))
            })
            .collect()
    }

    /// Keeps the recipient secrets of this node, in place of the ones before:
    /// a secret that is not given again is destroyed.
    ///
    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn set_recipients(&self, kept: &[(u64, [u8; 32])]) -> Result<(), Error> {
        let kept: Vec<(u64, Bytes)> = kept.iter().map(|(t, k)| (*t, Bytes(k.to_vec()))).collect();
        write_atomic(&self.dir.join("recipients.key"), &encode(&kept))
    }

    /// The sign-in state of this node, as `fileos-auth` keeps it.
    ///
    /// # Errors
    ///
    /// When the file is there but cannot be read.
    pub fn auth(&self) -> Result<Option<Vec<u8>>, Error> {
        let path = self.dir.join("auth.cbor");
        match fs::read(&path) {
            Ok(bytes) => Ok(Some(bytes)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error(&path, &e)),
        }
    }

    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn set_auth(&self, bytes: &[u8]) -> Result<(), Error> {
        write_atomic(&self.dir.join("auth.cbor"), bytes)
    }

    /// The token that a local client gives to the gateway. It is made the first time.
    ///
    /// # Errors
    ///
    /// When the token file cannot be read or made.
    pub fn token(&self) -> Result<String, Error> {
        let bytes = self.read_or_make("gateway.token", || {
            Ok(Hash(random_seed()?).to_string().into_bytes())
        })?;
        String::from_utf8(bytes).map_err(|e| Error(format!("gateway.token is not text: {e}")))
    }

    /// The ID of the home space, if there is one yet.
    ///
    /// # Errors
    ///
    /// When the file is there but broken.
    pub fn home(&self) -> Result<Option<Hash>, Error> {
        match fs::read_to_string(self.dir.join("home")) {
            Ok(text) => text.trim().parse().map(Some),
            Err(_) => Ok(None),
        }
    }

    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn set_home(&self, home: Hash) -> Result<(), Error> {
        write_atomic(&self.dir.join("home"), home.to_string().as_bytes())
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

    /// Keeps a piece of content under its hash, if the hash is right.
    ///
    /// # Errors
    ///
    /// When the hash is wrong, or the file cannot be written.
    pub fn put_piece(&self, hash: Hash, data: &[u8]) -> Result<(), Error> {
        if Hash::of(data) != hash {
            return Err(Error(format!("content that is not {hash}")));
        }
        let path = self.blob_path(hash);
        if !path.exists() {
            write_atomic(&path, data)?;
        }
        Ok(())
    }

    fn log_path(&self, space: Hash) -> PathBuf {
        self.dir.join("spaces").join(format!("{space}.log"))
    }

    fn base_path(&self, space: Hash) -> PathBuf {
        self.dir.join("spaces").join(format!("{space}.base"))
    }

    /// Adds entries to the end of the log of a space.
    ///
    /// # Errors
    ///
    /// When the log cannot be written.
    pub fn append_entries(&self, space: Hash, entries: &[Entry]) -> Result<(), Error> {
        if entries.is_empty() {
            return Ok(());
        }
        let path = self.log_path(space);
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| io_error(&path, &e))?;
        let out = frame(entries);
        file.write_all(&out)
            .and_then(|()| file.sync_data())
            .map_err(|e| io_error(&path, &e))
    }

    /// Keeps a space as it is now: its base and its entries. Used after
    /// compaction, when entries before the base are dropped.
    ///
    /// # Errors
    ///
    /// When the files cannot be written.
    pub fn save_space(&self, space: &Space) -> Result<(), Error> {
        if let Some(base) = space.base() {
            write_atomic(&self.base_path(space.id()), &encode(base))?;
        }
        write_atomic(&self.log_path(space.id()), &frame(&space.entries()))
    }

    /// Reads every space: from its base, if it has one, and its log.
    ///
    /// # Errors
    ///
    /// When a file cannot be read, or is broken.
    pub fn load_spaces(&self) -> Result<Vec<Space>, Error> {
        let dir = self.dir.join("spaces");
        let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
            .map_err(|e| io_error(&dir, &e))?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "log"))
            .collect();
        paths.sort();
        let mut spaces = Vec::new();
        for path in paths {
            let bytes = fs::read(&path).map_err(|e| io_error(&path, &e))?;
            let mut entries =
                read_entries(&bytes).map_err(|e| Error(format!("{}: {e}", path.display())))?;
            let base = path.with_extension("base");
            let mut space = if base.exists() {
                let bytes = fs::read(&base).map_err(|e| io_error(&base, &e))?;
                let snapshot: Snapshot = decode(&bytes)?;
                Space::from_snapshot(snapshot)?
            } else if entries.is_empty() {
                continue;
            } else {
                Space::from_genesis(&entries.remove(0))?
            };
            // Entries that are not good any more are left out.
            let _ = space.insert(entries);
            spaces.push(space);
        }
        Ok(spaces)
    }
}

impl Store {
    /// The cache of an app on this node: it does not sync, and can be made
    /// again.
    pub fn kv_load(&self, app: &str) -> BTreeMap<String, Vec<u8>> {
        fs::read(self.kv_path(app))
            .ok()
            .and_then(|bytes| decode::<BTreeMap<String, Bytes>>(&bytes).ok())
            .map(|map| map.into_iter().map(|(k, v)| (k, v.0)).collect())
            .unwrap_or_default()
    }

    /// # Errors
    ///
    /// When the file cannot be written.
    pub fn kv_save(&self, app: &str, map: &BTreeMap<String, Vec<u8>>) -> Result<(), Error> {
        let map: BTreeMap<&String, Bytes> =
            map.iter().map(|(k, v)| (k, Bytes(v.clone()))).collect();
        write_atomic(&self.kv_path(app), &encode(&map))
    }

    fn kv_path(&self, app: &str) -> PathBuf {
        let safe: String = app
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        self.dir.join("local").join(format!("{safe}.kv"))
    }
}

fn frame(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in entries {
        let bytes = encode(entry);
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(&bytes);
    }
    out
}

impl Contents for Store {
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
        let (len, rest) = bytes
            .split_first_chunk::<4>()
            .ok_or_else(|| Error("cut short".into()))?;
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
    use fileos_space::{Act, Content, Principal};

    use super::*;

    #[test]
    fn keeps_everything() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path()).unwrap();
        let key = store.key().unwrap();
        assert_eq!(store.key().unwrap().public(), key.public());
        let token = store.token().unwrap();
        assert_eq!(store.token().unwrap(), token);

        let (mut space, genesis) = Space::create(Principal::Key(key.public()), &key, 0, 1);
        store.put_piece(Hash::of(b"hello"), b"hello").unwrap();
        let entry = space
            .write(
                &key,
                1,
                vec![Act::Create {
                    name: "a".into(),
                    content: Content::of(b"hello"),
                    key: None,
                }],
            )
            .unwrap();
        store.append_entries(space.id(), &[genesis]).unwrap();
        store.append_entries(space.id(), &[entry]).unwrap();
        store.set_home(space.id()).unwrap();

        let again = Store::open(dir.path()).unwrap();
        let spaces = again.load_spaces().unwrap();
        assert_eq!(spaces.len(), 1);
        assert_eq!(spaces[0].digest(), space.digest());
        assert_eq!(again.home().unwrap(), Some(space.id()));
        assert_eq!(
            again.get(Hash::of(b"hello")).as_deref(),
            Some(&b"hello"[..])
        );

        // After compaction, the base and the rest are kept.
        space
            .checkpoint(&key, 2, fileos_space::Retention::default())
            .unwrap();
        assert!(space.compact());
        again.save_space(&space).unwrap();
        let spaces = again.load_spaces().unwrap();
        assert_eq!(spaces[0].digest(), space.digest());
        assert!(spaces[0].base().is_some());
    }
}
