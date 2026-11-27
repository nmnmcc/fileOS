// SPDX-License-Identifier: AGPL-3.0-only

//! Keeps the ancestors of an entry as runs of positions in causal order.

use std::ops::Range;

/// The common case is one run from the start, with no allocation. Parts
/// after a gap are kept as sorted ranges that do not touch.
#[derive(Clone, Debug, Default)]
pub(super) struct Past {
    prefix: usize,
    other: Vec<Range<usize>>,
}

impl Past {
    pub(super) fn has(&self, position: usize) -> bool {
        position < self.prefix
            || self
                .other
                .get(self.other.partition_point(|r| r.end <= position))
                .is_some_and(|run| run.start <= position)
    }

    pub(super) fn add_parent(&mut self, position: usize, past: &Self) {
        if past.prefix != 0 {
            self.add(0..past.prefix);
        }
        for run in &past.other {
            self.add(run.clone());
        }
        self.add(position..position + 1);
    }

    fn add(&mut self, run: Range<usize>) {
        if run.start <= self.prefix {
            self.prefix = self.prefix.max(run.end);
            let count = self.other.partition_point(|r| r.start <= self.prefix);
            if let Some(last) = self.other[..count].last() {
                self.prefix = self.prefix.max(last.end);
                self.other.drain(..count);
            }
            return;
        }
        let first = self.other.partition_point(|r| r.end < run.start);
        let end = self.other.partition_point(|r| r.start <= run.end);
        if first == end {
            self.other.insert(first, run);
        } else {
            let start = run.start.min(self.other[first].start);
            let stop = run.end.max(self.other[end - 1].end);
            self.other[first] = start..stop;
            self.other.drain(first + 1..end);
        }
    }
}
