// SPDX-License-Identifier: AGPL-3.0-only

//! Objects in a local directory: the data directory itself, or a replica on
//! an external disk. A write goes to a temporary file, which is synced and
//! then renamed. So a crash never leaves a half-written object under its real
//! name.

use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use hs_core::Hash;

/// A directory of named objects. Names may contain `/` to form subdirectories.
#[derive(Clone, Debug)]
pub struct Dir {
    root: PathBuf,
}

fn check_name(name: &str) -> io::Result<()> {
    let ok = !name.is_empty()
        && !name.starts_with('/')
        && name
            .split('/')
            .all(|p| !p.is_empty() && p != "." && p != "..")
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b"-_./".contains(&c));
    if ok {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("bad object name {name:?}"),
        ))
    }
}

impl Dir {
    /// Open (and create) a directory.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the directory cannot be created or opened.
    pub fn open(root: impl AsRef<Path>) -> io::Result<Dir> {
        fs::create_dir_all(root.as_ref())?;
        Ok(Dir {
            root: root.as_ref().to_path_buf(),
        })
    }

    /// The directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, name: &str) -> io::Result<PathBuf> {
        check_name(name)?;
        Ok(self.root.join(name))
    }

    /// Read an object.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is not valid or the file cannot be read.
    pub fn get(&self, name: &str) -> io::Result<Option<Vec<u8>>> {
        match fs::read(self.path(name)?) {
            Ok(b) => Ok(Some(b)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Open an object for streaming or seeking.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is not valid or the file does not open.
    pub fn open_file(&self, name: &str) -> io::Result<Option<fs::File>> {
        match fs::File::open(self.path(name)?) {
            Ok(file) => Ok(Some(file)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Read part of an object.
    ///
    /// # Errors
    ///
    /// Returns an error if the name, range or file is not valid, or the range cannot be read in
    /// full.
    pub fn get_range(&self, name: &str, start: u64, end: u64) -> io::Result<Vec<u8>> {
        let mut f = fs::File::open(self.path(name)?)?;
        let file_len = f.metadata()?.len();
        let len = end
            .checked_sub(start)
            .and_then(|len| usize::try_from(len).ok())
            .filter(|_| end <= file_len)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid object range"))?;
        f.seek(SeekFrom::Start(start))?;
        let mut buf = vec![0; len];
        f.read_exact(&mut buf)?;
        Ok(buf)
    }

    /// The size of an object.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is not valid or metadata cannot be read.
    pub fn len(&self, name: &str) -> io::Result<Option<u64>> {
        match fs::metadata(self.path(name)?) {
            Ok(m) => Ok(Some(m.len())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Write an object atomically.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is not valid, or the object cannot be written to a temporary
    /// file, synced or renamed.
    pub fn put(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        let path = self.path(name)?;
        let parent = path.parent().unwrap_or(&self.root);
        fs::create_dir_all(parent)?;
        let mut file = tempfile::Builder::new()
            .prefix(".tmp-")
            .tempfile_in(parent)?;
        file.write_all(bytes)?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|e| e.error)?;
        Ok(())
    }

    /// Publish an object, and on Unix flush its directory entries to storage.
    ///
    /// # Errors
    ///
    /// Returns an error if the object or the directories it is in cannot be synced.
    pub fn put_durable(&self, name: &str, bytes: &[u8]) -> io::Result<()> {
        self.put(name, bytes)?;
        #[cfg(unix)]
        {
            let path = self.path(name)?;
            for parent in path.ancestors().skip(1) {
                fs::File::open(parent)?.sync_all()?;
                if parent == self.root {
                    break;
                }
            }
        }
        Ok(())
    }

    /// Write only if the hash of the current content is `expected` (or, when
    /// `expected` is `None`, the object does not exist). Returns whether it
    /// wrote.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is not valid, the directory cannot be locked, or I/O fails.
    pub fn put_if(&self, name: &str, bytes: &[u8], expected: Option<Hash>) -> io::Result<bool> {
        // Each caller has its own descriptor, so separate processes also go one at a time.
        let lock = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join(".tmp-cas.lock"))?;
        lock.lock()?;
        let current = self.get(name)?.map(|b| Hash::of(&b));
        if current != expected {
            return Ok(false);
        }
        self.put_durable(name, bytes)?;
        Ok(true)
    }

    /// Delete an object. Deleting a missing object is not an error.
    ///
    /// # Errors
    ///
    /// Returns an error if the name is not valid or the operating system does not let it delete.
    pub fn delete(&self, name: &str) -> io::Result<()> {
        match fs::remove_file(self.path(name)?) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }

    /// List object names under a prefix (a directory path, or "" for all).
    ///
    /// # Errors
    ///
    /// Returns an error if the prefix is not valid or reading the directory fails.
    pub fn list(&self, prefix: &str) -> io::Result<Vec<String>> {
        let mut out = Vec::new();
        let start = if prefix.is_empty() {
            self.root.clone()
        } else {
            self.path(prefix.trim_end_matches('/'))?
        };
        if !start.exists() {
            return Ok(out);
        }
        let mut stack = vec![start];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir)? {
                let entry = entry?;
                let path = entry.path();
                if entry.file_type()?.is_dir() {
                    stack.push(path);
                } else if let Ok(rel) = path.strip_prefix(&self.root) {
                    let name = rel.to_string_lossy().replace('\\', "/");
                    if !name.contains(".tmp") {
                        out.push(name);
                    }
                }
            }
        }
        out.sort();
        Ok(out)
    }
}

/// Where a blob is in a directory: `ab/cdef…`. Two levels keep directories
/// small.
pub fn blob_name(hash: &Hash) -> String {
    let hex = hash.to_hex();
    format!("{}/{}", &hex[..2], &hex[2..])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_writes_keep_distinct_objects_separate() {
        let root = tempfile::tempdir().unwrap();
        let dir = Dir::open(root.path()).unwrap();
        let first = vec![1; 512 * 1024];
        let second = vec![2; 512 * 1024];
        let start = std::sync::Barrier::new(2);
        std::thread::scope(|scope| {
            scope.spawn(|| {
                start.wait();
                for _ in 0..20 {
                    dir.put("report.json", &first).unwrap();
                }
            });
            scope.spawn(|| {
                start.wait();
                for _ in 0..20 {
                    dir.put("report.txt", &second).unwrap();
                }
            });
        });
        assert_eq!(dir.get("report.json").unwrap(), Some(first));
        assert_eq!(dir.get("report.txt").unwrap(), Some(second));
    }

    #[test]
    fn put_get_list_conditional() {
        let tmp = std::env::temp_dir().join(format!("hs-dir-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        let d = Dir::open(&tmp).unwrap();
        d.put("a/b", b"one").unwrap();
        assert_eq!(d.get("a/b").unwrap().unwrap(), b"one");
        assert!(d.put("../x", b"").is_err());
        assert!(d.put_if("head", b"1", None).unwrap());
        assert!(!d.put_if("head", b"2", None).unwrap());
        assert!(d.put_if("head", b"2", Some(Hash::of(b"1"))).unwrap());
        assert_eq!(d.list("").unwrap(), vec!["a/b".to_string(), "head".into()]);
        assert_eq!(d.get_range("a/b", 1, 3).unwrap(), b"ne");
        d.delete("a/b").unwrap();
        assert_eq!(d.get("a/b").unwrap(), None);
        let _ = fs::remove_dir_all(&tmp);
    }
}
