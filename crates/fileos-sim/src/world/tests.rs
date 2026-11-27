// SPDX-License-Identifier: AGPL-3.0-only

use fileos_space::Permission::{
    AccessGive, AccessTake, DataKeep, FilesChange, FilesCreate, FilesDelete, FilesRead,
};

use super::*;

const WRITE: [Permission; 5] = [DataKeep, FilesRead, FilesCreate, FilesChange, FilesDelete];

fn all() -> Resource {
    Resource::Prefix(String::new())
}

/// Ming's laptop (the owner) and Mom's phone, which can write everywhere.
fn family() -> (World, usize, usize, Hash) {
    let mut world = World::default();
    let (ming, mom) = (world.add(1), world.add(2));
    let space = world.create(ming);
    world.give(ming, space, mom, &WRITE, all()).unwrap();
    world.connect(ming, mom);
    world.follow(mom, space);
    (world, ming, mom, space)
}

#[test]
fn a_node_reads_its_own_write_while_offline() {
    let (mut world, ming, mom, space) = family();
    world.disconnect(ming, mom);

    // Mom is not the owner, and no other node is online. She can still write,
    // and she reads what she wrote at once.
    world.put(mom, space, "notes/todo.txt", b"milk").unwrap();
    assert_eq!(world.read(mom, space, "notes/todo.txt"), [b"milk".to_vec()]);
    assert_eq!(
        world.read(ming, space, "notes/todo.txt"),
        Vec::<Vec<u8>>::new()
    );

    world.connect(ming, mom);
    assert_eq!(
        world.read(ming, space, "notes/todo.txt"),
        [b"milk".to_vec()]
    );
}

#[test]
fn every_node_comes_to_the_same_state() {
    let mut world = World::default();
    let (ming, mom, dad) = (world.add(1), world.add(2), world.add(3));
    let space = world.create(ming);
    world.give(ming, space, mom, &WRITE, all()).unwrap();
    world.give(ming, space, dad, &WRITE, all()).unwrap();
    // Only a line: Mom - Ming - Dad. Entries are sent on.
    world.connect(ming, mom);
    world.connect(ming, dad);
    world.follow(mom, space);
    world.follow(dad, space);

    world.put(mom, space, "a", b"from mom").unwrap();
    world.put(dad, space, "b", b"from dad").unwrap();
    assert_eq!(world.read(dad, space, "a"), [b"from mom".to_vec()]);
    assert_eq!(world.read(mom, space, "b"), [b"from dad".to_vec()]);
    let digests: BTreeSet<Hash> = [ming, mom, dad]
        .iter()
        .map(|n| world.space(*n, space).unwrap().digest())
        .collect();
    assert_eq!(digests.len(), 1);
}

#[test]
fn changes_made_offline_at_the_same_time_are_both_kept() {
    let (mut world, ming, mom, space) = family();
    let notes = world.put(ming, space, "notes", b"v1").unwrap();
    world.disconnect(ming, mom);
    world
        .change(ming, space, notes, "notes", Some(b"ming"))
        .unwrap();
    world
        .change(mom, space, notes, "notes", Some(b"mom"))
        .unwrap();
    world.connect(ming, mom);

    let mut versions = world.read(ming, space, "notes");
    versions.sort();
    assert_eq!(versions, [b"ming".to_vec(), b"mom".to_vec()]);
    assert_eq!(
        world.space(ming, space).unwrap().digest(),
        world.space(mom, space).unwrap().digest()
    );

    // One change after both settles it, on both nodes.
    world
        .change(mom, space, notes, "notes", Some(b"both"))
        .unwrap();
    assert_eq!(world.read(ming, space, "notes"), [b"both".to_vec()]);
}

#[test]
fn a_space_goes_only_to_nodes_that_may_keep_it() {
    let mut world = World::default();
    let (ming, stranger) = (world.add(1), world.add(9));
    let space = world.create(ming);
    world.put(ming, space, "secret", b"x").unwrap();
    world.connect(ming, stranger);
    world.follow(stranger, space);
    assert!(world.space(stranger, space).is_none());

    // Once it is given access, it gets the space.
    world
        .give(ming, space, stranger, &[DataKeep], all())
        .unwrap();
    assert!(world.space(stranger, space).is_some());
    assert_eq!(world.find(stranger, space, "secret").len(), 1);
}

#[test]
fn a_rename_keeps_a_grant_on_the_file_but_leaves_a_grant_on_the_prefix() {
    let mut world = World::default();
    let (ming, mom, chat) = (world.add(1), world.add(2), world.add(3));
    let space = world.create(ming);
    let cat = world.put(ming, space, "2026/cat.jpg", b"cat").unwrap();
    world
        .give(
            ming,
            space,
            mom,
            &[FilesRead],
            Resource::Prefix("2026/".into()),
        )
        .unwrap();
    world
        .give(ming, space, chat, &[FilesRead], Resource::File(cat))
        .unwrap();
    world
        .change(ming, space, cat, "best/cat.jpg", Some(b"cat"))
        .unwrap();
    let s = world.space(ming, space).unwrap();
    assert!(!s.may_read(world.who(mom), cat));
    assert!(s.may_read(world.who(chat), cat));
}

#[test]
fn strong_removal_undoes_offline_writes_made_while_access_was_taken() {
    let mut world = World::default();
    let (ming, mom) = (world.add(1), world.add(2));
    let space = world.create(ming);
    let grant = world.give(ming, space, mom, &WRITE, all()).unwrap();
    world.connect(ming, mom);
    world.follow(mom, space);
    world.disconnect(ming, mom);

    let write = world.put(mom, space, "from-mom", b"hi").unwrap();
    world.take(ming, space, grant).unwrap();
    // Mom no longer may keep the space, so she cannot get the take-back from
    // Ming; but Ming gets her write, and does not take it.
    world.connect(ming, mom);
    let s = world.space(ming, space).unwrap();
    assert!(s.has(write.entry), "Ming has the write");
    assert!(
        s.rejected().contains_key(&write.entry),
        "but it has no effect"
    );
    assert_eq!(world.find(ming, space, "from-mom"), Vec::new());
    assert!(
        !world
            .space(ming, space)
            .unwrap()
            .may(world.who(mom), FilesRead, &all())
    );
}

#[test]
fn a_manager_gives_on_and_take_back_cascades() {
    let mut world = World::default();
    let (ming, mom, ann) = (world.add(1), world.add(2), world.add(3));
    let space = world.create(ming);
    let to_mom = world
        .give(
            ming,
            space,
            mom,
            &[DataKeep, FilesRead, AccessGive, AccessTake],
            all(),
        )
        .unwrap();
    world.connect(ming, mom);
    world.connect(mom, ann);
    world.follow(mom, space);
    world.follow(ann, space);
    world
        .give(mom, space, ann, &[DataKeep, FilesRead], all())
        .unwrap();
    assert!(world.space(ann, space).is_some());

    world.take(ming, space, to_mom).unwrap();
    let s = world.space(ming, space).unwrap();
    assert!(!s.may(world.who(ann), FilesRead, &all()));
}

#[test]
fn large_files_sync_in_chunks_and_a_new_version_sends_only_what_changed() {
    let (mut world, ming, mom, space) = family();
    let mut x: u64 = 7;
    let big: Vec<u8> = (0..(2 << 20))
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x as u8
        })
        .collect();
    let file = world.put(ming, space, "video.bin", &big).unwrap();
    assert_eq!(
        world.read(mom, space, "video.bin"),
        std::slice::from_ref(&big)
    );

    let before = world.nodes[mom].contents.0.len();
    let mut edited = big.clone();
    edited[1 << 20] ^= 1;
    world
        .change(ming, space, file, "video.bin", Some(&edited))
        .unwrap();
    assert_eq!(world.read(mom, space, "video.bin"), [edited]);
    assert!(world.nodes[mom].contents.0.len() - before <= 3);
}

fn digest(world: &World, node: usize, space: Hash) -> Hash {
    world.space(node, space).unwrap().digest()
}

#[test]
fn compaction_keeps_the_same_state_and_drops_the_entries() {
    let (mut world, ming, mom, space) = family();
    let notes = world.put(ming, space, "notes", b"v1").unwrap();
    for i in 0..20u8 {
        world
            .change(mom, space, notes, "notes", Some(&[i]))
            .unwrap();
    }
    world.put(mom, space, "other", b"x").unwrap();
    let before = digest(&world, ming, space);
    world.checkpoint(ming, space).unwrap();
    assert!(world.compact(ming, space));
    assert!(world.compact(mom, space));
    assert_eq!(digest(&world, ming, space), before);
    assert_eq!(digest(&world, mom, space), before);
    assert_eq!(
        world.space(ming, space).unwrap().len(),
        1,
        "only the checkpoint"
    );

    // Work goes on after it.
    world
        .change(mom, space, notes, "notes", Some(b"after"))
        .unwrap();
    assert_eq!(world.read(ming, space, "notes"), [b"after".to_vec()]);
}

#[test]
fn a_new_node_starts_from_the_snapshot() {
    let (mut world, ming, _mom, space) = family();
    world.put(ming, space, "a", b"a").unwrap();
    world.checkpoint(ming, space).unwrap();
    world.compact(ming, space);
    world.put(ming, space, "b", b"b").unwrap();

    let box_ = world.add(5);
    world
        .give(ming, space, box_, &[DataKeep, FilesRead], all())
        .unwrap();
    world.connect(ming, box_);
    world.follow(box_, space);
    assert_eq!(world.read(box_, space, "a"), [b"a".to_vec()]);
    assert_eq!(world.read(box_, space, "b"), [b"b".to_vec()]);
    assert_eq!(digest(&world, box_, space), digest(&world, ming, space));
    assert!(world.space(box_, space).unwrap().base().is_some());
}

#[test]
fn a_change_made_offline_at_the_same_time_as_a_checkpoint_is_put_on_top_again() {
    let (mut world, ming, mom, space) = family();
    let notes = world.put(ming, space, "notes", b"v1").unwrap();
    world.disconnect(ming, mom);
    world
        .change(mom, space, notes, "notes", Some(b"mom, offline"))
        .unwrap();
    world.put(mom, space, "new", b"made offline").unwrap();
    world.checkpoint(ming, space).unwrap();
    world.compact(ming, space);

    world.connect(ming, mom);
    assert_eq!(world.read(ming, space, "notes"), [b"mom, offline".to_vec()]);
    assert_eq!(world.read(ming, space, "new"), [b"made offline".to_vec()]);
    assert_eq!(digest(&world, ming, space), digest(&world, mom, space));
    assert!(
        world
            .events(mom)
            .iter()
            .any(|e| matches!(e, Event::Again { .. })),
        "Mom is told"
    );
}

#[test]
fn a_node_that_was_away_long_gets_the_snapshot_and_keeps_its_work() {
    let (mut world, ming, mom, space) = family();
    world.disconnect(ming, mom);
    world.put(mom, space, "from-mom", b"offline").unwrap();
    for i in 0..3u8 {
        world.put(ming, space, &format!("f{i}"), &[i]).unwrap();
        world.checkpoint(ming, space).unwrap();
        world.compact(ming, space);
    }
    world.connect(ming, mom);
    assert_eq!(world.read(mom, space, "f2"), [vec![2]]);
    assert_eq!(world.read(ming, space, "from-mom"), [b"offline".to_vec()]);
    assert_eq!(digest(&world, ming, space), digest(&world, mom, space));
}

#[test]
fn only_the_owner_or_a_holder_of_space_checkpoint_makes_checkpoints() {
    let (mut world, ming, mom, space) = family();
    world.checkpoint(mom, space).unwrap_err();
    world
        .give(ming, space, mom, &[Permission::SpaceCheckpoint], all())
        .unwrap();
    world.checkpoint(mom, space).unwrap();
}

#[test]
fn old_versions_get_thinner_with_age() {
    let (mut world, ming, _mom, space) = family();
    let notes = world.put(ming, space, "notes", b"v").unwrap();
    // A version every ten minutes for three days.
    for i in 0..(3 * 24 * 6) {
        world.wait(10 * 60 * 1000);
        world
            .change(ming, space, notes, "notes", Some(&u32::to_be_bytes(i)))
            .unwrap();
    }
    let all_versions = world.space(ming, space).unwrap().history(notes).len();
    world.checkpoint(ming, space).unwrap();
    world.compact(ming, space);
    let kept = world.space(ming, space).unwrap().history(notes).len();
    assert!(all_versions > 400);
    // About one an hour for a day, then one a day: about 24 + 3 + 1.
    assert!((20..=40).contains(&kept), "{kept} versions kept");
}

#[test]
fn a_grant_to_a_person_reaches_every_device_of_that_person() {
    let mut world = World::default();
    let (mom, laptop, phone) = (world.add(1), world.add(2), world.add(3));
    // Ming's identity, with the laptop and the phone as its nodes.
    let rotation = fileos_core::SecretKey::from_seed(&[50; 32]);
    let signing = fileos_core::SecretKey::from_seed(&[51; 32]);
    let state = fileos_id::State {
        rotation_keys: vec![fileos_id::did_key(rotation.public())],
        verification_methods: [(
            fileos_id::SIGNING_METHOD.to_string(),
            fileos_id::did_key(signing.public()),
        )]
        .into(),
        ..fileos_id::State::default()
    };
    let genesis = fileos_id::Op::genesis(state, &rotation).unwrap();
    let did = genesis.did();
    let ops = vec![fileos_core::Bytes(genesis.to_bytes())];
    for node in [laptop, phone] {
        let cert = fileos_id::NodeCert::sign(&did, &signing, world.key(node));
        world
            .set_identity(node, ops.clone(), fileos_core::Bytes(cert.to_bytes()))
            .unwrap();
    }

    // Mom shares a space with Ming, the person, not with one of his keys.
    let space = world.create(mom);
    world.put(mom, space, "hello", b"for Ming").unwrap();
    world
        .give_to(
            mom,
            space,
            Principal::Identity(genesis.hash()),
            &WRITE,
            all(),
        )
        .unwrap();

    // The phone comes online: it shows that it is Ming's, Mom's node writes a
    // vouch for it, and the space comes.
    world.connect(mom, phone);
    world.follow(phone, space);
    assert_eq!(world.read(phone, space, "hello"), [b"for Ming".to_vec()]);
    world.put(phone, space, "reply", b"thanks").unwrap();
    assert_eq!(world.read(mom, space, "reply"), [b"thanks".to_vec()]);

    // The laptop gets it from the phone, with Mom offline.
    world.disconnect(mom, phone);
    world.connect(phone, laptop);
    world.follow(laptop, space);
    assert_eq!(world.read(laptop, space, "reply"), [b"thanks".to_vec()]);

    // A node that is not Ming's gets nothing.
    let stranger = world.add(9);
    world.connect(mom, stranger);
    world.follow(stranger, space);
    assert!(world.space(stranger, space).is_none());
}
