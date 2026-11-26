// SPDX-License-Identifier: AGPL-3.0-only

use std::sync::Mutex;

use super::*;

/// The .app folders that `apps` (scripts/apps.py) builds.
fn bundle(name: &str) -> Bundle {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/apps")
        .join(name);
    assert!(
        dir.exists(),
        "{} is missing: run `apps` first",
        dir.display()
    );
    Bundle::read_dir(&dir).unwrap()
}

#[derive(Default)]
struct Host {
    files: BTreeMap<String, Vec<u8>>,
    kv: Mutex<BTreeMap<String, Vec<u8>>>,
}

impl AppHost for Host {
    fn read(&self, _: &str, path: &str) -> Result<Vec<u8>, FsError> {
        self.files.get(path).cloned().ok_or(FsError::NotFound)
    }
    fn write(&self, _: &str, _: &str, _: Vec<u8>) -> Result<(), FsError> {
        Err(FsError::Denied("read only".into()))
    }
    fn append(&self, _: &str, _: &str, _: Vec<u8>) -> Result<(), FsError> {
        Err(FsError::Denied("read only".into()))
    }
    fn delete(&self, _: &str, _: &str) -> Result<(), FsError> {
        Err(FsError::Denied("read only".into()))
    }
    fn list(&self, _: &str, _: &str) -> Result<Vec<String>, FsError> {
        Ok(Vec::new())
    }
    fn kv_get(&self, _: &str, key: &str) -> Option<Vec<u8>> {
        self.kv.lock().unwrap().get(key).cloned()
    }
    fn kv_set(&self, _: &str, key: &str, value: Vec<u8>) {
        self.kv.lock().unwrap().insert(key.into(), value);
    }
    fn kv_delete(&self, _: &str, key: &str) {
        self.kv.lock().unwrap().remove(key);
    }
    fn kv_keys(&self, _: &str, prefix: &str) -> Vec<String> {
        self.kv
            .lock()
            .unwrap()
            .keys()
            .filter(|k| k.starts_with(prefix))
            .cloned()
            .collect()
    }
    fn log(&self, _: &str, _: &str) {}
}

#[test]
fn the_photos_app_indexes_a_photo() {
    let runtime = Runtime::new().unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
    png.extend_from_slice(&64u32.to_be_bytes());
    png.extend_from_slice(&32u32.to_be_bytes());
    let mut host = Host::default();
    host.files.insert("/photos/a.png".into(), png);
    let host = Arc::new(host);

    let mut app = runtime
        .app(&bundle("Photos.app"), Arc::<Host>::clone(&host))
        .unwrap();
    assert_eq!(app.manifest().name, "Photos");
    assert!(app.watches("/photos/a.png"));
    assert!(!app.watches("/photosx"));
    app.on_change("/photos/a.png").unwrap();
    assert_eq!(
        host.kv_get("photos", "photo:/photos/a.png").as_deref(),
        Some(&b"64x32"[..])
    );
    // A deleted file leaves the index.
    app.on_change("/photos/b.png").unwrap();
    assert_eq!(host.kv_keys("photos", "photo:"), ["photo:/photos/a.png"]);
}

/// A request with no body, for tests.
fn get(
    path: &str,
    user: Option<&str>,
) -> hyper::Request<
    impl hyper::body::Body<Data = bytes::Bytes, Error = wasmtime_wasi_http::Error> + Send + 'static,
> {
    use http_body_util::BodyExt as _;
    let mut request = hyper::Request::builder()
        .uri(format!("http://photos-test.localhost{path}"))
        .header("host", "photos-test.localhost");
    if let Some(user) = user {
        request = request.header("fos-user", user);
    }
    let body = http_body_util::Full::new(bytes::Bytes::new())
        .map_err(|never: std::convert::Infallible| -> wasmtime_wasi_http::Error { match never {} });
    request.body(body).unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn the_photos_app_answers_http() {
    use http_body_util::BodyExt as _;
    let runtime = Runtime::new().unwrap();
    let host = Arc::new(Host::default());
    host.kv_set("photos", "photo:/photos/a.png", b"64x32".to_vec());
    let app = runtime
        .http_app(&bundle("Photos.app"), Arc::<Host>::clone(&host))
        .unwrap();

    let response = app.handle(get("/", Some("did:fos:abc"))).await.unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(
        response.headers()["content-type"],
        "text/html; charset=utf-8"
    );
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let page = String::from_utf8_lossy(&body);
    assert!(page.contains("/photos/a.png"), "{page}");
    assert!(page.contains("64x32"), "{page}");
    assert!(page.contains("did:fos:abc"), "{page}");

    // Each request has a new instance; a second one works the same.
    let response = app.handle(get("/nothing", None)).await.unwrap();
    assert_eq!(response.status(), 404);
}
