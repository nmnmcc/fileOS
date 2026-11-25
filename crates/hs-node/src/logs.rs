// SPDX-License-Identifier: AGPL-3.0-only

//! A space's log on disk.
//!
//! New entries are appended to `tail`, each prefixed by its length. Every
//! [`SEGMENT_ENTRIES`] entries the tail is sealed into an immutable segment
//! named by its hash, listed in `segments`. Sealed segments never change, so
//! replicas and followers copy them once. A torn write at the end of the
//! tail (a crash mid-append) is cut off on load.

use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use hs_core::cose::Sign1;
use hs_core::event::{Entry, Proposal, decode_segment, encode_segment};
use hs_core::{Hash, PublicKey, cbor};
use hs_identity::Keypair;
use hs_space::{Applied, Payloads, Space};

use crate::{Committed, Error, ErrorKind, Result, WriteId};

/// Entries per sealed segment.
pub const SEGMENT_ENTRIES: usize = 256;

/// A space, its log and where its primary lives.
pub struct SpaceLog {
    /// The current state.
    pub space: Space,
    /// Every entry, oldest first.
    pub entries: Vec<Vec<u8>>,
    /// Sealed segments, oldest first.
    pub segments: Vec<Hash>,
    /// Where the primary can be reached, if it is not this node.
    pub home: Option<String>,
    /// Every payload hash referenced by the log.
    pub payloads: HashSet<Hash>,
    dir: PathBuf,
    append_failed: bool,
    pub(crate) pending: VecDeque<Pending>,
}

impl std::fmt::Debug for SpaceLog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpaceLog")
            .field("seq", &self.space.seq)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
pub(crate) struct Pending {
    pub(crate) applied: Applied,
    pub(crate) ordered_here: Option<bool>,
    pub(crate) indexed: bool,
}

fn payload_of(entry: &[u8]) -> Option<Hash> {
    let e = Entry::decode(&Sign1::decode(entry).ok()?.payload).ok()?;
    Proposal::decode(&Sign1::decode(&e.proposal).ok()?.payload)
        .ok()?
        .payload
}

fn bad_entry(error: impl std::error::Error + Send + Sync + 'static) -> Error {
    Error::io(format!("invalid committed entry: {error}")).with_source(error)
}

fn key_conflict() -> Error {
    Error::new(
        ErrorKind::Conflict,
        "idempotency key already used for a different request".into(),
    )
}

/// Read length-prefixed entries, stopping at a torn end.
fn read_tail(bytes: &[u8]) -> (Vec<Vec<u8>>, usize) {
    let mut out = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let Ok((len, used)) = cbor::Value::decode_prefix(&bytes[pos..]) else {
            break;
        };
        let Some(len) = len.as_u64().and_then(|n| usize::try_from(n).ok()) else {
            break;
        };
        if bytes.len() - pos - used < len {
            break;
        }
        out.push(bytes[pos + used..pos + used + len].to_vec());
        pos += used + len;
    }
    (out, pos)
}

impl SpaceLog {
    /// Start an empty log directory for a space.
    pub(crate) fn create(dir: &Path, space: Space, home: Option<String>) -> Result<SpaceLog> {
        fs::create_dir_all(dir.join("segments"))?;
        if let Some(h) = &home {
            fs::write(dir.join("home"), h)?;
        }
        Ok(SpaceLog {
            space,
            entries: Vec::new(),
            segments: Vec::new(),
            home,
            payloads: HashSet::new(),
            dir: dir.to_path_buf(),
            append_failed: false,
            pending: VecDeque::new(),
        })
    }

    /// Load a log and replay it.
    pub(crate) fn load(
        dir: &Path,
        id: Hash,
        payloads: &dyn Payloads,
    ) -> Result<(SpaceLog, Vec<Applied>)> {
        let mut entries = Vec::new();
        let mut segments = Vec::new();
        if let Some(list) = read_optional(|| fs::read_to_string(dir.join("segments.txt")))? {
            for line in list.lines().filter(|l| !l.is_empty()) {
                let h = Hash::from_hex(line)
                    .ok_or_else(|| Error::io(format!("{}: bad segment list", dir.display())))?;
                let bytes = fs::read(dir.join("segments").join(line))?;
                if Hash::of(&bytes) != h {
                    return Err(Error::io(format!("segment {line} is damaged")));
                }
                entries.extend(decode_segment(&bytes).map_err(|e| Error::io(e.to_string()))?);
                segments.push(h);
            }
        }
        let tail = read_optional(|| fs::read(dir.join("tail")))?.unwrap_or_default();
        let (tail_entries, good) = read_tail(&tail);
        if good < tail.len() {
            // Cut off a torn append.
            let f = fs::OpenOptions::new().write(true).open(dir.join("tail"))?;
            f.set_len(good as u64)?;
        }
        // After a crash between sealing and truncating, the tail repeats the
        // last segment; skip what is already there.
        let already = entries.len();
        for e in tail_entries {
            let seq = Sign1::decode(&e)
                .ok()
                .and_then(|m| Entry::decode(&m.payload).ok())
                .and_then(|x| usize::try_from(x.seq).ok());
            if seq.is_some_and(|s| s < already) {
                continue;
            }
            entries.push(e);
        }
        let (space, applied) = Space::replay(id, entries.iter().map(Vec::as_slice), payloads)?;
        let home =
            read_optional(|| fs::read_to_string(dir.join("home")))?.map(|s| s.trim().to_string());
        let payloads = entries.iter().filter_map(|e| payload_of(e)).collect();
        Ok((
            SpaceLog {
                space,
                entries,
                segments,
                home,
                payloads,
                dir: dir.to_path_buf(),
                append_failed: false,
                pending: VecDeque::new(),
            },
            applied,
        ))
    }

    pub(crate) fn append(&mut self, entry: &[u8]) -> Result<()> {
        append(&self.dir, &mut self.append_failed, entry)?;
        self.record(entry, payload_of(entry));
        Ok(())
    }

    pub(crate) fn apply(&mut self, entry: &[u8], payloads: &dyn Payloads) -> Result<()> {
        let Self {
            space,
            dir,
            append_failed,
            ..
        } = self;
        let applied =
            space.apply_with(entry, payloads, |bytes| append(dir, append_failed, bytes))?;
        self.record(entry, applied.proposal.payload);
        self.enqueue(applied, Some(false));
        Ok(())
    }

    pub(crate) fn order(
        &mut self,
        key: &Keypair,
        proposal: &[u8],
        payloads: &dyn Payloads,
        time: u64,
        check: impl FnOnce(Option<Hash>) -> Result<()>,
    ) -> Result<Committed> {
        let Self {
            space,
            dir,
            append_failed,
            ..
        } = self;
        let mut created = false;
        let check = |current: Option<Hash>| {
            created = current.is_none();
            check(current)
        };
        let ordered = space.order_checked(key, proposal, payloads, time, check, |bytes| {
            append(dir, append_failed, bytes)
        })?;
        match ordered {
            hs_space::Ordered::Duplicate(seq) => self.committed(seq, proposal),
            hs_space::Ordered::New(entry, applied) => {
                let out = Committed {
                    seq: applied.seq,
                    entry: applied.hash,
                    payload: applied.proposal.payload,
                    created: Some(created),
                };
                self.record(&entry, applied.proposal.payload);
                self.enqueue(applied, Some(true));
                Ok(out)
            }
        }
    }

    fn recorded(&self, seq: u64) -> Result<(Committed, Proposal, Vec<u8>)> {
        let bytes = usize::try_from(seq)
            .ok()
            .and_then(|n| self.entries.get(n))
            .ok_or_else(|| Error::io(format!("committed sequence {seq} missing")))?;
        let entry =
            Entry::decode(&Sign1::decode(bytes).map_err(bad_entry)?.payload).map_err(bad_entry)?;
        let signed = Sign1::decode(&entry.proposal).map_err(bad_entry)?;
        let proposal = Proposal::decode(&signed.payload).map_err(bad_entry)?;
        let committed = Committed {
            seq,
            entry: Hash::of(bytes),
            payload: proposal.payload,
            created: Some(false),
        };
        Ok((committed, proposal, signed.kid))
    }

    pub(crate) fn committed(&self, seq: u64, signed: &[u8]) -> Result<Committed> {
        let (result, recorded, kid) = self.recorded(seq)?;
        let candidate = Sign1::decode(signed).map_err(bad_entry)?;
        let candidate_proposal = Proposal::decode(&candidate.payload).map_err(bad_entry)?;
        let same_request = match (recorded.request, candidate_proposal.request) {
            (Some(a), Some(b)) => a == b,
            _ => {
                recorded.payload == candidate_proposal.payload
                    && recorded.media_type == candidate_proposal.media_type
            }
        };
        if kid == candidate.kid
            && recorded.nonce == candidate_proposal.nonce
            && recorded.kind == candidate_proposal.kind
            && recorded.target == candidate_proposal.target
            && recorded.grant == candidate_proposal.grant
            && same_request
        {
            Ok(result)
        } else {
            Err(key_conflict())
        }
    }

    pub(crate) fn replay(&self, author: PublicKey, write: &WriteId) -> Result<Option<Committed>> {
        let Some(seq) = self.space.sequence_for(author, &write.nonce) else {
            return Ok(None);
        };
        let (result, proposal, _) = self.recorded(seq)?;
        match proposal.request {
            Some(request) if request == write.request => Ok(Some(result)),
            Some(_) => Err(key_conflict()),
            None => Ok(None),
        }
    }

    pub(crate) fn enqueue(&mut self, applied: Applied, ordered_here: Option<bool>) {
        self.pending.push_back(Pending {
            applied,
            ordered_here,
            indexed: false,
        });
    }

    pub(crate) fn relocate(&mut self, dir: PathBuf) {
        self.dir = dir;
    }

    fn record(&mut self, entry: &[u8], payload: Option<Hash>) {
        self.entries.push(entry.to_vec());
        if let Some(payload) = payload {
            self.payloads.insert(payload);
        }
    }

    pub(crate) fn seal(&mut self) -> Result<()> {
        let sealed = self.segments.len() * SEGMENT_ENTRIES;
        let count = (self.entries.len() - sealed) / SEGMENT_ENTRIES;
        if count == 0 {
            return Ok(());
        }
        let objects = hs_objects::Dir::open(&self.dir)?;
        let mut segments = self.segments.clone();
        let end = sealed + count * SEGMENT_ENTRIES;
        for entries in self.entries[sealed..end].as_chunks::<SEGMENT_ENTRIES>().0 {
            let segment = encode_segment(entries);
            let hash = Hash::of(&segment);
            objects.put(&format!("segments/{hash}"), &segment)?;
            segments.push(hash);
        }
        sync_directory(&self.dir.join("segments"))?;
        let list: String = segments.iter().map(|hash| format!("{hash}\n")).collect();
        objects.put("segments.txt", list.as_bytes())?;
        sync_directory(&self.dir)?;
        self.segments = segments;
        // Once the segment list is durable, a repeated prefix in the tail is safe.
        let mut rest = Vec::new();
        for e in &self.entries[end..] {
            rest.extend(cbor::Value::Uint(e.len() as u64).encode());
            rest.extend_from_slice(e);
        }
        if let Err(error) = objects.put("tail", &rest) {
            tracing::warn!(name: "space.log.compaction_failed", %error,
                "Could not trim the sealed prefix from the space log");
        }
        Ok(())
    }

    pub(crate) fn has_unsealed_segment(&self) -> bool {
        self.entries.len() / SEGMENT_ENTRIES > self.segments.len()
    }

    /// A sealed segment's bytes.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if a segment listed in the log cannot be read.
    pub fn segment(&self, hash: &Hash) -> Result<Option<Vec<u8>>> {
        if !self.segments.contains(hash) {
            return Ok(None);
        }
        Ok(Some(fs::read(
            self.dir.join("segments").join(hash.to_hex()),
        )?))
    }

    /// The signed head, if this node is the primary.
    pub fn signed_head(&self, key: &Keypair) -> Option<Vec<u8>> {
        (self.space.primary == key.public()).then(|| self.space.head(key, self.segments.clone()))
    }

    /// Remember where the primary is.
    pub(crate) fn set_home(&mut self, home: &str) -> Result<()> {
        hs_objects::Dir::open(&self.dir)?.put("home", home.as_bytes())?;
        self.home = Some(home.to_string());
        Ok(())
    }
}

// The manifest must be committed before an old tail can be replaced.
pub(crate) fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn read_optional<T>(read: impl FnOnce() -> std::io::Result<T>) -> Result<Option<T>> {
    match read() {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn append(dir: &Path, append_failed: &mut bool, entry: &[u8]) -> Result<()> {
    if *append_failed {
        return Err(Error::io(
            "space log must be reopened after a failed write rollback".into(),
        ));
    }
    let mut framed = cbor::Value::Uint(entry.len() as u64).encode();
    framed.extend_from_slice(entry);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("tail"))?;
    let previous_len = file.metadata()?.len();
    if let Err(error) = file.write_all(&framed).and_then(|()| file.sync_data()) {
        if let Err(rollback) = file.set_len(previous_len).and_then(|()| file.sync_data()) {
            *append_failed = true;
            tracing::error!(name: "space.log.rollback_failed", %rollback,
                "Could not roll back the incomplete space log append");
        }
        return Err(error.into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn torn_tail() {
        let mut bytes = Vec::new();
        for e in [b"one".as_slice(), b"two"] {
            bytes.extend(cbor::Value::Uint(e.len() as u64).encode());
            bytes.extend_from_slice(e);
        }
        let full = bytes.len();
        bytes.extend([0x05, b'x']);
        let (entries, good) = read_tail(&bytes);
        assert_eq!(entries, vec![b"one".to_vec(), b"two".to_vec()]);
        assert_eq!(good, full);
    }
}
