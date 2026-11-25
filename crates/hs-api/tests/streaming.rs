// SPDX-License-Identifier: AGPL-3.0-only

//! Exercise streaming completions and range headers over actual HTTP.

use std::sync::Arc;

use hs_api::Api;
use hs_net::{client::Client, server};
use hs_node::Node;
use http::{Method, Request, StatusCode, header};

async fn serve(node: Node) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(server::serve(
        listener,
        Api::new(Arc::new(node)),
        None,
        std::future::pending(),
    ));
    (url, server)
}

#[tokio::test]
async fn blob_head_ranges_and_integrity_over_http() {
    let dir = tempfile::tempdir().unwrap();
    let init = Node::init(dir.path(), "Test", None).await.unwrap();
    let bytes: Vec<u8> = (0..151_389).map(|n| (n % 251) as u8).collect();
    let hash = init.node.put_blob(&bytes).unwrap();
    let name = hs_objects::blob_name(&hash);
    let blob_file = init.node.blobs.root().join(name);
    let token = init.owner_token;
    let (url, server) = serve(init.node).await;
    let request = |method, range: Option<&str>| {
        let mut builder = Request::builder()
            .method(method)
            .uri(format!("{url}/api/blobs/{hash}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"));
        if let Some(range) = range {
            builder = builder.header(header::RANGE, range);
        }
        builder.body(Vec::new()).unwrap()
    };

    let client = Client::new();
    let head = client.send(request(Method::HEAD, None)).await.unwrap();
    assert_eq!(head.status(), StatusCode::OK);
    assert_eq!(
        head.headers()[header::CONTENT_LENGTH],
        bytes.len().to_string()
    );
    assert_eq!(head.body(), b"");

    let range = "bytes=65530-65545";
    let head = client
        .send(request(Method::HEAD, Some(range)))
        .await
        .unwrap();
    assert_eq!(head.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(head.headers()[header::CONTENT_LENGTH], "16");
    assert_eq!(
        head.headers()[header::CONTENT_RANGE],
        "bytes 65530-65545/151389"
    );
    assert_eq!(head.body(), b"");
    let part = client
        .send(request(Method::GET, Some(range)))
        .await
        .unwrap();
    assert_eq!(part.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(part.body(), &bytes[65530..65546]);
    let invalid = client
        .send(request(Method::GET, Some("bytes=151389-")))
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::RANGE_NOT_SATISFIABLE);
    assert_eq!(invalid.headers()[header::CONTENT_RANGE], "bytes */151389");

    let full = client.send(request(Method::GET, None)).await.unwrap();
    assert_eq!(full.status(), StatusCode::OK);
    assert_eq!(full.body(), &bytes);
    let mut corrupt = bytes;
    corrupt[0] ^= 1;
    std::fs::write(blob_file, corrupt).unwrap();
    client.send(request(Method::GET, None)).await.unwrap_err();
    server.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn export_read_failure_is_reported_to_the_downloader() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().unwrap();
    let init = Node::init(dir.path(), "Test", None).await.unwrap();
    let file = dir.path().join("config.json");
    let token = init.owner_token;
    let (url, server) = serve(init.node).await;
    let client = hs_client::Client::new(url, token, false);

    let mut bytes = Vec::new();
    let parts = client.download("/api/export", &mut bytes).await.unwrap();
    assert_eq!(parts.status, StatusCode::OK);
    assert_ne!(bytes.as_slice(), b"");

    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
    if let Err(error) = std::fs::read(&file) {
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        let error = hs_api::export::archive(dir.path(), Vec::new()).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        let download = client
            .download("/api/export", &mut Vec::new())
            .await
            .unwrap_err();
        assert!(std::error::Error::source(&download).is_some());
    }
    server.abort();
}

#[tokio::test]
async fn homeserver_replica_publishes_a_new_manifest_and_restores_exact_bytes() {
    use hs_api::replica;
    use hs_objects::crypt::StorageKey;

    let host = tempfile::tempdir().unwrap();
    let init = Node::init(host.path(), "Backup host", None).await.unwrap();
    init.node.set_config(
        "s3_keys",
        serde_json::json!([{"access_key":"audit", "secret_key":"synthetic", "bucket":"backup"}]),
    ).unwrap();
    let (url, task) = serve(init.node).await;
    let address = url.strip_prefix("http://").unwrap();
    let store = replica::Store::parse(&format!(
        "hs://audit:synthetic@{address}/backup/client?http=1"
    ))
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("live");
    std::fs::create_dir(&source).unwrap();
    let file = source.join("config.json");
    let seed = [32; 32];
    let key = StorageKey::new(seed);
    std::fs::write(&file, b"first").unwrap();
    let first = replica::sync(&source, &key, &store).await.unwrap();
    assert_eq!((first.sent, first.deleted), (1, 0));
    std::fs::write(&file, b"second").unwrap();
    let second = replica::sync(&source, &key, &store).await.unwrap();
    assert_eq!((second.sent, second.deleted), (1, 1));
    let dest = dir.path().join("restored");
    assert_eq!(replica::restore(&store, seed, &dest).await.unwrap(), 1);
    assert_eq!(std::fs::read(dest.join("config.json")).unwrap(), b"second");
    task.abort();
}
