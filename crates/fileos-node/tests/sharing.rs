// SPDX-License-Identifier: AGPL-3.0-only

//! Sharing between apps: the photos app offers one photo, and the person
//! gives it to the chat app. Uses the apps in `target/apps` (run `apps`).

use std::path::Path;
use std::time::Duration;

use fileos_node::Node;

fn png() -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend_from_slice(&4u32.to_be_bytes());
    png.extend_from_slice(&4u32.to_be_bytes());
    png
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_shares_one_file_with_another() {
    let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/apps");
    assert!(apps.join("Chat.app").exists(), "run `apps` first");
    let dir = tempfile::tempdir().unwrap();
    Node::init(dir.path()).unwrap();
    let node = Node::open(dir.path()).unwrap();
    let photos = node.install(&apps.join("Photos.app")).unwrap().id;
    node.install(&apps.join("Chat.app")).unwrap();
    node.put("/photos/cat.png", &png()).unwrap();
    node.put("/photos/dog.png", &png()).unwrap();

    let app = node.http_app(&photos).unwrap();
    let answer = app
        .request("POST", "/share?path=/photos/cat.png", &[], Vec::new())
        .await
        .unwrap();
    assert_eq!(
        answer.status,
        202,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    let request = node.requests().pop().unwrap();
    assert_eq!(request.permissions, "share");
    assert_eq!(request.file.as_deref(), Some("photos/cat.png"));

    // The photos app does not take files from others.
    node.answer(&request.id, "photos").unwrap_err();
    // The chat app does: it gets this one file, at a path of its own.
    let at = node.answer(&request.id, "chat").unwrap();
    assert!(
        at.starts_with("/granted/") && at.ends_with("/cat.png"),
        "{at}"
    );
    let key = format!("shared:{at}");
    for _ in 0..100 {
        if node.kv("chat").contains_key(&key) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(node.kv("chat").contains_key(&key), "{:?}", node.kv("chat"));
    let view: Vec<String> = node.app_view("chat").into_iter().map(|v| v.name).collect();
    assert!(view.iter().any(|v| format!("/{v}") == at), "{view:?}");
    // The grant is on the one file, by its ID, for reading only.
    let grants = node.grants("/photos");
    let on_file: Vec<_> = grants
        .iter()
        .filter_map(|grant| grant.split_once(" on file "))
        .collect();
    assert_eq!(on_file.len(), 1, "{grants:?}");
    assert!(on_file[0].0.ends_with(" files.read"), "{grants:?}");
    assert!(
        on_file[0].1.parse::<fileos_space::Id>().is_ok(),
        "{grants:?}"
    );
}
