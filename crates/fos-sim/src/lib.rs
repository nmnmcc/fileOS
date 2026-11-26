// SPDX-License-Identifier: AGPL-3.0-only

//! A deterministic simulation of many fileOS nodes.
//!
//! Each node is a [`Replica`] with a native engine and blobs in memory.
//! Messages go through one queue, in order, and only between nodes that are
//! connected. Nothing here uses threads, clocks or the network, so a test
//! gives the same result every time.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use fos_core::{Error, Hash, PublicKey, SecretKey};
use fos_fs::{Folder, Op, Open};
use fos_sync::{Blobs, Event, Msg, Replica, Step};

pub mod world;

/// Blobs in memory.
#[derive(Debug, Default)]
pub struct MemBlobs(pub BTreeMap<Hash, Vec<u8>>);

impl Blobs for MemBlobs {
    fn has(&self, hash: Hash) -> bool {
        self.0.contains_key(&hash)
    }

    fn get(&self, hash: Hash) -> Option<Vec<u8>> {
        self.0.get(&hash).cloned()
    }
}

/// One simulated node.
#[derive(Debug)]
pub struct SimNode {
    pub key: SecretKey,
    pub replica: Replica,
    pub engine: Open,
    pub blobs: MemBlobs,
    /// Every event, in order.
    pub events: Vec<Event>,
}

/// Many nodes and the links between them.
#[derive(Debug, Default)]
pub struct Sim {
    pub nodes: Vec<SimNode>,
    links: BTreeSet<(usize, usize)>,
    queue: VecDeque<(usize, PublicKey, Msg)>,
}

impl Sim {
    /// Adds a node. Its key comes from `n`, so runs are the same every time.
    pub fn add(&mut self, n: u8) -> usize {
        let key = SecretKey::from_seed(&[n; 32]);
        let replica = Replica::new(key.clone(), u64::from(n) << 32);
        self.nodes.push(SimNode {
            key,
            replica,
            engine: Open::default(),
            blobs: MemBlobs::default(),
            events: Vec::new(),
        });
        self.nodes.len() - 1
    }

    pub fn key(&self, node: usize) -> PublicKey {
        self.nodes[node].key.public()
    }

    /// Every node may keep every folder.
    pub fn all_store(&mut self) {
        let keys: Vec<PublicKey> = self.nodes.iter().map(|n| n.key.public()).collect();
        for node in &mut self.nodes {
            node.engine.storers.clone_from(&keys);
        }
    }

    fn index(&self, key: PublicKey) -> usize {
        self.nodes
            .iter()
            .position(|n| n.key.public() == key)
            .expect("a known node")
    }

    pub fn connect(&mut self, a: usize, b: usize) {
        self.links.insert((a.min(b), a.max(b)));
        let (ka, kb) = (self.key(a), self.key(b));
        let node = &mut self.nodes[a];
        let step = node.replica.connected(kb, &mut node.engine);
        self.apply(a, step);
        let node = &mut self.nodes[b];
        let step = node.replica.connected(ka, &mut node.engine);
        self.apply(b, step);
        self.run();
    }

    pub fn disconnect(&mut self, a: usize, b: usize) {
        self.links.remove(&(a.min(b), a.max(b)));
        let (ka, kb) = (self.key(a), self.key(b));
        self.nodes[a].replica.disconnected(kb);
        self.nodes[b].replica.disconnected(ka);
    }

    pub fn create(&mut self, node: usize, name: &str) -> Hash {
        let n = &mut self.nodes[node];
        let (id, step) = n.replica.create(name, &mut n.engine);
        self.apply(node, step);
        self.run();
        id
    }

    pub fn follow(&mut self, node: usize, folder: Hash) {
        let step = self.nodes[node].replica.follow(folder);
        self.apply(node, step);
        self.run();
    }

    /// Puts a file, written by the node itself.
    ///
    /// # Errors
    ///
    /// When the node's engine does not take the write.
    pub fn put(&mut self, node: usize, folder: Hash, path: &str, data: &[u8]) -> Result<(), Error> {
        let n = &mut self.nodes[node];
        let hash = Hash::of(data);
        n.blobs.0.insert(hash, data.to_vec());
        let op = Op::Put {
            path: path.into(),
            blob: hash,
            size: data.len() as u64,
        };
        let key = n.key.clone();
        let step = n.replica.write(folder, op, &key, &mut n.engine)?;
        self.apply(node, step);
        self.run();
        Ok(())
    }

    pub fn folder(&self, node: usize, id: Hash) -> Option<&Folder> {
        self.nodes[node].replica.folder(id)
    }

    /// Says if a node has the bytes of a file.
    pub fn read(&self, node: usize, folder: Hash, path: &str) -> Option<Vec<u8>> {
        let (hash, _) = self.folder(node, folder)?.file(path)?;
        self.nodes[node].blobs.get(hash)
    }

    fn apply(&mut self, node: usize, step: Step) {
        for out in step.out {
            self.queue.push_back((node, out.to, out.msg));
        }
        for event in step.events {
            if let Event::Blob(data) = &event {
                self.nodes[node]
                    .blobs
                    .0
                    .insert(Hash::of(data), data.clone());
            }
            self.nodes[node].events.push(event);
        }
    }

    /// Delivers messages until there are none.
    pub fn run(&mut self) {
        while let Some((from, to, msg)) = self.queue.pop_front() {
            let to = self.index(to);
            if !self.links.contains(&(from.min(to), from.max(to))) {
                continue; // the link went down; the message is lost
            }
            let from_key = self.key(from);
            let n = &mut self.nodes[to];
            let step = n.replica.handle(from_key, msg, &mut n.engine, &n.blobs);
            self.apply(to, step);
        }
    }
}

#[cfg(test)]
mod tests;
