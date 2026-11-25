// SPDX-License-Identifier: AGPL-3.0-only

//! Measure small-request throughput and concurrent blob and log-write latency.

use std::hint::black_box;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use bytes::Bytes;
use hs_api::Api;
use hs_net::client::Client;
use hs_net::server::{self, Handler as _, Peer, Request};
use hs_node::Node;
use http::{Method, StatusCode, header};

fn main() {
    let iterations = std::env::var("HS_BENCH_REQUESTS")
        .ok()
        .and_then(|value| value.parse::<u32>().ok().filter(|count| *count > 0))
        .unwrap_or(100_000);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async {
        let temp = tempfile::tempdir().unwrap();
        let init = Node::init(temp.path(), "Benchmark", None).await.unwrap();
        let api = Api::new(Arc::new(init.node));
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
        mixed(api, &init.owner_token, (iterations / 1000).max(8)).await;
    });
}

async fn mixed(api: Arc<Api>, token: &str, samples: u32) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let hash = api
        .node
        .put_blob_async(vec![42u8; 4 * 1024 * 1024])
        .await
        .unwrap();
    let blob = format!("{url}/api/blobs/{hash}");
    let space = api.node.config().private;
    let listener = tokio::spawn(server::serve(listener, api, None, std::future::pending()));
    let client = Client::new();
    let auth = format!("Bearer {token}");
    let mut reads = Vec::with_capacity(samples as usize);
    let mut writes = Vec::with_capacity(samples as usize);
    let started = Instant::now();
    for offset in (0..samples).step_by(4) {
        let mut tasks = tokio::task::JoinSet::new();
        for sample in offset..(offset + 4).min(samples) {
            let read = http::Request::builder()
                .uri(&blob)
                .header(header::RANGE, "bytes=2097152-2097407")
                .header(header::AUTHORIZATION, &auth)
                .body(Vec::new())
                .unwrap();
            let write = http::Request::builder()
                .method(Method::PUT)
                .uri(format!(
                    "{url}/api/spaces/{space}/resources/note/bench-{sample}"
                ))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, &auth)
                .body(br#"{"title":"benchmark"}"#.to_vec())
                .unwrap();
            let reader = client.clone();
            let writer = client.clone();
            tasks.spawn(async move {
                let start = Instant::now();
                let response = reader.send(read).await.unwrap();
                assert_eq!(response.status(), StatusCode::PARTIAL_CONTENT);
                assert_eq!(response.body().len(), 256);
                (true, start.elapsed())
            });
            tasks.spawn(async move {
                let start = Instant::now();
                let response = writer.send(write).await.unwrap();
                assert_eq!(response.status(), StatusCode::CREATED);
                (false, start.elapsed())
            });
        }
        while let Some(result) = tasks.join_next().await {
            let (read, latency) = result.unwrap();
            if read {
                reads.push(latency);
            } else {
                writes.push(latency);
            }
        }
    }
    let elapsed = started.elapsed();
    println!(
        "Concurrent range GET and resource PUT: {:.0} requests/s",
        f64::from(samples) * 2.0 / elapsed.as_secs_f64()
    );
    percentile("range GET", &mut reads);
    percentile("resource PUT", &mut writes);
    listener.abort();
}

fn percentile(label: &str, durations: &mut [Duration]) {
    durations.sort_unstable();
    let p99 = durations[(durations.len() * 99).div_ceil(100) - 1];
    println!("{label}: p99 {p99:?} over {} requests", durations.len());
}
