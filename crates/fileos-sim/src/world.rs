// SPDX-License-Identifier: AGPL-3.0-only

//! A deterministic simulation of nodes of the new kernel ([`fileos_space`]).
//!
//! Each node is a [`Node`] with its content in memory. Messages go through
//! one queue, in order, and only between nodes that are connected. A message
//! to a node that is not connected is lost, as on a real network.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use fileos_core::{Error, Hash, PublicKey, SecretKey};
use fileos_space::crypto::Wrapped;
use fileos_space::{
    Act, Content, Contents, Effect, Event, File, Id, Msg, Node, Permission, Principal, Resource,
    Retention, Space, Step,
};

/// Content in memory.
#[derive(Debug, Default)]
pub struct MemContents(pub BTreeMap<Hash, Vec<u8>>);

impl Contents for MemContents {
    fn has(&self, hash: Hash) -> bool {
        self.0.contains_key(&hash)
    }

    fn get(&self, hash: Hash) -> Option<Vec<u8>> {
        self.0.get(&hash).cloned()
    }
}

/// One simulated node.
#[derive(Debug)]
pub struct WorldNode {
    pub key: SecretKey,
    pub node: Node,
    pub contents: MemContents,
    pub events: Vec<Event>,
}

/// Many nodes and the links between them.
#[derive(Debug, Default)]
pub struct World {
    pub nodes: Vec<WorldNode>,
    /// The time now, in milliseconds since 1970. Every act moves it on by a
    /// second; [`World::wait`] moves it on more.
    pub time: u64,
    links: BTreeSet<(usize, usize)>,
    queue: VecDeque<(usize, PublicKey, Msg)>,
}

impl World {
    /// Adds a node. Its key comes from `n`, so runs are the same every time.
    pub fn add(&mut self, n: u8) -> usize {
        let key = SecretKey::from_seed(&[n; 32]);
        let mut node = Node::new(key.clone(), u64::from(n) << 32);
        let _ = node.rotate_recipient(self.time);
        self.nodes.push(WorldNode {
            node,
            key,
            contents: MemContents::default(),
            events: Vec::new(),
        });
        self.nodes.len() - 1
    }

    /// Makes a node change its recipient key, and destroy all but the one
    /// before it.
    pub fn rotate(&mut self, node: usize) {
        let time = self.tick();
        let step = self.nodes[node].node.rotate_recipient(time);
        self.apply(node, step);
        self.run();
    }

    /// Lets `ms` milliseconds go by.
    pub fn wait(&mut self, ms: u64) {
        self.time += ms;
    }

    fn tick(&mut self) -> u64 {
        self.time += 1000;
        self.time
    }

    pub fn key(&self, node: usize) -> PublicKey {
        self.nodes[node].key.public()
    }

    pub fn who(&self, node: usize) -> Principal {
        Principal::Key(self.key(node))
    }

    fn index(&self, key: PublicKey) -> usize {
        self.nodes
            .iter()
            .position(|n| n.key.public() == key)
            .expect("a known node")
    }

    fn linked(&self, a: usize, b: usize) -> bool {
        self.links.contains(&(a.min(b), a.max(b)))
    }

    pub fn connect(&mut self, a: usize, b: usize) {
        self.links.insert((a.min(b), a.max(b)));
        let (ka, kb) = (self.key(a), self.key(b));
        let step = self.nodes[a].node.connected(kb);
        self.apply(a, step);
        let step = self.nodes[b].node.connected(ka);
        self.apply(b, step);
        self.run();
    }

    pub fn disconnect(&mut self, a: usize, b: usize) {
        self.links.remove(&(a.min(b), a.max(b)));
        let (ka, kb) = (self.key(a), self.key(b));
        self.nodes[a].node.disconnected(kb);
        self.nodes[b].node.disconnected(ka);
    }

    /// Makes a node a node of an identity: `ops` is the DID log, and `cert`
    /// the node certificate.
    ///
    /// # Errors
    ///
    /// When the certificate is not good for the node.
    pub fn set_identity(
        &mut self,
        node: usize,
        ops: Vec<fileos_core::Bytes>,
        cert: fileos_core::Bytes,
    ) -> Result<(), Error> {
        let step = self.nodes[node].node.set_identity(ops, cert)?;
        self.apply(node, step);
        self.run();
        Ok(())
    }

    /// Gives a node revocations of nodes of an identity, as its identity
    /// space would.
    pub fn revoke(&mut self, node: usize, revocations: Vec<fileos_core::Bytes>) {
        let step = self.nodes[node].node.add_revocations(revocations);
        self.apply(node, step);
        self.run();
    }

    /// Gives a principal (a key, a group or an identity) permissions.
    ///
    /// # Errors
    ///
    /// When the node may not give them.
    pub fn give_to(
        &mut self,
        node: usize,
        space: Hash,
        to: Principal,
        permissions: &[Permission],
        on: Resource,
    ) -> Result<Id, Error> {
        self.write(
            node,
            space,
            vec![Act::Give {
                to,
                effect: Effect::Allow,
                permissions: permissions.to_vec(),
                on,
            }],
        )
    }

    /// Makes a space owned by the node's key.
    pub fn create(&mut self, node: usize) -> Hash {
        let time = self.tick();
        let (id, step) = self.nodes[node].node.create(time);
        self.apply(node, step);
        self.run();
        id
    }

    pub fn follow(&mut self, node: usize, space: Hash) {
        let step = self.nodes[node].node.follow(space);
        self.apply(node, step);
        self.run();
    }

    /// Writes acts signed by the node's key.
    ///
    /// # Errors
    ///
    /// When the node may not do them.
    pub fn write(&mut self, node: usize, space: Hash, acts: Vec<Act>) -> Result<Id, Error> {
        let time = self.tick();
        let (entry, step) = self.nodes[node].node.write(space, time, acts)?;
        self.apply(node, step);
        self.run();
        Ok(Id {
            entry: entry.hash(),
            index: 0,
        })
    }

    /// Makes a file. Gives back its ID.
    ///
    /// # Errors
    ///
    /// When the node may not make it.
    pub fn put(&mut self, node: usize, space: Hash, name: &str, bytes: &[u8]) -> Result<Id, Error> {
        let (content, key) = self.keep(node, space, name, None, bytes)?;
        self.write(
            node,
            space,
            vec![Act::Create {
                name: name.into(),
                content,
                key,
            }],
        )
    }

    /// Changes a file: a new version after the newest ones here.
    ///
    /// # Errors
    ///
    /// When the file is not here or the node may not change it.
    pub fn change(
        &mut self,
        node: usize,
        space: Hash,
        file: Id,
        name: &str,
        bytes: Option<&[u8]>,
    ) -> Result<Id, Error> {
        let parents = self
            .space(node, space)
            .and_then(|s| s.file(file))
            .ok_or_else(|| Error::invalid("no such file".into()))?
            .versions
            .iter()
            .map(|v| v.id)
            .collect();
        let (content, key) = match bytes {
            Some(b) => {
                let (c, k) = self.keep(node, space, name, Some(file), b)?;
                (Some(c), k)
            }
            None => (None, None),
        };
        self.write(
            node,
            space,
            vec![Act::Change {
                file,
                parents,
                name: name.into(),
                content,
                key,
            }],
        )
    }

    /// Gives `to` the permissions on `on`. Gives back the ID of the grant.
    ///
    /// # Errors
    ///
    /// When the node may not give them.
    pub fn give(
        &mut self,
        node: usize,
        space: Hash,
        to: usize,
        permissions: &[Permission],
        on: Resource,
    ) -> Result<Id, Error> {
        let to = self.who(to);
        self.write(
            node,
            space,
            vec![Act::Give {
                to,
                effect: Effect::Allow,
                permissions: permissions.to_vec(),
                on,
            }],
        )
    }

    /// Takes a grant back.
    ///
    /// # Errors
    ///
    /// When the node may not take it back.
    pub fn take(&mut self, node: usize, space: Hash, grant: Id) -> Result<Id, Error> {
        self.write(node, space, vec![Act::Take { grant }])
    }

    /// Makes a checkpoint on a node, with the default retention.
    ///
    /// # Errors
    ///
    /// When the node may not make checkpoints.
    pub fn checkpoint(&mut self, node: usize, space: Hash) -> Result<(), Error> {
        let time = self.tick();
        let step = self.nodes[node]
            .node
            .checkpoint(space, time, Retention::default())?;
        self.apply(node, step);
        self.run();
        Ok(())
    }

    /// Compacts a space on a node. Gives back false when nothing was dropped.
    pub fn compact(&mut self, node: usize, space: Hash) -> bool {
        let (done, step) = self.nodes[node].node.compact(space);
        self.apply(node, step);
        self.run();
        done
    }

    /// Every event of every node, in order.
    pub fn events(&self, node: usize) -> &[Event] {
        &self.nodes[node].events
    }

    /// Makes the content of a file on a node, encrypted as the node does it,
    /// and keeps its pieces.
    fn keep(
        &mut self,
        node: usize,
        space: Hash,
        name: &str,
        file: Option<Id>,
        bytes: &[u8],
    ) -> Result<(Content, Option<Wrapped>), Error> {
        let n = &mut self.nodes[node];
        let (content, key, pieces) = n.node.seal(space, name, file, bytes)?;
        n.contents.0.extend(pieces);
        Ok((content, key))
    }

    pub fn space(&self, node: usize, id: Hash) -> Option<&Space> {
        self.nodes[node].node.space(id)
    }

    /// The files with `name` on a node.
    pub fn find(&self, node: usize, space: Hash, name: &str) -> Vec<File> {
        self.space(node, space)
            .map(|s| s.find(name))
            .unwrap_or_default()
    }

    /// The content of every newest version of the file with `name`, as this
    /// node has it.
    pub fn read(&mut self, node: usize, space: Hash, name: &str) -> Vec<Vec<u8>> {
        let files = self.find(node, space, name);
        let n = &mut self.nodes[node];
        files
            .into_iter()
            .flat_map(|f| f.versions.into_iter().map(move |v| (f.id, v)))
            .filter_map(|(id, v)| n.node.open(space, id, &v, &n.contents))
            .collect()
    }

    fn apply(&mut self, node: usize, step: Step) {
        for (to, msg) in step.out {
            self.queue.push_back((node, to, msg));
        }
        for event in step.events {
            if let Event::Content { from, data } = &event {
                let n = &mut self.nodes[node];
                n.contents.0.insert(Hash::of(data), data.clone());
                let more = n.node.content_saved(*from, &n.contents);
                self.queue
                    .extend(more.out.into_iter().map(|(to, msg)| (node, to, msg)));
            }
            self.nodes[node].events.push(event);
        }
    }

    /// Delivers messages until there are none.
    pub fn run(&mut self) {
        while let Some((from, to, msg)) = self.queue.pop_front() {
            let to = self.index(to);
            if !self.linked(from, to) {
                continue;
            }
            let key = self.key(from);
            let n = &mut self.nodes[to];
            let step = n.node.receive(key, msg, &n.contents);
            self.apply(to, step);
        }
    }
}

#[cfg(test)]
mod tests;
