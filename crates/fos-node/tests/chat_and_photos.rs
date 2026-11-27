// SPDX-License-Identifier: AGPL-3.0-only

//! The chat and photos apps on two nodes over libp2p, through their HTTP
//! interfaces, with the apps from `target/apps` (run `apps` first). This
//! checks the parts of the design that are built: apps as HTTP servers,
//! streams and files, sync between nodes, change events and `kv`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use fos_node::{NetConfig, Node, parse_permissions};

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
        listen: vec!["/ip4/127.0.0.1/tcp/0".into()],
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

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_and_photos_work_between_two_nodes() {
    let apps = apps();
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    Node::init(dir_a.path()).unwrap();
    Node::init(dir_b.path()).unwrap();
    let alice = start(dir_a.path());
    let bob = start(dir_b.path());
    until("alice listens", || !alice.status().listening.is_empty()).await;
    bob.dial(&alice.status().listening[0]).unwrap();
    until("they connect", || alice.status().peers.contains(&bob.key())).await;

    // Alice makes the spaces and shares them with Bob's node.
    let all = parse_permissions("all").unwrap();
    for at in ["/chat", "/photos"] {
        let id = alice.create(at).unwrap();
        alice.grant(at, bob.key(), &all).unwrap();
        bob.mount(id, at).unwrap();
        until("bob gets the space", || {
            bob.status().spaces.iter().any(|f| f.id == id)
        })
        .await;
    }

    // Both install both apps.
    let a_chat = alice.install(&apps.join("Chat.app")).unwrap().id;
    let b_chat = bob.install(&apps.join("Chat.app")).unwrap().id;
    let a_photos = alice.install(&apps.join("Photos.app")).unwrap().id;
    let b_photos = bob.install(&apps.join("Photos.app")).unwrap().id;

    // Chat: Alice writes, Bob sees it; Bob answers, Alice sees it.
    send(&alice, &a_chat, "family", "Alice", "hello+Bob").await;
    until_page_has(&bob, &b_chat, "/room/family", "hello Bob").await;
    send(&bob, &b_chat, "family", "Bob", "hi+Alice%21").await;
    until_page_has(&alice, &a_chat, "/room/family", "hi Alice!").await;
    let room = page(&alice, &a_chat, "/room/family").await;
    assert!(room.find("hello Bob") < room.find("hi Alice!"), "{room}");
    // Change events keep the count in each node's kv.
    until("alice's app counts the messages", || {
        alice.kv("chat").get("count:family").map(Vec::as_slice) == Some(b"2")
    })
    .await;
    assert!(page(&alice, &a_chat, "/").await.contains("2 messages"));
    // Text is escaped, so a message cannot put HTML into the page.
    send(
        &alice,
        &a_chat,
        "family",
        "Eve",
        "%3Cscript%3Ex%3C%2Fscript%3E",
    )
    .await;
    until_page_has(&bob, &b_chat, "/room/family", "&lt;script&gt;").await;
    assert!(
        !page(&bob, &b_chat, "/room/family")
            .await
            .contains("<script>")
    );

    // Photos: Bob uploads; Alice's app indexes it and gives the same bytes.
    let cat = png(64, 32);
    let answer = bob
        .http_app(&b_photos)
        .unwrap()
        .request("PUT", "/upload/cat.png", &[], cat.clone())
        .await
        .unwrap();
    assert_eq!(answer.status, 201);
    until_page_has(&alice, &a_photos, "/", "64x32").await;
    let photo = alice
        .http_app(&a_photos)
        .unwrap()
        .request("GET", "/photo?path=/photos/cat.png", &[], Vec::new())
        .await
        .unwrap();
    assert_eq!(photo.status, 200);
    assert_eq!(photo.header("content-type"), Some("image/png"));
    assert_eq!(photo.body, cat);
    // Paths outside /photos are not given.
    let outside = alice
        .http_app(&a_photos)
        .unwrap()
        .request("GET", "/photo?path=/chat/family", &[], Vec::new())
        .await
        .unwrap();
    assert_eq!(outside.status, 400);
}
