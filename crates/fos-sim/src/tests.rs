// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

#[test]
fn a_shared_folder_syncs_both_ways() {
    let mut sim = Sim::default();
    let (alice, bob) = (sim.add(1), sim.add(2));
    sim.all_store();
    sim.connect(alice, bob);

    let family = sim.create(alice, "family");
    sim.follow(bob, family);
    sim.put(alice, family, "/photos/a.jpg", b"photo a").unwrap();
    assert_eq!(
        sim.read(bob, family, "/photos/a.jpg").as_deref(),
        Some(&b"photo a"[..])
    );

    // Bob is not the ordering node: his write goes to Alice, and comes back
    // in the main line.
    sim.put(bob, family, "/notes/b.txt", b"from bob").unwrap();
    assert_eq!(
        sim.read(alice, family, "/notes/b.txt").as_deref(),
        Some(&b"from bob"[..])
    );
    assert_eq!(
        sim.folder(alice, family).unwrap().head(),
        sim.folder(bob, family).unwrap().head()
    );
    assert_eq!(sim.nodes[bob].replica.pending().len(), 0);
}

#[test]
fn writes_wait_for_the_ordering_node() {
    let mut sim = Sim::default();
    let (alice, bob) = (sim.add(1), sim.add(2));
    sim.all_store();
    sim.connect(alice, bob);
    let family = sim.create(alice, "family");
    sim.follow(bob, family);

    sim.disconnect(alice, bob);
    sim.put(bob, family, "/offline.txt", b"later").unwrap();
    sim.put(alice, family, "/while-apart.txt", b"x").unwrap();
    assert_eq!(sim.nodes[bob].replica.pending().len(), 1);
    assert!(sim.read(alice, family, "/offline.txt").is_none());

    sim.connect(alice, bob);
    assert_eq!(sim.nodes[bob].replica.pending().len(), 0);
    for node in [alice, bob] {
        let f = sim.folder(node, family).unwrap();
        assert_eq!(f.list("/"), ["offline.txt", "while-apart.txt"]);
    }
    assert_eq!(
        sim.read(bob, family, "/while-apart.txt").as_deref(),
        Some(&b"x"[..])
    );
}

#[test]
fn a_node_that_may_not_keep_a_folder_gets_nothing() {
    let mut sim = Sim::default();
    let (alice, eve) = (sim.add(1), sim.add(5));
    // Only Alice may keep folders, by Alice's engine.
    let only_alice = vec![sim.key(alice)];
    sim.nodes[alice].engine.storers = only_alice;
    sim.connect(alice, eve);
    let family = sim.create(alice, "family");
    sim.put(alice, family, "/secret.txt", b"s").unwrap();
    sim.follow(eve, family);
    assert!(sim.folder(eve, family).is_none());
}

#[test]
fn a_node_with_different_rules_forks() {
    let mut sim = Sim::default();
    let (alice, bob, carol, dave) = (sim.add(1), sim.add(2), sim.add(3), sim.add(4));
    sim.all_store();
    // Carol's node does not take Dave's writes. The others do.
    let dave_key = sim.key(dave);
    sim.nodes[carol].engine.deny = vec![dave_key];
    for node in [bob, carol, dave] {
        sim.connect(alice, node);
    }
    let family = sim.create(alice, "family");
    for node in [bob, carol, dave] {
        sim.follow(node, family);
    }

    sim.put(bob, family, "/a.txt", b"a").unwrap();
    sim.put(dave, family, "/photo.jpg", b"from dave").unwrap();
    sim.put(bob, family, "/b.txt", b"b").unwrap();

    // Alice, Bob and Dave follow the main line.
    for node in [alice, bob, dave] {
        assert_eq!(
            sim.folder(node, family).unwrap().list("/"),
            ["a.txt", "b.txt", "photo.jpg"]
        );
    }
    // Carol's copy stopped at Dave's entry. It has a fork, and she can see why.
    let carol_copy = sim.folder(carol, family).unwrap();
    assert_eq!(carol_copy.list("/"), ["a.txt"]);
    assert_eq!(carol_copy.fork().map(|f| f.seq), Some(2));
    assert!(
        sim.nodes[carol]
            .events
            .iter()
            .any(|e| matches!(e, Event::Forked { .. }))
    );
}
