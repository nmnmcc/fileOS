// SPDX-License-Identifier: AGPL-3.0-only

//! Conditional writes and list pagination through the HTTP handler.

use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

use bytes::Bytes;
use hs_api::Api;
use hs_net::server::{Body, Handler, Peer, Request};
use hs_node::Node;
use http::{Method, StatusCode, header};
use serde_json::{Value, json};

fn peer() -> Peer {
    Peer {
        addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 8080)),
        tls: false,
    }
}

fn request(
    path: &str,
    method: Method,
    token: &str,
    body: Bytes,
    headers: &[(&str, &str)],
) -> Request {
    let mut builder = http::Request::builder()
        .uri(path)
        .method(method)
        .header(header::AUTHORIZATION, format!("Bearer {token}"))
        .header(header::CONTENT_TYPE, "application/json");
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    builder.body(body).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_saves_conflict_and_retry_reuses_the_winning_write() {
    let dir = tempfile::tempdir().unwrap();
    let init = Node::init(dir.path(), "Audit", None).await.unwrap();
    let route = format!(
        "/api/spaces/{}/resources/note/edit",
        init.node.config().private
    );
    let token = init.owner_token;
    let api = Api::new(Arc::new(init.node));
    let original = api
        .handle(
            request(
                &route,
                Method::PUT,
                &token,
                Bytes::from_static(br#"{"text":"base"}"#),
                &[],
            ),
            peer(),
        )
        .await;
    assert_eq!(original.status(), StatusCode::CREATED);
    let tag = original.headers()[header::ETAG].to_str().unwrap();
    let versions = [
        ("first", Bytes::from_static(br#"{"text":"first"}"#)),
        ("second", Bytes::from_static(br#"{"text":"second"}"#)),
    ];
    let tasks = versions.each_ref().map(|(key, text)| {
        let req = request(
            &route,
            Method::PUT,
            &token,
            text.clone(),
            &[("if-match", tag), ("idempotency-key", key)],
        );
        let api = Arc::clone(&api);
        tokio::spawn(async move { api.handle(req, peer()).await })
    });
    let [first, second] = tasks;
    let [first, second] = [first.await.unwrap(), second.await.unwrap()];
    let mut statuses = [first.status(), second.status()];
    statuses.sort_unstable();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::PRECONDITION_FAILED]);
    let winner = usize::from(first.status() != StatusCode::OK);
    let (key, text) = &versions[winner];
    let retry = api
        .handle(
            request(
                &route,
                Method::PUT,
                &token,
                text.clone(),
                &[("if-match", tag), ("idempotency-key", key)],
            ),
            peer(),
        )
        .await;
    assert_eq!(retry.status(), StatusCode::OK);
    let get = api
        .handle(
            request(&route, Method::GET, &token, Bytes::new(), &[]),
            peer(),
        )
        .await;
    let Body::Full(body) = get.into_body() else {
        panic!("expected note body")
    };
    assert_eq!(&body, text);
    assert_eq!(
        api.node
            .entries(&api.node.config().private, None, 10)
            .await
            .unwrap()
            .len(),
        4
    );
}

#[tokio::test]
async fn opt_in_pages_reach_beyond_one_thousand_and_keep_legacy_arrays() {
    let dir = tempfile::tempdir().unwrap();
    let init = Node::init(dir.path(), "Audit", None).await.unwrap();
    let id = init.node.config().private.to_hex();
    let token = init.owner_token;
    let api = Api::new(Arc::new(init.node));
    for seq in 1..=1051 {
        api.node
            .index
            .apply(&hs_store::Change {
                space: &id,
                seq,
                time: 1,
                kind: "put",
                target: &format!("note/{seq:04}"),
                payload: None,
                media_type: "application/json",
                bytes: Some(b"{}"),
            })
            .await
            .unwrap();
    }
    let get = |route: String| {
        let api = Arc::clone(&api);
        let req = request(&route, Method::GET, &token, Bytes::new(), &[]);
        async move {
            let response = api.handle(req, peer()).await;
            assert_eq!(response.status(), StatusCode::OK);
            let Body::Full(body) = response.into_body() else {
                panic!("expected response body")
            };
            serde_json::from_slice::<Value>(&body).unwrap()
        }
    };
    for url in [
        "/api/timeline".to_string(),
        format!("/api/spaces/{id}/resources"),
    ] {
        let first = get(format!("{url}?page=1&limit=1000")).await;
        assert_eq!(first["items"].as_array().unwrap().len(), 1000);
        assert_eq!(first["next_offset"], 1000);
        let last = get(format!("{url}?page=1&limit=1000&offset=1000")).await;
        assert_eq!(last["items"].as_array().unwrap().len(), 51);
        assert_eq!(last["items"][0]["target"], "note/1001");
        assert_eq!(last["items"][50]["target"], "note/1051");
        assert_eq!(last["next_offset"], Value::Null);
        assert_eq!(
            get(format!("{url}?limit=1"))
                .await
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
}

#[tokio::test]
async fn access_logging_resumes_after_an_open_failure() {
    let dir = tempfile::tempdir().unwrap();
    let node = Node::init(dir.path(), "Audit", None).await.unwrap().node;
    let api = Api::new(Arc::new(node));
    let path = dir.path().join("access.log");
    std::fs::create_dir(&path).unwrap();
    assert_eq!(
        api.handle(Request::new(Bytes::new()), peer())
            .await
            .status(),
        StatusCode::OK
    );
    api.flush_access_log().await;
    std::fs::remove_dir(&path).unwrap();

    assert_eq!(
        api.handle(Request::new(Bytes::new()), peer())
            .await
            .status(),
        StatusCode::OK
    );
    api.flush_access_log().await;
    let events = std::fs::read_to_string(&path).unwrap();
    let entries: Vec<Value> = events
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["event.name"], json!("http.request.completed"));
}
