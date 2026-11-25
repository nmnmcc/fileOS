// SPDX-License-Identifier: AGPL-3.0-only

//! Many seeds through the simulation.

use super::*;

fn faulty(nodes: usize) -> Config {
    Config {
        nodes,
        drop: 0.15,
        duplicate: 0.1,
        max_delay: 6,
        propose: 0.04,
        ..Config::default()
    }
}

/// Run every seed, check it, and return the stats added together.
fn seeds(range: std::ops::Range<u64>, cfg: &Config, steps: u64) -> Stats {
    let mut t = Stats::default();
    for seed in range {
        let mut sim = Sim::with_config(seed, cfg.clone());
        sim.run(steps);
        if let Err(e) = sim.check() {
            panic!("seed {seed}: {e}");
        }
        let s = sim.stats();
        assert!(s.proposals > 0, "seed {seed}: no proposals");
        t.proposals += s.proposals;
        t.ordered += s.ordered;
        t.duplicates += s.duplicates;
        t.dropped += s.dropped;
        t.doubled += s.doubled;
        t.gaps += s.gaps;
        t.crashes += s.crashes;
        t.handovers += s.handovers;
    }
    println!("{t:?}");
    // The faults really took place.
    assert!(t.duplicates > 0 && t.dropped > 0 && t.doubled > 0 && t.gaps > 0);
    t
}

#[test]
fn deterministic() {
    let run = |seed| {
        let mut sim = Sim::with_config(
            seed,
            Config {
                crash: 0.02,
                handover_at: Some(100),
                ..faulty(4)
            },
        );
        sim.run(300);
        sim.check().unwrap();
        (sim.head(), sim.stats())
    };
    assert_eq!(run(7), run(7));
    assert_ne!(run(7).0, run(8).0);
}

#[test]
fn lossy_network() {
    seeds(0..80, &faulty(4), 300);
}

#[test]
fn crashes() {
    let cfg = Config {
        crash: 0.03,
        ..faulty(3)
    };
    assert!(seeds(1000..1060, &cfg, 300).crashes > 0);
}

#[test]
fn handover() {
    let cfg = Config {
        handover_at: Some(120),
        crash: 0.01,
        ..faulty(4)
    };
    assert_eq!(seeds(2000..2060, &cfg, 300).handovers, 60);
}

#[test]
fn single_node() {
    let mut sim = Sim::new(1, 1);
    sim.run(50);
    sim.check().unwrap();
    assert_eq!(sim.head().map(|h| h.0), Some(0));
}

/// A member change that was refused (one that would leave no owner) must not
/// change anything in memory.
#[test]
fn refused_member_change_leaves_state_unchanged() {
    let owner = Keypair::from_seed([1; 32]);
    let me = PublicKey([9; 32]);
    let (g, gp) = genesis_proposal(&owner, me, serde_json::json!({"name": "s"}), 0, vec![0]);
    let (mut space, _, _) = Space::genesis(&owner, &g, &gp, 0).unwrap();
    let demote = serde_json::json!({"node": owner.public().to_id(), "identity": me.to_id(), "role": "writer"})
        .to_string()
        .into_bytes();
    let blobs = |h: &Hash| (*h == Hash::of(&demote)).then(|| demote.clone());
    let p = propose(
        &owner,
        space.id,
        Kind::Member,
        "member/me",
        Some(&demote),
        "application/json",
        1,
        vec![1],
        None,
    );
    let before = space.members.clone();
    space.order(&owner, &p, &blobs, 1).unwrap_err();
    assert_eq!(space.members, before);
}
