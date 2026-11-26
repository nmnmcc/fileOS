// SPDX-License-Identifier: AGPL-3.0-only

use super::*;

fn key(n: u8) -> SecretKey {
    SecretKey::from_seed(&[n; 32])
}

fn put(folder: &Folder, author: &SecretKey, nonce: u64, path: &str, data: &[u8]) -> SignedWrite {
    let op = Op::Put {
        path: path.into(),
        blob: Hash::of(data),
        size: data.len() as u64,
    };
    SignedWrite::sign(Some(folder.id()), nonce, op, author)
}

#[test]
fn files_streams_and_lists() {
    let alice = key(1);
    let mut open = Open::default();
    let (mut f, _) = Folder::create("family", &alice, &mut open);
    f.order(put(&f, &alice, 1, "/photos/a.jpg", b"a"), &alice, &mut open)
        .unwrap();
    f.order(put(&f, &alice, 2, "/photos/b.jpg", b"b"), &alice, &mut open)
        .unwrap();
    let chat = Op::Append {
        path: "/chat".into(),
        data: Bytes(b"hi".to_vec()),
    };
    f.order(
        SignedWrite::sign(Some(f.id()), 3, chat, &alice),
        &alice,
        &mut open,
    )
    .unwrap();

    assert_eq!(f.list("/"), ["chat", "photos"]);
    assert_eq!(f.list("/photos"), ["a.jpg", "b.jpg"]);
    assert_eq!(f.file("/photos/a.jpg"), Some((Hash::of(b"a"), 1)));

    let delete = Op::Delete {
        path: "/photos/a.jpg".into(),
    };
    f.order(
        SignedWrite::sign(Some(f.id()), 4, delete, &alice),
        &alice,
        &mut open,
    )
    .unwrap();
    assert_eq!(f.list("/photos"), ["b.jpg"]);
    assert_eq!(f.next_seq(), 5);

    // A file cannot become a stream.
    let bad = Op::Append {
        path: "/photos/b.jpg".into(),
        data: Bytes(vec![]),
    };
    f.order(
        SignedWrite::sign(Some(f.id()), 5, bad, &alice),
        &alice,
        &mut open,
    )
    .unwrap_err();
    // Bad paths are not taken.
    f.order(put(&f, &alice, 6, "/a/../b", b"x"), &alice, &mut open)
        .unwrap_err();
}

#[test]
fn a_copy_follows_the_main_line() {
    let (alice, bob) = (key(1), key(2));
    let mut open = Open::default();
    let (mut main, genesis) = Folder::create("family", &alice, &mut open);
    let mut copy = Folder::from_genesis(genesis, &mut open).unwrap();

    let e1 = main
        .order(put(&main, &bob, 1, "/a", b"a"), &alice, &mut open)
        .unwrap();
    let e2 = main
        .order(put(&main, &bob, 2, "/b", b"b"), &alice, &mut open)
        .unwrap();
    assert_eq!(
        copy.accept(e2.clone(), &mut open),
        Err(Reject::Gap { next: 1 })
    );
    copy.accept(e1.clone(), &mut open).unwrap();
    copy.accept(e2, &mut open).unwrap();
    assert_eq!(copy.accept(e1, &mut open), Err(Reject::Old));
    assert_eq!(copy.head(), main.head());

    // Only the ordering node may order.
    copy.order(put(&copy, &bob, 3, "/c", b"c"), &bob, &mut open)
        .unwrap_err();

    // A changed entry is not taken.
    let mut e3 = main
        .order(put(&main, &bob, 3, "/c", b"c"), &alice, &mut open)
        .unwrap();
    e3.write.write.nonce = 99;
    assert!(matches!(
        copy.accept(e3, &mut open),
        Err(Reject::Invalid(_))
    ));
}

#[test]
fn a_different_engine_forks() {
    let (alice, carol, dave) = (key(1), key(3), key(4));
    let mut alice_engine = Open::default();
    let (mut main, genesis) = Folder::create("family", &alice, &mut alice_engine);
    // Carol's node does not take Dave's writes.
    let mut carol_engine = Open {
        deny: vec![dave.public()],
        ..Open::default()
    };
    let mut copy = Folder::from_genesis(genesis, &mut carol_engine).unwrap();

    let e1 = main
        .order(put(&main, &carol, 1, "/a", b"a"), &alice, &mut alice_engine)
        .unwrap();
    let e2 = main
        .order(put(&main, &dave, 1, "/d", b"d"), &alice, &mut alice_engine)
        .unwrap();
    let e3 = main
        .order(put(&main, &carol, 2, "/b", b"b"), &alice, &mut alice_engine)
        .unwrap();

    copy.accept(e1, &mut carol_engine).unwrap();
    assert_eq!(
        copy.accept(e2, &mut carol_engine),
        Err(Reject::Fork(Fork {
            seq: 2,
            reason: "denied".into()
        }))
    );
    assert_eq!(copy.accept(e3, &mut carol_engine), Err(Reject::Forked));
    assert_eq!(copy.fork().map(|f| f.seq), Some(2));
    assert_eq!(copy.list("/"), ["a"]);
    assert_eq!(main.list("/"), ["a", "b", "d"]);
}

#[test]
fn mounts_resolve() {
    let alice = key(1);
    let mut open = Open::default();
    let (mut root, _) = Folder::create("root", &alice, &mut open);
    let (mut photos, _) = Folder::create("photos", &alice, &mut open);
    photos
        .order(
            put(&photos, &alice, 1, "/2026/a.jpg", b"a"),
            &alice,
            &mut open,
        )
        .unwrap();
    let mount = Op::Mount {
        path: "/photos".into(),
        folder: photos.id(),
    };
    root.order(
        SignedWrite::sign(Some(root.id()), 1, mount, &alice),
        &alice,
        &mut open,
    )
    .unwrap();

    let folders = [root.clone(), photos.clone()];
    let lookup = |id: Hash| folders.iter().find(|f| f.id() == id);
    assert_eq!(
        resolve(root.id(), "/photos/2026/a.jpg", lookup).unwrap(),
        (photos.id(), "/2026/a.jpg".into())
    );
    assert_eq!(
        resolve(root.id(), "/photos", lookup).unwrap(),
        (photos.id(), "/".into())
    );
    assert_eq!(
        resolve(root.id(), "/notes/x", lookup).unwrap(),
        (root.id(), "/notes/x".into())
    );
    resolve(root.id(), "photos", lookup).unwrap_err();
}
