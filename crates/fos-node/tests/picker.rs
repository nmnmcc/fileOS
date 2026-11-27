// SPDX-License-Identifier: AGPL-3.0-only

//! The picker: an app asks for access, and the person picks what it gets.
//! Uses the photos app from `target/apps` (run `apps` first).

use std::path::Path;
use std::time::Duration;

use fos_node::Node;

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_asks_and_the_person_picks() {
    let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/apps");
    assert!(apps.join("Photos.app").exists(), "run `apps` first");
    let dir = tempfile::tempdir().unwrap();
    Node::init(dir.path()).unwrap();
    let node = Node::open(dir.path()).unwrap();
    node.create("/holiday").unwrap();
    node.put("/holiday/beach.png", &png(30, 20)).unwrap();
    let photos = node.install(&apps.join("Photos.app")).unwrap().id;
    let app = node.http_app(&photos).unwrap();

    // The app asks. It does not say where, and gets nothing yet.
    let answer = app
        .request("POST", "/import", &[], Vec::new())
        .await
        .unwrap();
    assert_eq!(answer.status, 202);
    let requests = node.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].app, "photos");
    assert_eq!(requests[0].permissions, "read");
    assert!(node.list("/").unwrap().iter().all(|n| !n.starts_with('.')));

    // The person picks /holiday. The app sees it at /granted/<handle> only.
    let at = node.answer(&requests[0].id, "/holiday").unwrap();
    assert!(at.starts_with("/granted/"), "{at}");
    assert_eq!(node.requests(), Vec::new());
    let key = format!("photo:{at}/beach.png");
    for _ in 0..100 {
        if node.kv("photos").contains_key(&key) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        node.kv("photos").get(&key).map(Vec::as_slice),
        Some(&b"30x20"[..])
    );
    let image = app
        .request(
            "GET",
            &format!("/photo?path={at}/beach.png"),
            &[],
            Vec::new(),
        )
        .await
        .unwrap();
    assert_eq!(image.body, png(30, 20));
    // It may read, not write: the person gave what the app asked for.
    let answer = app
        .request("PUT", "/upload/x.png", &[], png(1, 1))
        .await
        .unwrap();
    assert_eq!(answer.status, 201, "uploads still go to its own /photos");
    node.read("/holiday/x.png").unwrap_err();

    // A request that the person refuses goes away.
    app.request("POST", "/import", &[], Vec::new())
        .await
        .unwrap();
    let id = node.requests()[0].id.clone();
    node.refuse(&id).unwrap();
    assert_eq!(node.requests(), Vec::new());
    node.answer(&id, "/holiday").unwrap_err();
}
