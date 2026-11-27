// SPDX-License-Identifier: AGPL-3.0-only

//! Apps belong to the identity: an app installed on one node of a person
//! runs on every node of that person, with the same ID, key and view. Uses
//! the chat app from `target/apps` (run `apps` first).

use std::path::{Path, PathBuf};
use std::time::Duration;

use fileos_node::{NetConfig, Node};

fn apps() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/apps");
    assert!(
        dir.join("Chat.app").exists(),
        "{} is missing: run `apps` first",
        dir.display()
    );
    dir
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

/// The page that an app gives for `path`, as text.
async fn page(node: &Node, app: &str, path: &str) -> String {
    let answer = node
        .http_app(app)
        .unwrap()
        .request("GET", path, &[], Vec::new())
        .await
        .unwrap();
    assert_eq!(answer.status, 200, "{path}");
    String::from_utf8_lossy(&answer.body).into_owned()
}

/// Waits until the page of an app has `text` in it.
async fn until_page_has(node: &Node, app: &str, path: &str, text: &str) {
    for _ in 0..600 {
        if page(node, app, path).await.contains(text) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out: {path} has no {text:?}");
}

async fn send(node: &Node, app: &str, room: &str, from: &str, text: &str) {
    let answer = node
        .http_app(app)
        .unwrap()
        .request(
            "POST",
            &format!("/room/{room}"),
            &[("content-type", "application/x-www-form-urlencoded")],
            format!("from={from}&text={text}").into_bytes(),
        )
        .await
        .unwrap();
    assert_eq!(answer.status, 303);
    assert_eq!(answer.header("location"), Some(&*format!("/room/{room}")));
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_installed_on_one_node_runs_on_every_node_of_the_person() {
    let apps = apps();
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

    // The laptop makes the identity, a chat space for the person, and
    // installs the chat app.
    let did = laptop.identity_new().unwrap();
    let person = fileos_node::parse_principal(&did).unwrap();
    laptop.create("/chat").unwrap();
    laptop
        .grant_to(
            "/chat",
            person,
            &fileos_node::parse_permissions("read-write").unwrap(),
        )
        .unwrap();
    let id = laptop.install(&apps.join("Chat.app")).unwrap().id;

    // The VPS joins the identity. It gets the home space of the identity,
    // and with it the app, which it starts with the same ID.
    let space = laptop.identity_add_node(vps.key()).unwrap();
    vps.identity_join(space).unwrap();
    until("the VPS runs the chat app", || vps.http_app(&id).is_some()).await;

    // A message sent through the app on the VPS is seen on the laptop.
    send(&vps, &id, "family", "Ming", "from+the+VPS").await;
    until_page_has(&laptop, &id, "/room/family", "from the VPS").await;
    send(&laptop, &id, "family", "Ming", "from+the+laptop").await;
    until_page_has(&vps, &id, "/room/family", "from the laptop").await;

    // The VPS starts the app again after a restart.
    drop(vps);
    let vps = start(dir_b.path());
    assert!(vps.http_app(&id).is_some());
}
