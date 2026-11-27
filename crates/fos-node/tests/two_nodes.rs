// SPDX-License-Identifier: AGPL-3.0-only

//! Two nodes over libp2p, with the photos app
//! from `target/apps` (run `apps` first).

use std::path::{Path, PathBuf};
use std::time::Duration;

use fos_node::{NetConfig, Node, parse_permissions};

fn apps() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/apps");
    assert!(
        dir.join("Photos.app").exists(),
        "{} is missing: run `apps` first",
        dir.display()
    );
    dir
}

/// Waits until `ok` is true, for at most 30 seconds.
async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..600 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out: {what}");
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png
}

fn start(dir: &Path) -> Node {
    let node = Node::open(dir).unwrap();
    node.start_net(NetConfig {
        listen: vec!["/ip4/127.0.0.1/tcp/0".into()],
        peers: Vec::new(),
        mdns: false,
    })
    .unwrap();
    node
}

#[tokio::test(flavor = "multi_thread")]
async fn two_nodes_share_a_photos_space() {
    let apps = apps();
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    Node::init(dir_a.path()).unwrap();
    Node::init(dir_b.path()).unwrap();
    let alice = start(dir_a.path());
    let bob = start(dir_b.path());

    // Bob connects to Alice.
    until("alice listens", || !alice.status().listening.is_empty()).await;
    bob.dial(&alice.status().listening[0]).unwrap();
    until("they connect", || alice.status().peers.contains(&bob.key())).await;

    // Alice makes a space and lets Bob's node use, keep and manage it.
    let photos = alice.create("/photos").unwrap();
    alice
        .grant("/photos", bob.key(), &parse_permissions("all").unwrap())
        .unwrap();
    bob.mount(photos, "/photos").unwrap();
    let entries = |node: &Node| {
        node.status()
            .spaces
            .iter()
            .find(|f| f.id == photos)
            .map_or(0, |f| f.entries)
    };
    until("bob gets the space", || entries(&bob) == entries(&alice)).await;

    // Bob installs the photos app. Bob gives its key a grant in Alice's
    // space: he may, because Alice let him manage it.
    let app_key = bob.install(&apps.join("Photos.app")).unwrap().key;
    assert_ne!(app_key, bob.key());
    assert_eq!(bob.status().apps, ["photos"]);

    // A photo from Alice comes to Bob, and his photos app indexes it.
    alice.put("/photos/2026/a.png", &png(64, 32)).unwrap();
    until("bob's app indexes alice's photo", || {
        bob.kv("photos")
            .get("photo:/photos/2026/a.png")
            .map(Vec::as_slice)
            == Some(b"64x32")
    })
    .await;
    assert_eq!(bob.read("/photos/2026/a.png").unwrap(), png(64, 32));

    // Bob writes too, with no ordering node.
    bob.put("/photos/2026/b.png", &png(10, 20)).unwrap();
    until("alice gets bob's photo", || {
        alice.read("/photos/2026/b.png").is_ok()
    })
    .await;
    until("both copies are the same", || {
        entries(&bob) == entries(&alice)
    })
    .await;
    assert_eq!(alice.list("/photos/2026").unwrap(), ["a.png", "b.png"]);

    // Alice has no photos app; the app is Bob's alone.
    assert!(alice.kv("photos").is_empty());
    bob.read("/apps/Photos.app/manifest.toml").unwrap();
    alice.read("/apps/Photos.app/manifest.toml").unwrap_err();

    // A file with more pieces than fit in the network queues still arrives.
    let mut noise = 1_u64;
    let video: Vec<u8> = (0..(4 << 20))
        .map(|_| {
            noise ^= noise << 13;
            noise ^= noise >> 7;
            noise ^= noise << 17;
            noise as u8
        })
        .collect();
    alice.put("/photos/clip.bin", &video).unwrap();
    until("bob gets every piece of the file", || {
        bob.read("/photos/clip.bin")
            .as_deref()
            .is_ok_and(|bytes| bytes == video)
    })
    .await;

    // Bob's node comes back after a restart, with its spaces and its app.
    let before = entries(&bob);
    drop(bob);
    let bob = Node::open(dir_b.path()).unwrap();
    assert_eq!(entries(&bob), before);
    assert_eq!(bob.status().apps, ["photos"]);
    assert_eq!(bob.read("/photos/2026/b.png").unwrap(), png(10, 20));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_without_a_grant_gets_nothing() {
    let (dir_a, dir_e) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    Node::init(dir_a.path()).unwrap();
    Node::init(dir_e.path()).unwrap();
    let alice = start(dir_a.path());
    let eve = start(dir_e.path());
    until("alice listens", || !alice.status().listening.is_empty()).await;
    eve.dial(&alice.status().listening[0]).unwrap();
    until("they connect", || alice.status().peers.contains(&eve.key())).await;

    let secret = alice.create("/secret").unwrap();
    alice.put("/secret/plan.txt", b"plan").unwrap();
    eve.mount(secret, "/secret").unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(eve.status().spaces.iter().all(|f| f.id != secret));
    eve.read("/secret/plan.txt").unwrap_err();
    // Without a grant, Eve cannot write into Alice's space from her own node.
    eve.put("/secret/x.txt", b"x").unwrap_err();
}
