// SPDX-License-Identifier: AGPL-3.0-only

use fos_space::Permission::{
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
    let rotation = fos_core::SecretKey::from_seed(&[50; 32]);
    let signing = fos_core::SecretKey::from_seed(&[51; 32]);
    let state = fos_id::State {
        rotation_keys: vec![fos_id::did_key(rotation.public())],
        verification_methods: [(
            fos_id::SIGNING_METHOD.to_string(),
            fos_id::did_key(signing.public()),
        )]
        .into(),
        ..fos_id::State::default()
    };
    let genesis = fos_id::Op::genesis(state, &rotation).unwrap();
    let did = genesis.did();
    let ops = vec![fos_core::Bytes(genesis.to_bytes())];
    for node in [laptop, phone] {
        let cert = fos_id::NodeCert::sign(&did, &signing, world.key(node));
        world
            .set_identity(node, ops.clone(), fos_core::Bytes(cert.to_bytes()))
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

    // The laptop gets it from the phone, with Mom offline. The phone also
    // gives the laptop the keys: both may read, so this leaks nothing.
    world.disconnect(mom, phone);
    world.connect(phone, laptop);
    world.follow(laptop, space);
    assert_eq!(world.read(laptop, space, "reply"), [b"thanks".to_vec()]);

    // A node that is not Ming's gets nothing.
    let stranger = world.add(9);
    world.connect(mom, stranger);
    world.follow(stranger, space);
    assert!(world.space(stranger, space).is_none());

    // The phone is lost. Ming takes it away from his identity: Mom's node
    // starts a new epoch, and the phone cannot read what comes after, even
    // with every entry and every piece.
    let revocation = fos_id::NodeRevocation::sign(&did, &signing, world.key(phone));
    world.connect(mom, laptop);
    world.revoke(laptop, vec![fos_core::Bytes(revocation.to_bytes())]);
    world
        .put(mom, space, "after", b"not for the phone")
        .unwrap();
    assert_eq!(
        world.read(laptop, space, "after"),
        [b"not for the phone".to_vec()]
    );
    let entries = world.space(mom, space).unwrap().entries();
    let mut copy = fos_space::Space::from_genesis(&entries[0]).unwrap();
    copy.insert(entries[1..].to_vec()).unwrap();
    let ring = copy.keyring(&[&world.nodes[phone].key]);
    let file = copy.find("after").remove(0);
    assert_eq!(
        copy.data_keys(&ring, file.id, &file.versions[0]),
        Vec::<[u8; 32]>::new()
    );
}

#[test]
fn a_node_that_only_keeps_a_space_sees_only_ciphertext() {
    let mut world = World::default();
    let (ming, relay) = (world.add(1), world.add(7));
    let space = world.create(ming);
    let secret = b"the plan for the party, which nobody else may read".repeat(10);
    world.put(ming, space, "plan.txt", &secret).unwrap();
    world.give(ming, space, relay, &[DataKeep], all()).unwrap();
    world.connect(ming, relay);
    world.follow(relay, space);
    assert_eq!(
        world.find(relay, space, "plan.txt").len(),
        1,
        "it has the space"
    );
    assert_eq!(world.read(relay, space, "plan.txt"), Vec::<Vec<u8>>::new());
    assert!(
        !world.nodes[relay].contents.0.is_empty(),
        "it has the pieces"
    );
    for piece in world.nodes[relay].contents.0.values() {
        assert!(
            !piece
                .windows(16)
                .any(|w| secret.windows(16).any(|s| s == w))
        );
    }
}

#[test]
fn a_reader_whose_access_is_taken_back_cannot_read_what_comes_after() {
    let mut world = World::default();
    let (ming, mom, dad) = (world.add(1), world.add(2), world.add(3));
    let space = world.create(ming);
    let to_mom = world
        .give(ming, space, mom, &[DataKeep, FilesRead], all())
        .unwrap();
    world
        .give(ming, space, dad, &[DataKeep, FilesRead], all())
        .unwrap();
    world.put(ming, space, "before", b"old news").unwrap();
    world.connect(ming, mom);
    world.connect(ming, dad);
    world.follow(mom, space);
    world.follow(dad, space);
    assert_eq!(world.read(mom, space, "before"), [b"old news".to_vec()]);
    let epoch = world.space(ming, space).unwrap().epoch();

    world.take(ming, space, to_mom).unwrap();
    let new_epoch = world.space(ming, space).unwrap().epoch();
    assert_ne!(epoch, new_epoch, "taking access back starts a new epoch");
    world.put(ming, space, "after", b"new plans").unwrap();
    assert_eq!(world.read(dad, space, "after"), [b"new plans".to_vec()]);
    assert_eq!(world.read(dad, space, "before"), [b"old news".to_vec()]);

    // Even with every entry and piece, Mom's node has no key for the new
    // epoch.
    let entries = world.space(ming, space).unwrap().entries();
    world.nodes[mom].node.space(space).unwrap();
    let pieces = world.nodes[ming].contents.0.clone();
    world.nodes[mom].contents.0.extend(pieces);
    let mut copy = fos_space::Space::from_genesis(&entries[0]).unwrap();
    copy.insert(entries[1..].to_vec()).unwrap();
    let ring = copy.keyring(&[&world.nodes[mom].key]);
    let file = copy.find("after").remove(0);
    let v = &file.versions[0];
    assert_eq!(copy.data_keys(&ring, file.id, v), Vec::<[u8; 32]>::new());
}

#[test]
fn a_member_of_a_group_gets_the_keys_of_the_group() {
    let mut world = World::default();
    let (ming, mom) = (world.add(1), world.add(2));
    let space = world.create(ming);
    world.put(ming, space, "photos/a.jpg", b"jpeg").unwrap();
    let family = world
        .write(
            ming,
            space,
            vec![Act::Group {
                owner: world.who(ming),
            }],
        )
        .unwrap();
    world
        .write(
            ming,
            space,
            vec![Act::Give {
                to: Principal::Group(family),
                effect: Effect::Allow,
                permissions: vec![DataKeep, FilesRead],
                on: Resource::Prefix("photos/".into()),
            }],
        )
        .unwrap();
    // Mom keeps the space, but is not in the family yet.
    world.give(ming, space, mom, &[DataKeep], all()).unwrap();
    world.connect(ming, mom);
    world.follow(mom, space);
    assert_eq!(
        world.read(mom, space, "photos/a.jpg"),
        Vec::<Vec<u8>>::new()
    );
    world
        .write(
            ming,
            space,
            vec![Act::Give {
                to: world.who(mom),
                effect: Effect::Allow,
                permissions: vec![Permission::Member],
                on: Resource::Group(family),
            }],
        )
        .unwrap();
    assert_eq!(world.read(mom, space, "photos/a.jpg"), [b"jpeg".to_vec()]);
}

#[test]
fn a_grant_on_one_file_opens_that_file_only() {
    let mut world = World::default();
    let (ming, chat) = (world.add(1), world.add(3));
    let space = world.create(ming);
    let cat = world.put(ming, space, "2026/cat.jpg", b"cat").unwrap();
    world.put(ming, space, "2026/dog.jpg", b"dog").unwrap();
    world.give(ming, space, chat, &[DataKeep], all()).unwrap();
    world
        .give(ming, space, chat, &[FilesRead], Resource::File(cat))
        .unwrap();
    world.connect(ming, chat);
    world.follow(chat, space);
    assert_eq!(world.read(chat, space, "2026/cat.jpg"), [b"cat".to_vec()]);
    assert_eq!(
        world.read(chat, space, "2026/dog.jpg"),
        Vec::<Vec<u8>>::new()
    );
    // Renamed, it is still the same key.
    world
        .change(ming, space, cat, "best/cat.jpg", Some(b"cat"))
        .unwrap();
    assert_eq!(world.read(chat, space, "best/cat.jpg"), [b"cat".to_vec()]);
    // In a new epoch the file gets a new data key, and the reader gets it.
    let other = world.add(9);
    let g = world.give(ming, space, other, &[FilesRead], all()).unwrap();
    world.take(ming, space, g).unwrap();
    world
        .change(ming, space, cat, "best/cat.jpg", Some(b"cat, again"))
        .unwrap();
    assert_eq!(
        world.read(chat, space, "best/cat.jpg"),
        [b"cat, again".to_vec()]
    );
}

#[test]
fn a_deleted_file_cannot_be_opened_once_its_keys_are_dropped() {
    let (mut world, ming, mom, space) = family();
    let secret = world.put(ming, space, "secret.txt", b"burn this").unwrap();
    let old = world
        .space(ming, space)
        .unwrap()
        .file(secret)
        .unwrap()
        .versions[0]
        .clone();
    world
        .change(ming, space, secret, "secret.txt", None)
        .unwrap();
    // Access to the space changes, so there is a new epoch; and time goes by,
    // so the bin empties at the next checkpoint.
    let other = world.add(9);
    let grant = world.give(ming, space, other, &[FilesRead], all()).unwrap();
    world.take(ming, space, grant).unwrap();
    world.wait(31 * 24 * 60 * 60 * 1000);
    world.checkpoint(ming, space).unwrap();
    world.compact(ming, space);
    world.compact(mom, space);
    for node in [ming, mom] {
        let n = &mut world.nodes[node];
        let s = n.node.space(space).unwrap();
        assert!(s.file(secret).is_none(), "the file is gone");
        let ring = s.keyring(&[&n.key]);
        assert!(
            s.data_keys(&ring, secret, &old).is_empty(),
            "and so is its key"
        );
        assert_eq!(n.node.open(space, secret, &old, &n.contents), None);
    }
}
