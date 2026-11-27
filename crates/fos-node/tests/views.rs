// SPDX-License-Identifier: AGPL-3.0-only

//! What an app sees is its own view: mounts at names it knows, a private
//! data space, and grants given at run time. Uses the photos app from
//! `target/apps` (run `apps` first).

use std::path::{Path, PathBuf};
use std::time::Duration;

use fos_node::{Node, parse_permissions};

fn apps() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/apps");
    assert!(
        dir.join("Photos.app").exists(),
        "{} is missing: run `apps` first",
        dir.display()
    );
    dir
}

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend_from_slice(&width.to_be_bytes());
    png.extend_from_slice(&height.to_be_bytes());
    png
}

async fn request(node: &Node, app: &str, method: &str, path: &str, body: Vec<u8>) -> (u16, String) {
    let answer = node
        .http_app(app)
        .unwrap()
        .request(method, path, &[], body)
        .await
        .unwrap();
    (
        answer.status,
        String::from_utf8_lossy(&answer.body).into_owned(),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn an_app_sees_only_its_view() {
    let dir = tempfile::tempdir().unwrap();
    Node::init(dir.path()).unwrap();
    let node = Node::open(dir.path()).unwrap();
    node.create("/pictures").unwrap();
    node.put("/pictures/old.png", &png(10, 20)).unwrap();

    let photos = node.install(&apps().join("Photos.app")).unwrap().id;
    let view: Vec<String> = node
        .app_view("photos")
        .into_iter()
        .map(|v| v.name)
        .collect();
    assert_eq!(view, ["photos"]);

    // The person points the app's /photos at the space at /pictures. The app
    // does not know where its /photos is.
    let at = node
        .app_grant(
            "photos",
            "/pictures",
            &parse_permissions("read-write").unwrap(),
            Some("photos"),
        )
        .unwrap();
    assert_eq!(at, "/photos");
    let (status, _) = request(&node, &photos, "PUT", "/upload/cat.png", png(64, 32)).await;
    assert_eq!(status, 201);
    assert_eq!(node.read("/pictures/cat.png").unwrap(), png(64, 32));
    for _ in 0..100 {
        if node.kv("photos").len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let (_, page) = request(&node, &photos, "GET", "/", Vec::new()).await;
    assert!(page.contains("64x32") && page.contains("10x20"), "{page}");

    // A grant with no name goes to /granted/<handle>.
    node.create("/more").unwrap();
    let at = node
        .app_grant("photos", "/more", &parse_permissions("read").unwrap(), None)
        .unwrap();
    assert!(at.starts_with("/granted/"), "{at}");

    // The private data of the app is not for the node's own key.
    node.put("/.appdata/photos/x", b"x").unwrap_err();

    // Taking the mount back takes the grant back. The app's /photos is now a
    // name in its own private data, so what it writes no longer reaches
    // /pictures.
    node.app_take("photos", "photos").unwrap();
    assert!(node.app_view("photos").iter().all(|v| v.name != "photos"));
    request(&node, &photos, "PUT", "/upload/dog.png", png(1, 1)).await;
    node.read("/pictures/dog.png").unwrap_err();
    assert!(
        node.list("/pictures")
            .unwrap()
            .iter()
            .all(|n| n != "dog.png")
    );
}
