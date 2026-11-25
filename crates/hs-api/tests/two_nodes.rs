// SPDX-License-Identifier: AGPL-3.0-only

//! Two nodes on loopback share a space: one creates it and adds the other,
//! the other joins, and writes made on either side reach both.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use hs_api::Api;
use hs_net::client::Client;
use hs_node::Node;
use serde_json::{Value as Json, json};
use tokio::net::TcpListener;

struct Running {
    url: String,
    token: String,
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("hs-two-nodes-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

async fn start(name: &str) -> Running {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let done = Node::init(&tmp(name), name, Some(&url)).await.unwrap();
    let api = Api::new(Arc::new(done.node));
    api.start();
    tokio::spawn(hs_net::server::serve(
        listener,
        api,
        None,
        std::future::pending(),
    ));
    Running {
        url,
        token: done.owner_token,
    }
}

async fn call(n: &Running, method: &str, path: &str, body: Option<Json>) -> (u16, Json) {
    call_with(n, method, path, body, &[]).await
}

async fn call_with(
    n: &Running,
    method: &str,
    path: &str,
    body: Option<Json>,
    headers: &[(&str, &str)],
) -> (u16, Json) {
    let mut b = http::Request::builder()
        .method(method)
        .uri(format!("{}{path}", n.url))
        .header("authorization", format!("Bearer {}", n.token));
    if body.is_some() {
        b = b.header("content-type", "application/json");
    }
    for (name, value) in headers {
        b = b.header(*name, *value);
    }
    let req = b
        .body(
            body.map(|v| serde_json::to_vec(&v).unwrap())
                .unwrap_or_default(),
        )
        .unwrap();
    let resp = Client::new().send(req).await.unwrap();
    let status = resp.status().as_u16();
    (
        status,
        serde_json::from_slice(resp.body()).unwrap_or(Json::Null),
    )
}

/// Poll until `path` answers 200, for up to five seconds.
async fn eventually(n: &Running, path: &str) -> Json {
    for _ in 0..50 {
        let (s, v) = call(n, "GET", path, None).await;
        if s == 200 {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("{path} never appeared on {}", n.url);
}

#[tokio::test(flavor = "multi_thread")]
async fn share_a_space() {
    let a = start("a").await;
    let b = start("b").await;

    let (s, space) = call(&a, "POST", "/api/spaces", Some(json!({ "name": "family" }))).await;
    assert_eq!(s, 201, "{space}");
    let id = space["id"].as_str().unwrap().to_string();

    let (s, m) = call(
        &a,
        "POST",
        "/api/spaces/family/members",
        Some(json!({ "url": b.url, "role": "writer" })),
    )
    .await;
    assert_eq!(s, 201, "{m}");

    let (s, joined) = call(
        &b,
        "POST",
        "/api/spaces/join",
        Some(json!({ "home": a.url, "space": id })),
    )
    .await;
    assert_eq!(s, 201, "{joined}");

    // A writes; it is pushed to B.
    let (s, w) = call(
        &a,
        "PUT",
        "/api/spaces/family/resources/note/from-a",
        Some(json!({ "title": "from A" })),
    )
    .await;
    assert!(s == 200 || s == 201, "{s} {w}");
    let got = eventually(&b, &format!("/api/spaces/{id}/resources/note/from-a")).await;
    assert_eq!(got["title"], "from A", "{got}");

    // B writes; it is forwarded to A, the primary, and ordered there.
    let (s, w) = call(
        &b,
        "PUT",
        &format!("/api/spaces/{id}/resources/note/from-b"),
        Some(json!({ "title": "from B" })),
    )
    .await;
    assert!(s == 200 || s == 201, "{s} {w}");
    let got = eventually(&a, "/api/spaces/family/resources/note/from-b").await;
    assert_eq!(got["title"], "from B", "{got}");
    let got = eventually(&b, &format!("/api/spaces/{id}/resources/note/from-b")).await;
    assert_eq!(got["title"], "from B", "{got}");

    // Two clients use the same version, including one writing through the follower.
    let tag = w["etag"].as_str().unwrap();
    let condition = [("if-match", tag)];
    let path = format!("/api/spaces/{id}/resources/note/from-b");
    let (one, two) = tokio::join!(
        call_with(
            &a,
            "PUT",
            &path,
            Some(json!({ "title": "A's edit" })),
            &condition
        ),
        call_with(
            &b,
            "PUT",
            &path,
            Some(json!({ "title": "B's edit" })),
            &condition
        )
    );
    let mut statuses = [one.0, two.0];
    statuses.sort_unstable();
    assert_eq!(statuses, [200, 412], "{one:?} {two:?}");
    let winner = if one.0 == 200 { "A's edit" } else { "B's edit" };
    assert_eq!(eventually(&a, &path).await["title"], winner);

    let _ = std::fs::remove_dir_all(tmp("a"));
    let _ = std::fs::remove_dir_all(tmp("b"));
}
