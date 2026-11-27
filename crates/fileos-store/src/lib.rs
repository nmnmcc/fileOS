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
//! spaces/<id>.log         the snapshot and later entries of each space
//! spaces/<id>.base        the snapshot in older logs
//! local/<app>.kv          cache files of each app, on this node only
//! gateway.token           the token for the local gateway
//! gateway.addr            the address the gateway listens on, while it runs
//! ```
//!
//! A copy of the directory is a backup.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufReader, Read as _, Seek as _, Write as _};
use std::path::{Path, PathBuf};

use fileos_core::{Bytes, Error, Hash, SecretKey, decode, encode};
use fileos_space::{Contents, Entry, Snapshot, Space};

/// The data directory of one node.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
    entropy: fn() -> Result<[u8; 32], Error>,
}

fn io_error(path: &Path, error: io::Error) -> Error {
    Error::from(error).context(path.display())
}

/// 32 random bytes from the operating system.
///
/// # Errors
///
/// When the operating system has no source of randomness.
pub fn random_seed() -> Result<[u8; 32], Error> {
    let mut seed = [0; 32];
    getrandom::fill(&mut seed).map_err(|e| Error::internal(format!("no randomness: {e}")))?;
    Ok(seed)
}

impl Store {
    /// Opens a data directory, and makes it if it is not there.
    ///
    /// # Errors
    ///
    /// When the directories cannot be made.
    pub fn open(dir: impl AsRef<Path>) -> Result<Self, Error> {
        Self::open_with_entropy(dir.as_ref(), random_seed)
    }

    /// Opens a store with a source of random bytes supplied by the caller.
    ///
    /// # Errors
    ///
    /// When the directories cannot be made.
    pub fn open_with_entropy(
        dir: &Path,
        entropy: fn() -> Result<[u8; 32], Error>,
    ) -> Result<Self, Error> {
        for sub in ["blobs", "spaces", "local"] {
            let path = dir.join(sub);
            fs::create_dir_all(&path).map_err(|e| io_error(&path, e))?;
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            entropy,
        })
    }

    /// Gets random bytes for a key or a space ID.
    ///
    /// # Errors
    ///
    /// When the random source fails.
    pub fn seed(&self) -> Result<[u8; 32], Error> {
        (self.entropy)()
    }

    /// The path to the data directory.
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
            Err(e) => Err(io_error(&path, e)),
        }
    }

    /// The key of this node. It is made the first time.
    ///
    /// # Errors
    ///
    /// When the key file cannot be read or made, or is broken.
    pub fn key(&self) -> Result<SecretKey, Error> {
        let seed = self.read_or_make("node.key", || self.seed().map(Vec::from))?;
        let seed: [u8; 32] = seed.try_into().map_err(|bytes: Vec<u8>| {
            Error::invalid(format!("node.key has {} bytes, not 32", bytes.len()))
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
            Err(e) => return Err(io_error(&path, e)),
        };
        let seed: [u8; 32] = seed.try_into().map_err(|bytes: Vec<u8>| {
            Error::invalid(format!("{name} has {} bytes, not 32", bytes.len()))
        })?;
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
            return Err(Error::invalid(format!(
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
            Err(e) => return Err(io_error(&path, e)),
        };
        let kept: Vec<(u64, Bytes)> = decode(&bytes)?;
        kept.into_iter()
            .map(|(t, k)| {
                let k: [u8; 32] = k.0.try_into().map_err(|bytes: Vec<u8>| {
                    Error::invalid(format!(
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

    /// The token that a local client gives to the gateway. It is made the first time.
    ///
    /// # Errors
    ///
    /// When the token file cannot be read or made.
    pub fn token(&self) -> Result<String, Error> {
        let bytes = self.read_or_make("gateway.token", || {
            Ok(Hash(self.seed()?).to_string().into_bytes())
        })?;
        String::from_utf8(bytes)
            .map_err(|e| Error::invalid(format!("gateway.token is not text: {e}")))
    }

    /// The ID of the home space, if there is one yet.
    ///
    /// # Errors
    ///
    /// When the file is there but broken.
    pub fn home(&self) -> Result<Option<Hash>, Error> {
        let path = self.dir.join("home");
        match fs::read_to_string(&path) {
            Ok(text) => text.trim().parse().map(Some),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(io_error(&path, e)),
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
            return Err(Error::invalid(format!("content that is not {hash}")));
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

    fn lock_logs(&self) -> Result<fs::File, Error> {
        let path = self.dir.join("spaces").join(".lock");
        let file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .map_err(|e| io_error(&path, e))?;
        file.lock().map_err(|e| io_error(&path, e))?;
        Ok(file)
    }

    /// Adds entries to a space log after removing any incomplete tail.
    ///
    /// # Errors
    ///
    /// When the log cannot be written or its snapshot is incomplete.
    pub fn append_entries(&self, space: Hash, entries: &[Entry]) -> Result<(), Error> {
        if entries.is_empty() {
            return Ok(());
        }
        let _lock = self.lock_logs()?;
        let path = self.log_path(space);
        let mut out = Vec::new();
        for entry in entries {
            push_frame(&mut out, &encode(entry))?;
        }
        let mut file = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| io_error(&path, e))?;
        let len = file.metadata().map_err(|e| io_error(&path, e))?.len();
        let end = log_end(&file, len).map_err(|e| io_error(&path, e))?;
        if end != len {
            file.set_len(end).map_err(|e| io_error(&path, e))?;
        }
        file.seek(io::SeekFrom::Start(end))
            .map_err(|e| io_error(&path, e))?;
        if let Err(error) = file.write_all(&out).and_then(|()| file.sync_all()) {
            file.set_len(end)
                .and_then(|()| file.sync_all())
                .map_err(|e| {
                    io_error(&path, e)
                        .context(format_args!("could not undo a failed append ({error})"))
                })?;
            return Err(io_error(&path, error));
        }
        if len == 0 {
            sync_parent(&path).map_err(|e| io_error(&path, e))?;
        }
        Ok(())
    }

    /// Replaces a space's snapshot and log in one write after compaction.
    ///
    /// # Errors
    ///
    /// When the log cannot be written.
    pub fn save_space(&self, space: &Space) -> Result<(), Error> {
        let _lock = self.lock_logs()?;
        let mut out = Vec::from(LOG_HEADER);
        let base = space.base().map(encode).unwrap_or_default();
        push_frame(&mut out, &base)?;
        for entry in space.iter() {
            push_frame(&mut out, &encode(entry))?;
        }
        let path = self.log_path(space.id());
        write_atomic(&path, &out)?;
        // The new log is now complete. An old snapshot is no longer needed.
        let _ = fs::remove_file(self.base_path(space.id()));
        Ok(())
    }

    /// Reads each space from its snapshot and complete log entries.
    ///
    /// # Errors
    ///
    /// When a file cannot be read, or a complete frame is broken.
    pub fn load_spaces(&self) -> Result<Vec<Space>, Error> {
        let _lock = self.lock_logs()?;
        let dir = self.dir.join("spaces");
        let mut paths = fs::read_dir(&dir)
            .map_err(|e| io_error(&dir, e))?
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| io_error(&dir, e))?;
        paths.retain(|p| p.extension().is_some_and(|x| x == "log"));
        paths.sort();
        let mut spaces = Vec::new();
        for path in paths {
            let bytes = fs::read(&path).map_err(|e| io_error(&path, e))?;
            let (base, entries) = read_log(&path, &bytes).map_err(|e| e.context(path.display()))?;
            let mut entries = entries.into_iter();
            let mut space = if let Some(snapshot) = base {
                Space::restore_local_snapshot(snapshot)?
            } else if let Some(genesis) = entries.next() {
                Space::from_genesis(&genesis)?
            } else {
                continue;
            };
            // Old logs can include entries before their snapshot.
            let _ = space.insert(entries.collect());
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

// A zero-length entry cannot occur in an old log. It marks a log that
// starts with a snapshot frame. A zero-length snapshot means no base.
const LOG_HEADER: [u8; 4] = [0; 4];

fn push_frame(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), Error> {
    let len = u32::try_from(bytes.len())
        .map_err(|e| Error::invalid("a log frame is too large".into()).with_source(e))?;
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

/// Finds the end of the last complete frame without decoding the entries.
fn log_end(file: &fs::File, len: u64) -> io::Result<u64> {
    let mut reader = BufReader::new(file);
    let mut pos = 0;
    let mut needs_base = false;
    while len - pos >= 4 {
        let mut size = [0; 4];
        reader.read_exact(&mut size)?;
        let size = u32::from_be_bytes(size);
        if pos == 0 && size == 0 {
            needs_base = true;
        } else if u64::from(size) <= len - pos - 4 {
            needs_base = false;
        } else {
            break;
        }
        reader.seek_relative(i64::from(size))?;
        pos += 4 + u64::from(size);
    }
    if needs_base {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "a cut snapshot",
        ));
    }
    Ok(pos)
}

impl Contents for Store {
    fn has(&self, hash: Hash) -> bool {
        self.blob_path(hash).exists()
    }

    fn get(&self, hash: Hash) -> Option<Vec<u8>> {
        fs::read(self.blob_path(hash)).ok()
    }
}

fn take_frame<'a>(bytes: &mut &'a [u8]) -> Option<&'a [u8]> {
    let (len, rest) = bytes.split_first_chunk::<4>()?;
    let (frame, rest) = rest.split_at_checked(u32::from_be_bytes(*len) as usize)?;
    *bytes = rest;
    Some(frame)
}

fn read_log(path: &Path, bytes: &[u8]) -> Result<(Option<Snapshot>, Vec<Entry>), Error> {
    let mut tail = bytes;
    let base = if let Some(rest) = tail.strip_prefix(&LOG_HEADER) {
        tail = rest;
        let frame = take_frame(&mut tail).ok_or_else(|| Error::invalid("a cut snapshot".into()))?;
        if frame.is_empty() {
            None
        } else {
            Some(decode(frame)?)
        }
    } else {
        let base = path.with_extension("base");
        match fs::read(&base) {
            Ok(bytes) => Some(decode(&bytes)?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(io_error(&base, e)),
        }
    };
    let mut entries = Vec::new();
    while let Some(frame) = take_frame(&mut tail) {
        entries.push(decode(frame)?);
    }
    Ok((base, entries))
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Writes a file so that it is never seen half written.
///
/// # Errors
///
/// When the file cannot be written or synced.
pub fn write_atomic(path: &Path, data: &[u8]) -> Result<(), Error> {
    write_replacement(path, data).map_err(|e| io_error(path, e))
}

fn write_replacement(path: &Path, data: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(data)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)?;
    sync_parent(path)
}

#[cfg(test)]
mod tests {
    use fileos_space::{Act, Content, Principal};

    use super::*;

    #[test]
    fn a_cut_log_accepts_new_writes_after_restart() {
        let key = SecretKey::from_seed(&[7; 32]);
        let (mut space, genesis) = Space::create(Principal::Key(key.public()), &key, 0, 1);
        let entry = space
            .write(
                &key,
                1,
                vec![Act::Create {
                    name: "first".into(),
                    content: Content::of(b"first"),
                    key: None,
                }],
            )
            .unwrap();
        let mut frame = Vec::new();
        push_frame(&mut frame, &encode(&entry)).unwrap();
        for prefix in [&frame[..2], &frame[..9]] {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(dir.path()).unwrap();
            store
                .append_entries(space.id(), std::slice::from_ref(&genesis))
                .unwrap();
            let mut file = fs::OpenOptions::new()
                .append(true)
                .open(store.log_path(space.id()))
                .unwrap();
            file.write_all(prefix).unwrap();
            drop(file);

            let mut spaces = store.load_spaces().unwrap();
            let restored = &mut spaces[0];
            assert_eq!(restored.find("first"), []);
            let next = restored
                .write(
                    &key,
                    2,
                    vec![Act::Create {
                        name: "next".into(),
                        content: Content::of(b"next"),
                        key: None,
                    }],
                )
                .unwrap();
            store.append_entries(restored.id(), &[next]).unwrap();
            let spaces = store.load_spaces().unwrap();
            assert_eq!(spaces[0].digest(), restored.digest());
        }
    }

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
        // Old stores keep the base in a separate file.
        let base_path = again.base_path(space.id());
        fs::write(&base_path, encode(space.base().unwrap())).unwrap();
        let mut frames = Vec::new();
        for entry in space.iter() {
            push_frame(&mut frames, &encode(entry)).unwrap();
        }
        fs::write(again.log_path(space.id()), frames).unwrap();
        let old = again.load_spaces().unwrap();
        assert_eq!(old[0].digest(), space.digest());
        again.save_space(&space).unwrap();
        assert!(!base_path.exists());
        let spaces = again.load_spaces().unwrap();
        assert_eq!(spaces[0].digest(), space.digest());
        assert!(spaces[0].base().is_some());
    }
}
