// SPDX-License-Identifier: AGPL-3.0-only

//! Check the generated REST contract and its distribution copy.

use std::collections::BTreeMap;

#[test]
fn the_committed_document_is_current() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/openapi.json");
    let built = hs_api::openapi::json();
    if std::env::var_os("HS_UPDATE_OPENAPI").is_some() {
        std::fs::write(path, &built).expect("write docs/openapi.json");
        return;
    }
    let kept = std::fs::read(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
    assert!(
        kept == built,
        "docs/openapi.json is out of date; run `HS_UPDATE_OPENAPI=1 cargo test -p hs-api --test openapi`"
    );
}

#[test]
fn describes_all_rest_operations() {
    // Federation, S3, DAV, MCP, identity-provider endpoints and website routes
    // use separate protocols outside this REST contract.
    let routes = BTreeMap::from([
        ("/api/blobs", &["post"][..]),
        ("/api/blobs/{hash}", &["get", "head"]),
        ("/api/events", &["get"]),
        ("/api/export", &["get"]),
        ("/api/grants", &["get", "post"]),
        ("/api/grants/{grant}", &["delete"]),
        ("/api/health", &["get"]),
        ("/api/identity", &["get"]),
        ("/api/near", &["get"]),
        ("/api/node", &["get"]),
        ("/api/openapi.json", &["get"]),
        ("/api/push/key", &["get"]),
        ("/api/push/subscribe", &["post"]),
        ("/api/push/test", &["post"]),
        ("/api/related/{space}/{type}/{id}", &["get"]),
        ("/api/replicas", &["get"]),
        ("/api/replicas/sync", &["post"]),
        ("/api/search", &["get"]),
        ("/api/session", &["delete", "get", "post"]),
        ("/api/spaces", &["get", "post"]),
        ("/api/spaces/join", &["post"]),
        ("/api/spaces/{space}", &["get"]),
        ("/api/spaces/{space}/erase/{type}/{id}", &["post"]),
        ("/api/spaces/{space}/handover", &["post"]),
        ("/api/spaces/{space}/members", &["post"]),
        ("/api/spaces/{space}/members/{node}", &["delete"]),
        ("/api/spaces/{space}/resources", &["get"]),
        (
            "/api/spaces/{space}/resources/{type}/{id}",
            &["delete", "get", "head", "patch", "put"],
        ),
        ("/api/spaces/{space}/sync", &["post"]),
        ("/api/timeline", &["get"]),
    ]);
    let document: serde_json::Value = serde_json::from_slice(&hs_api::openapi::json()).unwrap();
    let paths = document["paths"].as_object().unwrap();
    let described: BTreeMap<&str, Vec<&str>> = paths
        .iter()
        .map(|(path, operations)| {
            let mut methods: Vec<&str> = operations
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            methods.sort_unstable();
            (path.as_str(), methods)
        })
        .collect();
    let expected: BTreeMap<_, _> = routes
        .into_iter()
        .map(|(path, methods)| (path, methods.to_vec()))
        .collect();
    assert_eq!(described, expected);
}
