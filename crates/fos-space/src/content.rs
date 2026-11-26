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

use fos_core::{Hash, decode, encode};

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
    let mut pieces = Vec::new();
    let mut list: Vec<(Hash, u64)> = Vec::new();
    let mut rest = data;
    loop {
        let n = cut(rest);
        let (chunk, tail) = rest.split_at(n);
        let hash = Hash::of(chunk);
        list.push((hash, chunk.len() as u64));
        pieces.push((hash, chunk.to_vec()));
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

/// The hashes of the chunks of `content`, if its list is here.
pub fn chunks(content: &Content, store: &dyn Contents) -> Option<Vec<Hash>> {
    if !content.chunks {
        return Some(vec![content.hash]);
    }
    let list: Vec<(Hash, u64)> = decode(&store.get(content.hash)?).ok()?;
    Some(list.into_iter().map(|(h, _)| h).collect())
}

/// The bytes of `content`, if all of it is here and good.
pub fn join(content: &Content, store: &dyn Contents) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(usize::try_from(content.size).ok()?);
    for hash in chunks(content, store)? {
        let chunk = store.get(hash)?;
        if Hash::of(&chunk) != hash {
            return None;
        }
        out.extend_from_slice(&chunk);
    }
    (out.len() as u64 == content.size).then_some(out)
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
