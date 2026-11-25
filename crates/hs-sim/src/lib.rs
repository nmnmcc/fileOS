// SPDX-License-Identifier: AGPL-3.0-only

//! Deterministic simulation of many nodes, for tests.
//!
//! [`Sim`] runs one space replicated across several nodes over a simulated
//! network that delays, drops, duplicates and reorders messages, and can
//! crash nodes and hand the space over to another primary. Everything is
//! driven by one seeded PRNG, so a seed always gives the same run: when a
//! seed fails, rerun it to debug.
//!
//! The protocol simulated:
//! - members send signed proposals to the primary they know of, retrying
//!   with the same nonce until acknowledged;
//! - the primary orders them ([`Space::order`]), appends the entry to its log,
//!   acknowledges and pushes the entry to every member;
//! - members apply entries in order and, on a gap (or now and then), pull the
//!   missing ones (`after = seq`) from the primary they know of. Any node
//!   answers pulls from its log.

use std::collections::BTreeMap;

use hs_core::cose::Sign1;
use hs_core::event::{Entry, Kind};
use hs_core::{Hash, PublicKey};
use hs_identity::Keypair;
use hs_space::{Ordered, Space, genesis_proposal, propose};

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// `SplitMix64`: a small, fast, seeded PRNG. Not for cryptography.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    /// A generator for this seed.
    pub fn new(seed: u64) -> Rng {
        Rng(seed)
    }

    /// The next 64 random bits.
    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..n` (0 if `n` is 0).
    pub fn below(&mut self, n: u64) -> u64 {
        if n == 0 { 0 } else { self.next_u64() % n }
    }

    /// True with probability `p`.
    pub fn chance(&mut self, p: f64) -> bool {
        // 53 random bits as a float in [0, 1).
        ((self.next_u64() >> 11) as f64 / (1u64 << 53) as f64) < p
    }

    /// 32 random bytes.
    pub fn bytes32(&mut self) -> [u8; 32] {
        let mut out = [0; 32];
        for c in out.as_chunks_mut::<8>().0 {
            c.copy_from_slice(&self.next_u64().to_le_bytes());
        }
        out
    }
}

/// Fault rates and pacing. Probabilities are per message or per step.
#[derive(Clone, Debug)]
pub struct Config {
    /// Number of nodes (at least 1; 2 or more for a handover).
    pub nodes: usize,
    /// Probability that a message is lost.
    pub drop: f64,
    /// Probability that a message is delivered twice.
    pub duplicate: f64,
    /// Messages take 1 to `max_delay` steps, so later ones can overtake.
    pub max_delay: u64,
    /// Probability per step that each member (other than the primary) makes
    /// a new proposal.
    pub propose: f64,
    /// Probability per step that some node crashes and restarts from its log.
    pub crash: f64,
    /// Step at which node 0 hands the space over to node 1, if any.
    pub handover_at: Option<u64>,
    /// Steps before an unacknowledged proposal is sent again.
    pub retry_after: u64,
    /// Every node pulls from its primary this often, to recover lost pushes.
    pub pull_every: u64,
}

impl Default for Config {
    fn default() -> Config {
        Config {
            nodes: 3,
            drop: 0.1,
            duplicate: 0.05,
            max_delay: 5,
            propose: 0.05,
            crash: 0.0,
            handover_at: None,
            retry_after: 8,
            pull_every: 10,
        }
    }
}

/// Counts of what happened during a run, to see that faults were exercised.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Proposals made by members.
    pub proposals: u64,
    /// Entries ordered.
    pub ordered: u64,
    /// Retries the primary recognised as duplicates.
    pub duplicates: u64,
    /// Messages lost.
    pub dropped: u64,
    /// Messages delivered twice.
    pub doubled: u64,
    /// Entries that arrived after a gap and caused a pull.
    pub gaps: u64,
    /// Node crashes.
    pub crashes: u64,
    /// Handovers ordered.
    pub handovers: u64,
}

#[derive(Clone)]
enum Msg {
    /// A signed proposal; `id` indexes [`Sim::sent`].
    Propose {
        from: usize,
        id: usize,
        signed: Vec<u8>,
    },
    /// The primary ordered proposal `id` at `seq`.
    Ack { id: usize, seq: u64 },
    /// New entries (a push, or the answer to a pull).
    Entries(Vec<Vec<u8>>),
    /// Send me the entries after `after` (all of them if `None`).
    Pull { from: usize, after: Option<u64> },
}

/// A proposal a member made, and what came of it.
#[derive(Clone, Debug)]
struct Sent {
    node: usize,
    nonce: Vec<u8>,
    signed: Vec<u8>,
    /// Sequence number from the first acknowledgement.
    acked: Option<u64>,
    last_sent: u64,
}

struct Node {
    key: Keypair,
    /// In-memory state; rebuilt from `log` after a crash.
    space: Option<Space>,
    /// Persisted: signed entries, in order.
    log: Vec<Vec<u8>>,
    /// Persisted: proposals not yet acknowledged (indexes into `sent`).
    outbox: Vec<usize>,
    nonces: u64,
    down_until: u64,
}

/// A simulated network of nodes sharing one space.
pub struct Sim {
    cfg: Config,
    rng: Rng,
    step: u64,
    id: Hash,
    nodes: Vec<Node>,
    index: BTreeMap<PublicKey, usize>,
    /// Payloads by hash, shared by every node (blob transfer is not simulated).
    blobs: BTreeMap<Hash, Vec<u8>>,
    /// In flight: (delivery step, send order) -> (recipient, message).
    net: BTreeMap<(u64, u64), (usize, Msg)>,
    sends: u64,
    sent: Vec<Sent>,
    stats: Stats,
    /// Things that must never happen; [`Sim::check`] reports them.
    errors: Vec<String>,
}

impl std::fmt::Debug for Sim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Sim").finish_non_exhaustive()
    }
}

fn entry_seq(signed: &[u8]) -> Option<u64> {
    let msg = Sign1::decode(signed).ok()?;
    Some(Entry::decode(&msg.payload).ok()?.seq)
}

impl Sim {
    /// A simulation of `nodes` nodes with the default fault rates.
    pub fn new(seed: u64, nodes: usize) -> Sim {
        Sim::with_config(
            seed,
            Config {
                nodes,
                ..Config::default()
            },
        )
    }

    /// A simulation with these settings. Node 0 creates the space and adds
    /// every other node as a writer; the others start empty and catch up by
    /// pulling.
    pub fn with_config(seed: u64, cfg: Config) -> Sim {
        let mut rng = Rng::new(seed);
        let count = cfg.nodes.max(1);
        let nodes: Vec<Node> = (0..count)
            .map(|_| Node {
                key: Keypair::from_seed(rng.bytes32()),
                space: None,
                log: Vec::new(),
                outbox: Vec::new(),
                nonces: 0,
                down_until: 0,
            })
            .collect();
        let index = nodes
            .iter()
            .enumerate()
            .map(|(i, n)| (n.key.public(), i))
            .collect();
        let mut sim = Sim {
            cfg,
            rng,
            step: 0,
            id: Hash::ZERO,
            nodes,
            index,
            blobs: BTreeMap::new(),
            net: BTreeMap::new(),
            sends: 0,
            sent: Vec::new(),
            stats: Stats::default(),
            errors: Vec::new(),
        };
        let identity = |i: usize| PublicKey([i as u8 + 1; 32]);
        let nonce = sim.nonce(0);
        let (g, gp) = genesis_proposal(
            &sim.nodes[0].key,
            identity(0),
            serde_json::json!({ "name": "sim" }),
            0,
            nonce,
        );
        sim.blobs.insert(Hash::of(&gp), gp.clone());
        let (space, e0, _) = match Space::genesis(&sim.nodes[0].key, &g, &gp, 0) {
            Ok(v) => v,
            Err(e) => {
                sim.errors.push(format!("genesis: {e}"));
                return sim;
            }
        };
        sim.id = space.id;
        sim.nodes[0].space = Some(space);
        sim.nodes[0].log.push(e0);
        for i in 1..count {
            let body = serde_json::json!({
                "node": sim.nodes[i].key.public().to_id(),
                "identity": identity(i).to_id(),
                "role": "writer",
            });
            sim.local(0, Kind::Member, &format!("member/{i}"), &body);
        }
        sim
    }

    fn nonce(&mut self, node: usize) -> Vec<u8> {
        let n = &mut self.nodes[node];
        n.nonces += 1;
        // Node index and counter: unique per author, and the same on retry.
        [(node as u64).to_le_bytes(), n.nonces.to_le_bytes()].concat()
    }

    /// A proposal by `node` that it orders itself (it must be the primary).
    fn local(&mut self, node: usize, kind: Kind, target: &str, body: &serde_json::Value) {
        let payload = body.to_string().into_bytes();
        self.blobs.insert(Hash::of(&payload), payload.clone());
        let nonce = self.nonce(node);
        let signed = propose(
            &self.nodes[node].key,
            self.id,
            kind,
            target,
            Some(&payload),
            "application/json",
            self.step,
            nonce,
            None,
        );
        self.order(node, &signed);
    }

    /// As `node`, order a proposal; returns its sequence number.
    fn order(&mut self, node: usize, signed: &[u8]) -> Option<u64> {
        let Sim {
            nodes, blobs, step, ..
        } = self;
        let lookup = |h: &Hash| blobs.get(h).cloned();
        let n = &mut nodes[node];
        let space = n.space.as_mut()?;
        match space.order(&n.key, signed, &lookup, *step) {
            Ok(Ordered::New(entry, applied)) => {
                n.log.push(entry.clone());
                self.stats.ordered += 1;
                if applied.proposal.kind == Kind::Handover {
                    self.stats.handovers += 1;
                }
                for to in 0..self.nodes.len() {
                    if to != node {
                        self.send(to, Msg::Entries(vec![entry.clone()]));
                    }
                }
                Some(applied.seq)
            }
            Ok(Ordered::Duplicate(seq)) => {
                self.stats.duplicates += 1;
                Some(seq)
            }
            Err(e) => {
                // A member that has not yet learnt of a handover asks the old
                // primary; that is expected. Anything else is a bug.
                if e.0 != "not the primary of this space" {
                    self.errors.push(format!("step {}: order: {e}", self.step));
                }
                None
            }
        }
    }

    fn send(&mut self, to: usize, msg: Msg) {
        if self.rng.chance(self.cfg.drop) {
            self.stats.dropped += 1;
            return;
        }
        let copies = if self.rng.chance(self.cfg.duplicate) {
            self.stats.doubled += 1;
            2
        } else {
            1
        };
        for _ in 0..copies {
            let at = self.step + 1 + self.rng.below(self.cfg.max_delay.max(1));
            self.sends += 1;
            self.net.insert((at, self.sends), (to, msg.clone()));
        }
    }

    /// The node `node` believes is the primary.
    fn primary_of(&self, node: usize) -> usize {
        self.nodes[node]
            .space
            .as_ref()
            .and_then(|s| self.index.get(&s.primary).copied())
            .unwrap_or(0)
    }

    fn pull(&mut self, node: usize) {
        let to = self.primary_of(node);
        if to != node {
            let after = self.nodes[node].space.as_ref().map(|s| s.seq);
            self.send(to, Msg::Pull { from: node, after });
        }
    }

    /// Apply entries in order; on a gap, pull what is missing.
    fn receive(&mut self, node: usize, entries: Vec<Vec<u8>>) {
        for e in entries {
            let Some(seq) = entry_seq(&e) else {
                self.errors.push("undecodable entry".into());
                return;
            };
            let Sim { nodes, blobs, .. } = self;
            let lookup = |h: &Hash| blobs.get(h).cloned();
            let n = &mut nodes[node];
            let result = match &mut n.space {
                None if seq == 0 => {
                    Space::replay(self.id, [e.as_slice()], &lookup).map(|(s, _)| {
                        n.space = Some(s);
                    })
                }
                Some(s) if seq <= s.seq => continue,
                Some(s) if seq == s.seq + 1 => s.apply(&e, &lookup).map(|_| ()),
                _ => {
                    self.stats.gaps += 1;
                    self.pull(node);
                    return;
                }
            };
            match result {
                Ok(()) => n.log.push(e),
                Err(err) => {
                    self.errors.push(format!(
                        "step {}: node {node} apply {seq}: {err}",
                        self.step
                    ));
                    return;
                }
            }
        }
    }

    fn deliver(&mut self, to: usize, msg: Msg) {
        if self.nodes[to].down_until > self.step {
            return;
        }
        match msg {
            Msg::Propose { from, id, signed } => {
                if let Some(seq) = self.order(to, &signed) {
                    self.send(from, Msg::Ack { id, seq });
                }
            }
            Msg::Ack { id, seq } => {
                let s = &mut self.sent[id];
                match s.acked {
                    Some(prev) if prev != seq => self
                        .errors
                        .push(format!("proposal {id} acknowledged at {prev} and at {seq}")),
                    _ => s.acked = Some(seq),
                }
                self.nodes[to].outbox.retain(|&p| p != id);
            }
            Msg::Entries(entries) => self.receive(to, entries),
            Msg::Pull { from, after } => {
                let start = after.map_or(0, |a| a.saturating_add(1));
                let log = &self.nodes[to].log;
                let start = usize::try_from(start).unwrap_or(usize::MAX).min(log.len());
                let end = (start + 64).min(log.len());
                if start < end {
                    let entries = log[start..end].to_vec();
                    self.send(from, Msg::Entries(entries));
                }
            }
        }
    }

    /// Crash a node: it loses its memory and, after a while, rebuilds its
    /// state from its persisted log.
    fn crash(&mut self, node: usize) {
        let Sim { nodes, blobs, .. } = self;
        let lookup = |h: &Hash| blobs.get(h).cloned();
        let n = &mut nodes[node];
        self.stats.crashes += 1;
        n.space = None;
        if !n.log.is_empty() {
            match Space::replay(self.id, n.log.iter().map(Vec::as_slice), &lookup) {
                Ok((s, _)) => n.space = Some(s),
                Err(e) => self.errors.push(format!("node {node} replay: {e}")),
            }
        }
        n.down_until = self.step + 1 + self.rng.below(20);
    }

    fn tick(&mut self, active: bool) {
        self.step += 1;
        while let Some(entry) = self.net.first_entry() {
            if entry.key().0 > self.step {
                break;
            }
            let (to, msg) = entry.remove();
            self.deliver(to, msg);
        }
        if active
            && self.cfg.handover_at == Some(self.step)
            && self.nodes.len() > 1
            && self.primary_of(0) == 0
        {
            let body = serde_json::json!({ "primary": self.nodes[1].key.public().to_id() });
            self.local(0, Kind::Handover, "space/primary", &body);
        }
        if active && self.cfg.crash > 0.0 && self.rng.chance(self.cfg.crash) {
            let node = self.rng.below(self.nodes.len() as u64) as usize;
            self.crash(node);
        }
        for node in 0..self.nodes.len() {
            if self.nodes[node].down_until > self.step {
                continue;
            }
            let primary = self.primary_of(node);
            if active && primary != node && self.rng.chance(self.cfg.propose) {
                self.new_proposal(node);
            }
            for id in self.nodes[node].outbox.clone() {
                if self.sent[id].last_sent + self.cfg.retry_after <= self.step {
                    self.sent[id].last_sent = self.step;
                    let signed = self.sent[id].signed.clone();
                    if primary == node {
                        // It became the primary itself: order directly.
                        if let Some(seq) = self.order(node, &signed) {
                            self.sent[id].acked.get_or_insert(seq);
                            self.nodes[node].outbox.retain(|&p| p != id);
                        }
                    } else {
                        self.send(
                            primary,
                            Msg::Propose {
                                from: node,
                                id,
                                signed,
                            },
                        );
                    }
                }
            }
            let every = self.cfg.pull_every.max(1);
            if (self.step + node as u64).is_multiple_of(every) {
                self.pull(node);
            }
        }
    }

    fn new_proposal(&mut self, node: usize) {
        let k = self.sent.len();
        let payload = serde_json::json!({ "node": node, "k": k })
            .to_string()
            .into_bytes();
        self.blobs.insert(Hash::of(&payload), payload.clone());
        // A few shared targets, so writes overwrite each other.
        let target = format!("note/{}", self.rng.below(8));
        let nonce = self.nonce(node);
        let signed = propose(
            &self.nodes[node].key,
            self.id,
            Kind::Put,
            &target,
            Some(&payload),
            "application/json",
            self.step,
            nonce.clone(),
            None,
        );
        self.sent.push(Sent {
            node,
            nonce,
            signed,
            acked: None,
            last_sent: 0,
        });
        self.nodes[node].outbox.push(k);
        self.stats.proposals += 1;
    }

    /// Run `steps` steps with proposals, faults and the handover enabled.
    pub fn run(&mut self, steps: u64) {
        for _ in 0..steps {
            self.tick(true);
        }
    }

    /// Keep delivering, retrying and pulling (with faults, but no new
    /// proposals or crashes) until every proposal is acknowledged and every
    /// replica has the same head. Returns false if that takes more than
    /// `max_steps`.
    pub fn settle(&mut self, max_steps: u64) -> bool {
        for _ in 0..max_steps {
            if self.quiet() {
                return true;
            }
            self.tick(false);
        }
        self.quiet()
    }

    fn quiet(&self) -> bool {
        let tip = self.head();
        self.nodes.iter().all(|n| {
            n.outbox.is_empty()
                && n.down_until <= self.step
                && n.space.as_ref().map(|s| (s.seq, s.head)) == tip
        })
    }

    /// The latest `(seq, head)` of any replica.
    pub fn head(&self) -> Option<(u64, Hash)> {
        self.nodes
            .iter()
            .filter_map(|n| n.space.as_ref().map(|s| (s.seq, s.head)))
            .max_by_key(|(seq, _)| *seq)
    }

    /// What happened so far.
    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Settle, then check the invariants: every replica has the same `seq`,
    /// `head`, members and resources; every log replays to its replica; every
    /// acknowledged proposal is in the log exactly once, at the acknowledged
    /// sequence number; nothing was refused that should not have been.
    ///
    /// # Errors
    ///
    /// Returns an error if replicas disagree or an event log fails to reconstruct its replica.
    pub fn check(&mut self) -> Result<(), String> {
        if !self.settle(50_000) {
            return Err(format!("did not settle by step {}", self.step));
        }
        if let Some(e) = self.errors.first() {
            return Err(e.clone());
        }
        let lookup = |h: &Hash| self.blobs.get(h).cloned();
        let first = self.nodes[0].space.as_ref().ok_or("node 0 has no space")?;
        let mut applied = Vec::new();
        for (i, n) in self.nodes.iter().enumerate() {
            let s = n.space.as_ref().ok_or(format!("node {i} has no space"))?;
            if (s.seq, s.head) != (first.seq, first.head) {
                return Err(format!("node {i} is at {} not {}", s.seq, first.seq));
            }
            if s.resources != first.resources || s.members != first.members {
                return Err(format!("node {i} state differs"));
            }
            if s.primary != first.primary {
                return Err(format!("node {i} has another primary"));
            }
            let (replayed, a) = Space::replay(self.id, n.log.iter().map(Vec::as_slice), &lookup)
                .map_err(|e| format!("node {i} log: {e}"))?;
            if replayed.head != s.head || replayed.resources != s.resources {
                return Err(format!("node {i} log does not match its state"));
            }
            applied = a;
        }
        let mut seen: BTreeMap<(PublicKey, &[u8]), Vec<u64>> = BTreeMap::new();
        for a in &applied {
            seen.entry((a.author, &a.proposal.nonce))
                .or_default()
                .push(a.seq);
        }
        for (id, s) in self.sent.iter().enumerate() {
            let Some(acked) = s.acked else {
                return Err(format!("proposal {id} was never acknowledged"));
            };
            let key = (self.nodes[s.node].key.public(), s.nonce.as_slice());
            match seen.get(&key).map(Vec::as_slice) {
                Some([seq]) if *seq == acked => {}
                other => {
                    return Err(format!(
                        "proposal {id} acknowledged at {acked} but in the log at {other:?}"
                    ));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
