// SPDX-License-Identifier: AGPL-3.0-only

use fos_core::SecretKey;

use super::*;

fn key(n: u8) -> SecretKey {
    SecretKey::from_seed(&[n; 32])
}

fn who(k: &SecretKey) -> Principal {
    Principal::Key(k.public())
}

fn create(name: &str, bytes: &[u8]) -> Act {
    Act::Create {
        name: name.into(),
        content: Content::of(bytes),
    }
}

fn give(to: &SecretKey, permissions: &[Permission], on: Resource) -> Act {
    Act::Give {
        to: who(to),
        effect: Effect::Allow,
        permissions: permissions.to_vec(),
        on,
    }
}

fn prefix(p: &str) -> Resource {
    Resource::Prefix(p.into())
}

fn first(entry: &Entry) -> Id {
    Id {
        entry: entry.hash(),
        index: 0,
    }
}

const WRITE: [Permission; 4] = [
    Permission::FilesRead,
    Permission::FilesCreate,
    Permission::FilesChange,
    Permission::FilesDelete,
];

/// A copy of `space` made from its entries in reverse order.
fn copy_backwards(space: &Space) -> Space {
    let mut entries = space.entries();
    let genesis = entries.remove(0);
    let mut copy = Space::from_genesis(&genesis).unwrap();
    entries.reverse();
    for e in entries {
        copy.insert(vec![e]).unwrap();
    }
    copy
}

#[test]
fn the_id_of_a_space_and_a_file_is_the_hash_of_the_entry_that_made_it() {
    let ming = key(1);
    let (mut space, genesis) = Space::create(who(&ming), &ming, 0, 7);
    assert_eq!(space.id(), genesis.hash());
    let e = space
        .write(&ming, 0, vec![create("2026/cat.jpg", b"cat")])
        .unwrap();
    let files = space.find("2026/cat.jpg");
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].id, first(&e));
}

#[test]
fn bad_entries_are_not_taken() {
    let ming = key(1);
    let (mut space, _) = Space::create(who(&ming), &ming, 0, 1);
    let mut e = space.write(&ming, 0, vec![create("a", b"a")]).unwrap();
    e.header.seq = 9;
    let mut other = Space::from_genesis(&space.entries()[0]).unwrap();
    other.insert(vec![e]).unwrap_err();
    space.write(&ming, 0, vec![create("/a", b"a")]).unwrap_err();
    space
        .write(&ming, 0, vec![create("a/../b", b"a")])
        .unwrap_err();
}

#[test]
fn a_rename_keeps_the_id_and_moves_out_of_a_prefix_grant() {
    let (ming, mom, chat) = (key(1), key(2), key(3));
    let (mut space, _) = Space::create(who(&ming), &ming, 0, 1);
    let e = space
        .write(&ming, 0, vec![create("2026/cat.jpg", b"cat")])
        .unwrap();
    let cat = first(&e);
    space
        .write(&ming, 0, vec![
                give(&mom, &[Permission::FilesRead], prefix("2026/")),
                give(&chat, &[Permission::FilesRead], Resource::File(cat)),
            ],
        )
        .unwrap();
    assert!(space.may_read(who(&mom), cat));
    assert!(space.may_read(who(&chat), cat));

    space
        .write(&ming, 0, vec![Act::Change {
                file: cat,
                parents: vec![cat],
                name: "best/cat.jpg".into(),
                content: Some(Content::of(b"cat")),
            }],
        )
        .unwrap();
    assert_eq!(
        space
            .file(cat)
            .unwrap()
            .names()
            .into_iter()
            .collect::<Vec<_>>(),
        ["best/cat.jpg"]
    );
    assert!(!space.may_read(who(&mom), cat), "the file left the prefix");
    assert!(
        space.may_read(who(&chat), cat),
        "a grant on a file follows it"
    );
}

#[test]
fn grants_are_given_on_only_smaller() {
    let (ming, mom, ann) = (key(1), key(2), key(3));
    let (mut space, _) = Space::create(who(&ming), &ming, 0, 1);
    space
        .write(&ming, 0, vec![give(
                &mom,
                &[Permission::FilesRead, Permission::AccessGive],
                prefix("2026/"),
            )],
        )
        .unwrap();
    // Less: fine.
    space
        .write(&mom, 0, vec![give(&ann, &[Permission::FilesRead], prefix("2026/05/"))],
        )
        .unwrap();
    // A wider prefix, or a permission Mom does not have: not allowed.
    space
        .write(&mom, 0, vec![give(&ann, &[Permission::FilesRead], prefix(""))])
        .unwrap_err();
    space
        .write(&mom, 0, vec![give(&ann, &[Permission::FilesChange], prefix("2026/"))],
        )
        .unwrap_err();
}

#[test]
fn taking_back_a_grant_takes_back_what_was_given_on_from_it() {
    let (ming, mom, ann) = (key(1), key(2), key(3));
    let (mut space, _) = Space::create(who(&ming), &ming, 0, 1);
    let g = space
        .write(&ming, 0, vec![give(
                &mom,
                &[Permission::FilesRead, Permission::AccessGive],
                prefix(""),
            )],
        )
        .unwrap();
    space
        .write(&mom, 0, vec![give(&ann, &[Permission::FilesRead], prefix(""))])
        .unwrap();
    let all = prefix("");
    assert!(space.may(who(&ann), Permission::FilesRead, &all));
    space
        .write(&ming, 0, vec![Act::Take { grant: first(&g) }])
        .unwrap();
    assert!(!space.may(who(&mom), Permission::FilesRead, &all));
    assert!(!space.may(who(&ann), Permission::FilesRead, &all));
    let anns = space
        .grants()
        .into_iter()
        .find(|g| g.given.to == who(&ann))
        .unwrap();
    assert!(!anns.in_effect, "the access page shows it as not in effect");
}

#[test]
fn a_deny_wins() {
    let (ming, mom) = (key(1), key(2));
    let (mut space, _) = Space::create(who(&ming), &ming, 0, 1);
    space
        .write(&ming, 0, vec![
                give(&mom, &WRITE, prefix("")),
                Act::Give {
                    to: who(&mom),
                    effect: Effect::Deny,
                    permissions: vec![Permission::FilesCreate],
                    on: prefix("private/"),
                },
            ],
        )
        .unwrap();
    space.write(&mom, 0, vec![create("2026/a", b"a")]).unwrap();
    space
        .write(&mom, 0, vec![create("private/a", b"a")])
        .unwrap_err();
}

#[test]
fn writes_at_the_same_time_are_kept_side_by_side_and_the_order_does_not_matter() {
    let (ming, mom) = (key(1), key(2));
    let (mut a, _) = Space::create(who(&ming), &ming, 0, 1);
    let e = a.write(&ming, 0, vec![create("notes", b"v1")]).unwrap();
    let notes = first(&e);
    a.write(&ming, 0, vec![give(&mom, &WRITE, prefix(""))])
        .unwrap();
    let mut b = copy_backwards(&a);

    let change = |bytes: &[u8]| Act::Change {
        file: notes,
        parents: vec![notes],
        name: "notes".into(),
        content: Some(Content::of(bytes)),
    };
    let x = a.write(&ming, 0, vec![change(b"ming")]).unwrap();
    let y = b.write(&mom, 0, vec![change(b"mom")]).unwrap();
    a.insert(vec![y]).unwrap();
    b.insert(vec![x]).unwrap();

    assert_eq!(a.digest(), b.digest());
    assert_eq!(copy_backwards(&a).digest(), a.digest());
    let file = a.file(notes).unwrap();
    assert_eq!(file.versions.len(), 2, "both versions are kept");

    // A change after both versions settles it.
    let parents = file.versions.iter().map(|v| v.id).collect();
    a.write(&ming, 0, vec![Act::Change {
            file: notes,
            parents,
            name: "notes".into(),
            content: Some(Content::of(b"both")),
        }],
    )
    .unwrap();
    assert_eq!(a.file(notes).unwrap().versions.len(), 1);
}

#[test]
fn strong_removal_undoes_what_was_done_at_the_same_time() {
    let (ming, mom) = (key(1), key(2));
    let (mut a, _) = Space::create(who(&ming), &ming, 0, 1);
    let g = a
        .write(&ming, 0, vec![give(&mom, &WRITE, prefix(""))])
        .unwrap();
    let mut b = copy_backwards(&a);

    // Mom writes, offline, while Ming takes her access back.
    let w = b.write(&mom, 0, vec![create("from-mom", b"hi")]).unwrap();
    assert_eq!(b.find("from-mom").len(), 1, "Mom reads her own write");
    let t = a
        .write(&ming, 0, vec![Act::Take { grant: first(&g) }])
        .unwrap();

    a.insert(vec![w.clone()]).unwrap();
    b.insert(vec![t]).unwrap();
    assert_eq!(a.digest(), b.digest());
    assert_eq!(a.find("from-mom"), Vec::new());
    assert!(a.rejected().contains_key(&w.hash()));
}

#[test]
fn a_write_before_the_removal_stays() {
    let (ming, mom) = (key(1), key(2));
    let (mut a, _) = Space::create(who(&ming), &ming, 0, 1);
    let g = a
        .write(&ming, 0, vec![give(&mom, &WRITE, prefix(""))])
        .unwrap();
    let mut b = copy_backwards(&a);
    let w = b.write(&mom, 0, vec![create("from-mom", b"hi")]).unwrap();
    a.insert(vec![w]).unwrap();
    a.write(&ming, 0, vec![Act::Take { grant: first(&g) }])
        .unwrap();
    assert_eq!(a.find("from-mom").len(), 1);
}

#[test]
fn two_managers_who_take_each_other_at_the_same_time_both_lose_and_the_owner_stays() {
    let (ming, mom, dad) = (key(1), key(2), key(3));
    let manage = [
        Permission::AccessGive,
        Permission::AccessTake,
        Permission::FilesCreate,
    ];
    let (mut a, _) = Space::create(who(&ming), &ming, 0, 1);
    let e = a
        .write(&ming, 0, vec![
                give(&mom, &manage, prefix("")),
                give(&dad, &manage, prefix("")),
            ],
        )
        .unwrap();
    let (to_mom, to_dad) = (
        first(&e),
        Id {
            entry: e.hash(),
            index: 1,
        },
    );
    let mut b = copy_backwards(&a);
    let x = a.write(&mom, 0, vec![Act::Take { grant: to_dad }]).unwrap();
    let y = b.write(&dad, 0, vec![Act::Take { grant: to_mom }]).unwrap();
    a.insert(vec![y]).unwrap();
    b.insert(vec![x]).unwrap();
    assert_eq!(a.digest(), b.digest());
    let all = prefix("");
    assert!(!a.may(who(&mom), Permission::FilesCreate, &all));
    assert!(!a.may(who(&dad), Permission::FilesCreate, &all));
    assert!(a.may(who(&ming), Permission::FilesCreate, &all));
    a.write(&ming, 0, vec![create("still-here", b"")]).unwrap();
}

fn group(space: &mut Space, by: &SecretKey, owner: Principal) -> Id {
    first(&space.write(by, 0, vec![Act::Group { owner }]).unwrap())
}

fn add(space: &mut Space, by: &SecretKey, to: Principal, group: Id) -> Id {
    first(
        &space
            .write(
                by,
                0,
                vec![Act::Give {
                    to,
                    effect: Effect::Allow,
                    permissions: vec![Permission::Member],
                    on: Resource::Group(group),
                }],
            )
            .unwrap(),
    )
}

#[test]
fn a_grant_to_a_group_is_for_everyone_in_it() {
    let (ming, mom, dad) = (key(1), key(2), key(3));
    let (mut space, _) = Space::create(who(&ming), &ming, 0, 1);
    let family = group(&mut space, &ming, who(&ming));
    let to_mom = add(&mut space, &ming, who(&mom), family);
    space
        .write(
            &ming,
            0,
            vec![Act::Give {
                to: Principal::Group(family),
                effect: Effect::Allow,
                permissions: vec![Permission::FilesRead],
                on: prefix("photos/"),
            }],
        )
        .unwrap();
    let photos = prefix("photos/");
    assert!(space.may(who(&mom), Permission::FilesRead, &photos));
    assert!(!space.may(who(&dad), Permission::FilesRead, &photos));
    add(&mut space, &ming, who(&dad), family);
    assert!(space.may(who(&dad), Permission::FilesRead, &photos));
    space.write(&ming, 0, vec![Act::Take { grant: to_mom }]).unwrap();
    assert!(!space.is_in(who(&mom), family));
    assert!(!space.may(who(&mom), Permission::FilesRead, &photos));
}

#[test]
fn a_group_owned_by_a_group_is_managed_by_its_members() {
    let (ming, mom, ann) = (key(1), key(2), key(3));
    let (mut space, _) = Space::create(who(&ming), &ming, 0, 1);
    let parents = group(&mut space, &ming, who(&ming));
    add(&mut space, &ming, who(&mom), parents);
    // Mom is in "parents", which owns "family": she manages "family".
    let family = group(&mut space, &mom, Principal::Group(parents));
    add(&mut space, &mom, who(&ann), family);
    assert!(space.is_in(who(&ann), family));
    // Ann is in "family" but does not manage it.
    space
        .write(
            &ann,
            0,
            vec![Act::Give {
                to: who(&key(4)),
                effect: Effect::Allow,
                permissions: vec![Permission::Member],
                on: Resource::Group(family),
            }],
        )
        .unwrap_err();
}

#[test]
fn groups_in_groups_and_cycles() {
    let (ming, ann, stranger) = (key(1), key(2), key(9));
    let (mut space, _) = Space::create(who(&ming), &ming, 0, 1);
    let a = group(&mut space, &ming, who(&ming));
    let b = group(&mut space, &ming, who(&ming));
    add(&mut space, &ming, Principal::Group(b), a);
    add(&mut space, &ming, Principal::Group(a), b);
    add(&mut space, &ming, who(&ann), b);
    assert!(space.is_in(who(&ann), a), "Ann is in b, which is in a");
    assert!(!space.is_in(who(&stranger), a), "the cycle gives nothing");
}
