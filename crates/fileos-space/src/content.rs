// SPDX-License-Identifier: AGPL-3.0-only

//! Content in chunks, cut at points that the content picks.
//!
//! Bytes are cut with a gear hash, as in `FastCDC` (Xia and others, 2016), with
//! normalized chunking: chunks are 16 KiB to 256 KiB, about 64 KiB on
//! average. A change in the middle of a file changes only the chunks around
//! it, so a new version keeps the other chunks, and sync sends only what is
//! missing.
//!
//! Each chunk is named by its BLAKE3 hash. Content of one chunk is named by
//! the hash of its bytes. Content of more chunks is named by the hash of a
//! **list** (the CBOR form of the hashes and sizes of its chunks), which is
//! kept in the content store too.

use fileos_core::{Hash, decode, encode};

use crate::crypto::{self, Key};
use crate::entry::Content;
use crate::sync::Contents;

const MIN: usize = 16 * 1024;
const AVERAGE: usize = 64 * 1024;
const MAX: usize = 256 * 1024;
/// More bits before the average size make a cut less likely there, and fewer
/// after make it more likely: so most chunks are near the average.
const MASK_SMALL: u64 = mask(18);
const MASK_LARGE: u64 = mask(14);

const fn mask(bits: u32) -> u64 {
    ((1 << bits) - 1) << (64 - bits)
}

/// 256 random numbers, made the same way every time (splitmix64).
const GEAR: [u64; 256] = {
    let mut table = [0; 256];
    let mut state: u64 = 0x6669_6c65_4f53_2026;
    let mut i = 0;
    while i < 256 {
        state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        table[i] = z ^ (z >> 31);
        i += 1;
    }
    table
};

/// The length of the first chunk of `data`.
fn cut(data: &[u8]) -> usize {
    if data.len() <= MIN {
        return data.len();
    }
    let end = data.len().min(MAX);
    let middle = end.min(AVERAGE);
    let mut h: u64 = 0;
    for (i, b) in data.iter().enumerate().take(end).skip(MIN) {
        h = (h << 1).wrapping_add(GEAR[*b as usize]);
        let mask = if i < middle { MASK_SMALL } else { MASK_LARGE };
        if h & mask == 0 {
            return i + 1;
        }
    }
    end
}

/// Cuts `data` into chunks. Gives back the content, and every piece to keep
/// in the content store by its hash: the chunks, and the list if there is
/// more than one chunk.
pub fn split(data: &[u8]) -> (Content, Vec<(Hash, Vec<u8>)>) {
    split_with(data, <[u8]>::to_vec)
}

/// Cuts `data` into chunks and encrypts each one with `data_key`.
///
/// The nonce of a chunk comes from the key and the chunk, so the same chunk
/// in a new version of the file gives the same bytes and is kept once. The
/// pieces are named by the hash of what is kept: the nonce and the
/// ciphertext.
///
/// # Panics
///
/// Never: a BLAKE3 hash has 32 bytes, so it has 24.
pub fn seal(data: &[u8], data_key: &Key) -> (Content, Vec<(Hash, Vec<u8>)>) {
    split_with(data, |chunk| {
        let nonce: [u8; 24] = blake3::keyed_hash(data_key, chunk).as_bytes()[..24]
            .try_into()
            .expect("24 bytes");
        let mut piece = nonce.to_vec();
        piece.extend(crypto::seal(data_key, &nonce, chunk, b""));
        piece
    })
}

/// The bytes of content made by [`seal`], if all of it is here and the key is right.
pub fn open(content: &Content, data_key: &Key, store: &dyn Contents) -> Option<Vec<u8>> {
    Reader::sealed(*content, vec![*data_key])?.join(store)
}

fn split_with(data: &[u8], piece: impl Fn(&[u8]) -> Vec<u8>) -> (Content, Vec<(Hash, Vec<u8>)>) {
    let mut pieces = Vec::new();
    let mut list: Vec<(Hash, u64)> = Vec::new();
    let mut rest = data;
    loop {
        let n = cut(rest);
        let (chunk, tail) = rest.split_at(n);
        let bytes = piece(chunk);
        let hash = Hash::of(&bytes);
        list.push((hash, bytes.len() as u64));
        pieces.push((hash, bytes));
        rest = tail;
        if rest.is_empty() {
            break;
        }
    }
    let size = data.len() as u64;
    if list.len() == 1 {
        return (
            Content {
                hash: list[0].0,
                size,
                chunks: false,
            },
            pieces,
        );
    }
    let bytes = encode(&list);
    let hash = Hash::of(&bytes);
    pieces.push((hash, bytes));
    (
        Content {
            hash,
            size,
            chunks: true,
        },
        pieces,
    )
}

/// The bytes added to each encrypted chunk by its XChaCha20 nonce and tag.
const SEALED_OVERHEAD: u64 = 24 + 16;

/// Checks a chunk list and gives the hashes and sizes that it names.
pub(crate) fn chunk_list(content: &Content, bytes: &[u8]) -> Option<Vec<(Hash, u64)>> {
    if Hash::of(bytes) != content.hash {
        return None;
    }
    let list: Vec<(Hash, u64)> = decode(bytes).ok()?;
    (list.len() >= 2
        && (list_size(&list, 0) == Some(content.size)
            || list_size(&list, SEALED_OVERHEAD) == Some(content.size)))
    .then_some(list)
}

fn list_size(list: &[(Hash, u64)], overhead: u64) -> Option<u64> {
    list.iter().try_fold(0_u64, |total, &(_, size)| {
        let size = size.checked_sub(overhead)?;
        if size == 0 || size > MAX as u64 {
            return None;
        }
        total.checked_add(size)
    })
}

/// The hashes of the chunks of `content`, if its list is here.
pub fn chunks(content: &Content, store: &dyn Contents) -> Option<Vec<Hash>> {
    if !content.chunks {
        return (content.size <= MAX as u64).then(|| vec![content.hash]);
    }
    Some(
        chunk_list(content, &store.get(content.hash)?)?
            .into_iter()
            .map(|(hash, _)| hash)
            .collect(),
    )
}

/// A read of one file version, with the keys that can open it.
pub struct Reader {
    content: Content,
    keys: Option<Vec<Key>>,
}

impl std::fmt::Debug for Reader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Reader")
            .field("content", &self.content)
            .finish_non_exhaustive()
    }
}

impl Reader {
    pub(crate) fn plain(content: Content) -> Self {
        Self {
            content,
            keys: None,
        }
    }

    pub(crate) fn sealed(content: Content, keys: Vec<Key>) -> Option<Self> {
        (!keys.is_empty()).then_some(Self {
            content,
            keys: Some(keys),
        })
    }

    /// The stored content and its size before encryption.
    pub fn content(&self) -> Content {
        self.content
    }

    /// Passes verified chunks to `write` in order.
    ///
    /// A missing or invalid chunk, or a stopped write, gives `None`.
    /// Earlier chunks may have already reached `write`.
    pub fn read(
        &self,
        store: &dyn Contents,
        mut write: impl FnMut(Vec<u8>) -> Option<()>,
    ) -> Option<()> {
        let overhead = if self.keys.is_some() {
            SEALED_OVERHEAD
        } else {
            0
        };
        let content = self.content;
        let one;
        let list;
        let parts = if content.chunks {
            list = chunk_list(&content, &store.get(content.hash)?)?;
            if list_size(&list, overhead) != Some(content.size) {
                return None;
            }
            list.as_slice()
        } else {
            if content.size > MAX as u64 {
                return None;
            }
            one = [(content.hash, content.size + overhead)];
            &one
        };
        let mut key = None;
        for &(hash, size) in parts {
            let piece = store.get(hash)?;
            if piece.len() as u64 != size || Hash::of(&piece) != hash {
                return None;
            }
            let chunk = match &self.keys {
                None => piece,
                Some(keys) => {
                    let (nonce, sealed) = piece.split_at_checked(24)?;
                    let nonce = nonce.try_into().ok()?;
                    if let Some(key) = key {
                        crypto::open(key, nonce, sealed, b"")?
                    } else {
                        let (found, bytes) = keys.iter().find_map(|candidate| {
                            let bytes = crypto::open(candidate, nonce, sealed, b"")?;
                            Some((candidate, bytes))
                        })?;
                        key = Some(found);
                        bytes
                    }
                }
            };
            write(chunk)?;
        }
        Some(())
    }

    /// Collects all of the verified chunks into one buffer.
    pub fn join(&self, store: &dyn Contents) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        self.read(store, |chunk| {
            if out.is_empty() {
                out = chunk;
            } else {
                out.try_reserve(chunk.len()).ok()?;
                out.extend_from_slice(&chunk);
            }
            Some(())
        })?;
        Some(out)
    }
}

/// Passes verified unencrypted chunks to `write` in order.
pub fn read(
    content: &Content,
    store: &dyn Contents,
    write: impl FnMut(Vec<u8>) -> Option<()>,
) -> Option<()> {
    Reader::plain(*content).read(store, write)
}

/// The bytes of unencrypted content, if all of it is here and good.
pub fn join(content: &Content, store: &dyn Contents) -> Option<Vec<u8>> {
    Reader::plain(*content).join(store)
}

/// Says if every piece of `content` is here.
pub fn complete(content: &Content, store: &dyn Contents) -> bool {
    chunks(content, store).is_some_and(|hashes| hashes.iter().all(|h| store.has(*h)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    struct Mem(BTreeMap<Hash, Vec<u8>>);

    impl Contents for Mem {
        fn has(&self, hash: Hash) -> bool {
            self.0.contains_key(&hash)
        }
        fn get(&self, hash: Hash) -> Option<Vec<u8>> {
            self.0.get(&hash).cloned()
        }
    }

    fn noise(n: usize, seed: u64) -> Vec<u8> {
        let mut x = seed;
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                x as u8
            })
            .collect()
    }

    #[test]
    fn sealed_content_opens_only_with_its_key_and_keeps_chunks() {
        let data = noise(1 << 20, 9);
        let (key, other) = ([1; 32], [2; 32]);
        let (c, pieces) = seal(&data, &key);
        let mut store = Mem(pieces.into_iter().collect());
        assert_eq!(open(&c, &key, &store).unwrap(), data);
        assert_eq!(open(&c, &other, &store), None);

        let reader = Reader::sealed(c, vec![other, key]).unwrap();
        let mut count = 0;
        let mut offset = 0;
        reader
            .read(&store, |chunk| {
                let end = offset + chunk.len();
                assert_eq!(chunk, data[offset..end]);
                offset = end;
                count += 1;
                Some(())
            })
            .unwrap();
        assert!(count > 1);
        assert_eq!(offset, data.len());
        assert!(
            store
                .0
                .values()
                .all(|p| !p.windows(64).any(|w| data.windows(64).next() == Some(w)))
        );
        let mut edited = data.clone();
        edited[1 << 19] ^= 1;
        let before = store.0.len();
        let (c2, pieces) = seal(&edited, &key);
        store.0.extend(pieces);
        assert!(store.0.len() - before <= 3);
        assert_eq!(open(&c2, &key, &store).unwrap(), edited);
    }

    #[test]
    fn small_content_is_one_chunk() {
        let (c, pieces) = split(b"hello");
        assert!(!c.chunks);
        assert_eq!(c, Content::of(b"hello"));
        assert_eq!(pieces.len(), 1);
    }

    #[test]
    fn a_change_in_the_middle_keeps_most_chunks() {
        let a = noise(4 << 20, 1);
        let mut b = a.clone();
        b[2 << 20] ^= 0xff;
        let (ca, pa) = split(&a);
        let (cb, pb) = split(&b);
        assert!(ca.chunks);
        assert_ne!(ca.hash, cb.hash);
        let sizes: Vec<usize> = pa.iter().map(|p| p.1.len()).collect();
        assert!(sizes.iter().all(|n| *n <= MAX));
        let mut store = Mem(pa.into_iter().collect());
        assert_eq!(join(&ca, &store).unwrap(), a);
        let before = store.0.len();
        store.0.extend(pb);
        // Only the changed chunk and the new list are new.
        assert!(
            store.0.len() - before <= 3,
            "{} new pieces",
            store.0.len() - before
        );
        assert_eq!(join(&cb, &store).unwrap(), b);
    }
}
