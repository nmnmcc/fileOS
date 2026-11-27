// SPDX-License-Identifier: AGPL-3.0-only

//! Times file writes, a sync, and reads through the in-memory nodes.

use std::hint::black_box;
use std::time::Instant;

use fileos_sim::world::World;
use fileos_space::{Permission, Resource};

fn main() {
    for count in [100, 1000] {
        let mut world = World::default();
        let writer = world.add(1);
        let reader = world.add(2);
        let space = world.create(writer);
        world
            .give(
                writer,
                space,
                reader,
                &[Permission::FilesRead],
                Resource::Prefix(String::new()),
            )
            .unwrap();

        let start = Instant::now();
        for i in 0..count {
            world
                .put(writer, space, &format!("file-{i}"), &i32::to_le_bytes(i))
                .unwrap();
        }
        let write = start.elapsed();

        let start = Instant::now();
        world.connect(writer, reader);
        world.follow(reader, space);
        let sync = start.elapsed();
        assert_eq!(
            world.space(writer, space).unwrap().digest(),
            world.space(reader, space).unwrap().digest()
        );

        let start = Instant::now();
        for i in 0..count {
            black_box(world.read(reader, space, &format!("file-{i}")));
        }
        let read = start.elapsed();
        println!("{count} files: write {write:?}, sync {sync:?}, read {read:?}");
    }
}
