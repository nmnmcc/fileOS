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
    assert_eq!(world.read(mom, space, "video.bin"), std::slice::from_ref(&big));

    let before = world.nodes[mom].contents.0.len();
    let mut edited = big.clone();
    edited[1 << 20] ^= 1;
    world
        .change(ming, space, file, "video.bin", Some(&edited))
        .unwrap();
    assert_eq!(world.read(mom, space, "video.bin"), [edited]);
    assert!(world.nodes[mom].contents.0.len() - before <= 3);
}
