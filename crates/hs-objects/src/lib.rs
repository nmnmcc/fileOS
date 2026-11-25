// SPDX-License-Identifier: AGPL-3.0-only

//! Immutable objects: blobs, sealed segments and checkpoints, and where they
//! are kept.
//!
//! Everything here is either content-addressed and never changes, or (log
//! heads) changed only by conditional write. That is what lets the same
//! objects live in a local directory, on another `hs` or in an S3 bucket, and
//! makes sync, backup, migration and federation one operation: copy what the
//! other side lacks ([`plan_sync`]).

pub mod crypt;
mod dir;
pub mod s3;

#[doc(inline)]
pub use dir::{Dir, blob_name};

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// Which objects to copy so that `to` has everything `from` has.
/// Both lists hold object names; heads are handled separately.
pub fn plan_sync<'a>(from: &'a [String], to: &[String]) -> Vec<&'a String> {
    let have: std::collections::HashSet<&String> = to.iter().collect();
    from.iter().filter(|n| !have.contains(n)).collect()
}

/// Parse a `Range: bytes=a-b` header against a length. Returns the inclusive
/// start and exclusive end, or `None` if the range is not satisfiable.
pub fn parse_range(header: &str, len: u64) -> Option<(u64, u64)> {
    let spec = header.strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().ok()?;
        (len.saturating_sub(n), len)
    } else {
        let start: u64 = a.parse().ok()?;
        let end = if b.is_empty() {
            len
        } else {
            b.parse::<u64>().ok()?.saturating_add(1).min(len)
        };
        (start, end)
    };
    (start < end && start < len).then_some((start, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-9", 100), Some((0, 10)));
        assert_eq!(parse_range("bytes=90-", 100), Some((90, 100)));
        assert_eq!(parse_range("bytes=-5", 100), Some((95, 100)));
        assert_eq!(parse_range("bytes=95-200", 100), Some((95, 100)));
        assert_eq!(parse_range("bytes=100-", 100), None);
        assert_eq!(parse_range("bytes=0-1,4-5", 100), None);
    }

    #[test]
    fn sync_plan() {
        let a = vec!["x".to_string(), "y".into(), "z".into()];
        let b = vec!["y".to_string()];
        assert_eq!(plan_sync(&a, &b), vec!["x", "z"]);
    }
}
