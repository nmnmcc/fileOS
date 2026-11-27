// SPDX-License-Identifier: AGPL-3.0-only

//! Schedules: the kernel sends a request to the app's own handler. Uses the
//! photos app from `target/apps` (run `apps` first).

use std::path::Path;

use fos_node::Node;

#[tokio::test(flavor = "multi_thread")]
async fn the_kernel_runs_the_schedules_of_an_app() {
    let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/apps");
    assert!(apps.join("Photos.app").exists(), "run `apps` first");
    let dir = tempfile::tempdir().unwrap();
    Node::init(dir.path()).unwrap();
    let node = Node::open(dir.path()).unwrap();
    let id = node.install(&apps.join("Photos.app")).unwrap().id;
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend_from_slice(&7u32.to_be_bytes());
    png.extend_from_slice(&9u32.to_be_bytes());
    node.put("/photos/a.png", &png).unwrap();

    assert_eq!(node.run_schedule(&id, "reindex").await.unwrap(), 200);
    assert_eq!(
        node.kv("photos")
            .get("photo:/photos/a.png")
            .map(Vec::as_slice),
        Some(&b"7x9"[..])
    );
    node.run_schedule(&id, "nothing").await.unwrap_err();
    // The same path from outside, with no fos-event header, is not a
    // schedule: the app does not take it.
    let answer = node
        .http_app(&id)
        .unwrap()
        .request("POST", "/.well-known/fos/schedule/reindex", &[], Vec::new())
        .await
        .unwrap();
    assert_ne!(answer.status, 200);
}
