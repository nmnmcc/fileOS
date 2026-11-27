// SPDX-License-Identifier: AGPL-3.0-only

//! One identity (a did:fos DID) with two nodes, over libp2p.

use std::path::Path;
use std::time::Duration;

use fos_core::SecretKey;
use fos_id::Link;
use fos_node::{NetConfig, Node, parse_permissions, parse_principal};

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

fn start(dir: &Path) -> Node {
    let node = Node::open(dir).unwrap();
    node.start_net(NetConfig {
        listen: vec!["/ip4/127.0.0.1/tcp/0".parse().unwrap()],
        peers: Vec::new(),
        mdns: false,
    })
    .unwrap();
    node
}

#[tokio::test(flavor = "multi_thread")]
async fn an_identity_has_its_nodes() {
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    Node::init(dir_a.path()).unwrap();
    Node::init(dir_b.path()).unwrap();
    let laptop = start(dir_a.path());
    let vps = start(dir_b.path());
    until("the laptop listens", || {
        !laptop.status().listening.is_empty()
    })
    .await;
    vps.dial(&laptop.status().listening[0]).unwrap();
    until("they connect", || {
        laptop.status().peers.contains(&vps.key())
    })
    .await;

    // The laptop makes the identity and holds its keys. It is the first node.
    assert!(laptop.identity().is_none());
    let did = laptop.identity_new().unwrap();
    assert!(did.starts_with("did:fos:"));
    let id = laptop.identity().unwrap();
    assert_eq!(id.did.as_deref(), Some(did.as_str()));
    assert!(id.holds_rotation_key && id.holds_signing_key);
    assert_eq!(id.rotation_keys.len(), 1);
    assert_eq!(id.nodes, [laptop.key()]);
    let link: Link = id.link.unwrap().parse().unwrap();
    assert_eq!(link.did, did);
    assert_eq!(link.hints, laptop.status().listening);
    laptop.identity_new().unwrap_err();

    // The laptop adds the VPS, and the VPS joins.
    let space = laptop.identity_add_node(vps.key()).unwrap();
    assert_eq!(space, id.space);
    vps.identity_join(space).unwrap();
    until("the VPS gets the identity space", || {
        vps.identity().is_some_and(|id| id.nodes.len() == 2)
    })
    .await;
    let on_vps = vps.identity().unwrap();
    assert_eq!(on_vps.did.as_deref(), Some(did.as_str()));
    assert!(!on_vps.holds_rotation_key && !on_vps.holds_signing_key);
    assert!(on_vps.nodes.contains(&laptop.key()) && on_vps.nodes.contains(&vps.key()));

    // Only the node that holds the signing key can add nodes. The VPS cannot, and
    // cannot write into the identity space.
    let stranger = SecretKey::from_seed(&[7; 32]).public();
    vps.identity_add_node(stranger).unwrap_err();
    vps.put("/identity/nodes/x", b"x").unwrap_err();
    assert_eq!(vps.list("/identity/nodes").unwrap().len(), 2);

    // A space shared with the person, not with one key, reaches every node
    // of the identity.
    let shared = laptop.create("/shared").unwrap();
    laptop.put("/shared/note.txt", b"for all my nodes").unwrap();
    laptop
        .grant_to(
            "/shared",
            parse_principal(&did).unwrap(),
            &parse_permissions("read-write").unwrap(),
        )
        .unwrap();
    vps.mount(shared, "/shared").unwrap();
    until("the VPS gets the space shared with the identity", || {
        vps.read("/shared/note.txt").is_ok()
    })
    .await;
    vps.put("/shared/from-vps.txt", b"hi").unwrap();
    until("the laptop gets the VPS's write", || {
        laptop.read("/shared/from-vps.txt").is_ok()
    })
    .await;

    // After a restart, the laptop still holds the keys and can take the VPS away.
    drop(laptop);
    let laptop = start(dir_a.path());
    assert!(laptop.identity().unwrap().holds_signing_key);
    until("the laptop listens", || {
        !laptop.status().listening.is_empty()
    })
    .await;
    vps.dial(&laptop.status().listening[0]).unwrap();
    until("they connect again", || {
        laptop.status().peers.contains(&vps.key())
    })
    .await;
    laptop.identity_remove_node(vps.key()).unwrap();
    assert_eq!(laptop.identity().unwrap().nodes, [laptop.key()]);
    // The VPS is no longer a node of the identity in the shared space: its
    // own node refuses its write once the revocation comes, and a write
    // that went out before has no effect on the laptop.
    let rejected = |node: &Node| {
        node.status()
            .spaces
            .iter()
            .find(|s| s.id == shared)
            .map_or(0, |s| s.rejected)
    };
    let before = rejected(&laptop);
    until("the VPS's write is refused", || {
        vps.put("/shared/after.txt", b"x").is_err() || rejected(&laptop) > before
    })
    .await;
    laptop.read("/shared/after.txt").unwrap_err();
    until("the VPS sees that it was taken away", || {
        vps.identity().is_some_and(|id| id.nodes == [laptop.key()])
    })
    .await;
    // A node that is not in the identity gives no addresses in its link.
    let link: Link = vps.identity().unwrap().link.unwrap().parse().unwrap();
    assert_eq!(link.hints, Vec::<String>::new());
}
