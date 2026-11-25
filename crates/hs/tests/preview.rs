// SPDX-License-Identifier: AGPL-3.0-only

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use hs_api::Api;
use hs_net::server::{Body, Handler as _, Peer};
use hs_node::Node;
use http::{StatusCode, header};

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x03, 0x00, 0x00, 0x00, 0x02, 0x08, 0x02, 0x00, 0x00, 0x00, 0x12, 0x16, 0xf1,
    0x4d, 0x00, 0x00, 0x00, 0x11, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xcf, 0xc0, 0x00,
    0x41, 0x0c, 0x30, 0xc6, 0x7f, 0x00, 0x3b, 0xd8, 0x05, 0xfb, 0x39, 0x4d, 0x96, 0xa5, 0x00, 0x00,
    0x00, 0x00, 0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

#[tokio::test]
async fn serves_thumbnails_through_the_binary_worker() {
    let dir = tempfile::tempdir().unwrap();
    let initialized = Node::init(dir.path(), "Test", None).await.unwrap();
    let node = Arc::new(initialized.node);
    let valid = node.put_blob(PNG).unwrap();
    let invalid = node.put_blob(b"unknown format").unwrap();
    let peer = Peer {
        addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 8080)),
        tls: false,
    };
    let request = |hash| {
        http::Request::builder()
            .uri(format!("/api/blobs/{hash}?thumb=32"))
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", initialized.owner_token),
            )
            .body(Bytes::new())
            .unwrap()
    };

    let broken_api =
        Api::with_thumbnail_worker(Arc::clone(&node), dir.path().join("missing-worker"));
    assert_eq!(
        broken_api.handle(request(valid), peer).await.status(),
        StatusCode::BAD_GATEWAY
    );
    broken_api.flush_access_log().await;
    let api = Api::with_thumbnail_worker(node, PathBuf::from(env!("CARGO_BIN_EXE_hs")));
    assert_eq!(
        api.handle(request(invalid), peer).await.status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    let response = api.handle(request(valid), peer).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "image/jpeg");
    let Body::Full(bytes) = response.into_body() else {
        panic!("expected a complete thumbnail")
    };
    let preview = hs_media::probe(&bytes).unwrap();
    assert_eq!((preview.width, preview.height), (3, 2));

    api.flush_access_log().await;
    let access = std::fs::read_to_string(dir.path().join("access.log")).unwrap();
    let events: Vec<serde_json::Value> = access
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(events.len(), 3);
    assert!(events.iter().all(|event| {
        event["http.route"] == "/api" && event["event.name"] == "http.request.completed"
    }));
    assert!(!access.contains(&valid.to_hex()));
    assert!(!access.contains(&initialized.owner_token));
    assert!(!access.contains("127.0.0.1"));
}
