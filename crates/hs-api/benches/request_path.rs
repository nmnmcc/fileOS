// SPDX-License-Identifier: AGPL-3.0-only

//! Measure handler throughput for a small request with access logging enabled.

use std::hint::black_box;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use hs_api::Api;
use hs_net::server::{Handler as _, Peer, Request};
use hs_node::Node;
use http::StatusCode;

fn main() {
    let iterations = std::env::var("HS_BENCH_REQUESTS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok().filter(|count| *count > 0))
        .unwrap_or(100_000);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let node = Node::init(temp.path(), "Benchmark", None)
            .await
            .unwrap()
            .node;
        let api = Api::new(Arc::new(node));
        let peer = Peer {
            addr: SocketAddr::from((Ipv4Addr::LOCALHOST, 8080)),
            tls: false,
        };
        let response = api.handle(Request::new(Bytes::new()), peer).await;
        assert_eq!(response.status(), StatusCode::OK);
        let start = Instant::now();
        for _ in 0..iterations {
            black_box(api.handle(Request::new(Bytes::new()), peer).await);
        }
        api.flush_access_log().await;
        println!(
            "GET / with access logging: {:.0} handlers/s over {iterations} requests",
            f64::from(iterations) / start.elapsed().as_secs_f64()
        );
    });
}
