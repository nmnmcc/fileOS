// SPDX-License-Identifier: AGPL-3.0-only

//! Apps as HTTP servers, with `wasi:http`.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use fileos_core::Error;
use http_body_util::BodyExt as _;
use hyper::body::{Body as HttpBody, Frame, SizeHint};
use wasmtime::Store;
use wasmtime::component::Linker;
use wasmtime_wasi_http::WasiHttpView as _;
use wasmtime_wasi_http::p2::bindings::ProxyPre;
use wasmtime_wasi_http::p2::bindings::http::types::Scheme;
use wasmtime_wasi_http::p2::body::HyperOutgoingBody;

use crate::{APP_FUEL, AppCtx, AppHost, Bundle, FsError, Host, Kind, Runtime, wasm_error};

mod imports {
    wasmtime::component::bindgen!({
        path: "../../wit",
        world: "app",
        require_store_data_send: true,
        imports: { default: async | trappable },
        with: {
            "wasi:http": wasmtime_wasi_http::p2::bindings::http,
            "wasi:io": wasmtime_wasi::p2::bindings::io,
            "wasi:clocks": wasmtime_wasi::p2::bindings::clocks,
        },
    });
}

use imports::fileos::app::{access, fs, kv, log};
use wasmtime::component::HasSelf;

fn add_host(linker: &mut Linker<AppCtx>) -> Result<(), Error> {
    fs::add_to_linker::<AppCtx, HasSelf<AppCtx>>(linker, |c| c).map_err(wasm_error)?;
    kv::add_to_linker::<AppCtx, HasSelf<AppCtx>>(linker, |c| c).map_err(wasm_error)?;
    log::add_to_linker::<AppCtx, HasSelf<AppCtx>>(linker, |c| c).map_err(wasm_error)?;
    access::add_to_linker::<AppCtx, HasSelf<AppCtx>>(linker, |c| c).map_err(wasm_error)
}

impl AppCtx {
    fn blocking<T, F>(&self, call: F) -> impl Future<Output = wasmtime::Result<T>> + use<T, F>
    where
        T: Send + 'static,
        F: FnOnce(&dyn AppHost, &str) -> T + Send + 'static,
    {
        let host = self.host.clone();
        let app = self.app.clone();
        let workers = Arc::clone(&self.host_workers);
        async move {
            let worker = workers
                .acquire_owned()
                .await
                .map_err(wasmtime::Error::new)?;
            tokio::task::spawn_blocking(move || {
                let _worker = worker;
                call(host.0.as_ref(), &app)
            })
            .await
            .map_err(wasmtime::Error::new)
        }
    }
}

fn to_wit(error: FsError) -> fs::Error {
    match error {
        FsError::Denied(message) => fs::Error::Denied(message),
        FsError::NotFound => fs::Error::NotFound,
        FsError::Invalid(message) | FsError::Unavailable(message) => fs::Error::Invalid(message),
    }
}

impl fs::Host for AppCtx {
    async fn read(&mut self, path: String) -> wasmtime::Result<Result<Vec<u8>, fs::Error>> {
        let result = self
            .blocking(move |host, app| host.read(app, &path))
            .await?;
        Ok(result.map_err(to_wit))
    }

    async fn write(
        &mut self,
        path: String,
        data: Vec<u8>,
    ) -> wasmtime::Result<Result<(), fs::Error>> {
        let result = self
            .blocking(move |host, app| host.write(app, &path, data))
            .await?;
        Ok(result.map_err(to_wit))
    }

    async fn append(
        &mut self,
        path: String,
        data: Vec<u8>,
    ) -> wasmtime::Result<Result<(), fs::Error>> {
        let result = self
            .blocking(move |host, app| host.append(app, &path, data))
            .await?;
        Ok(result.map_err(to_wit))
    }

    async fn delete(&mut self, path: String) -> wasmtime::Result<Result<(), fs::Error>> {
        let result = self
            .blocking(move |host, app| host.delete(app, &path))
            .await?;
        Ok(result.map_err(to_wit))
    }

    async fn list_dir(&mut self, path: String) -> wasmtime::Result<Result<Vec<String>, fs::Error>> {
        let result = self
            .blocking(move |host, app| host.list(app, &path))
            .await?;
        Ok(result.map_err(to_wit))
    }
}

impl kv::Host for AppCtx {
    async fn get(&mut self, key: String) -> wasmtime::Result<Option<Vec<u8>>> {
        self.blocking(move |host, app| host.kv_get(app, &key)).await
    }

    async fn set(&mut self, key: String, value: Vec<u8>) -> wasmtime::Result<()> {
        self.blocking(move |host, app| host.kv_set(app, &key, value))
            .await
    }

    async fn delete(&mut self, key: String) -> wasmtime::Result<()> {
        self.blocking(move |host, app| host.kv_delete(app, &key))
            .await
    }

    async fn keys(&mut self, prefix: String) -> wasmtime::Result<Vec<String>> {
        self.blocking(move |host, app| host.kv_keys(app, &prefix))
            .await
    }
}

impl log::Host for AppCtx {
    async fn info(&mut self, message: String) -> wasmtime::Result<()> {
        self.blocking(move |host, app| host.log(app, &message))
            .await
    }
}

impl access::Host for AppCtx {
    async fn ask(
        &mut self,
        purpose: String,
        permissions: String,
    ) -> wasmtime::Result<Result<String, String>> {
        self.blocking(move |host, app| host.ask(app, &purpose, &permissions))
            .await
    }

    async fn share(&mut self, path: String) -> wasmtime::Result<Result<String, String>> {
        self.blocking(move |host, app| host.share(app, &path)).await
    }
}

/// The most time one request may take.
const REQUEST_TIME: Duration = Duration::from_secs(30);

/// How much fuel an app may use before it lets other work run. With this,
/// the time limit works even when an app does not stop.
const FUEL_SLICE: u64 = 1_000_000;

/// The stream of bytes and trailers from an app.
pub struct Body(HyperOutgoingBody);

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Body")
    }
}

impl HttpBody for Body {
    type Data = Bytes;
    type Error = Error;

    fn poll_frame(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Error>>> {
        Pin::new(&mut self.get_mut().0).poll_frame(cx).map(|frame| {
            frame.map(|frame| frame.map_err(|e| Error::internal(e.to_string()).with_source(e)))
        })
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.0.size_hint()
    }
}

/// A whole HTTP response, with all of its body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Answer {
    pub status: u16,
    /// The headers, with names in lowercase.
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: Vec<u8>,
}

impl Answer {
    /// The first header with this name, as text.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .and_then(|(_, v)| std::str::from_utf8(v).ok())
    }
}

/// An app, ready to take HTTP requests for its origin.
#[derive(Clone)]
pub struct HttpApp {
    id: String,
    host: Host,
    requests: Arc<tokio::sync::Semaphore>,
    host_workers: Arc<tokio::sync::Semaphore>,
    pre: ProxyPre<AppCtx>,
}

impl std::fmt::Debug for HttpApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "HttpApp({})", self.id)
    }
}

impl Runtime {
    /// Makes an app ready for HTTP requests. Its calls go to `host`.
    ///
    /// # Errors
    ///
    /// When the bundle is not an app, or its component does not export
    /// `wasi:http/incoming-handler`.
    pub fn http_app(&self, bundle: &Bundle, host: Host) -> Result<HttpApp, Error> {
        let component = self.component(bundle, Kind::App)?;
        let mut linker = Linker::new(&self.engine);
        add_host(&mut linker)?;
        wasmtime_wasi_http::p2::add_to_linker_async(&mut linker).map_err(wasm_error)?;
        let pre = ProxyPre::new(linker.instantiate_pre(&component).map_err(wasm_error)?)
            .map_err(wasm_error)?;
        Ok(HttpApp {
            id: bundle.manifest.id.clone(),
            host,
            requests: Arc::clone(&self.requests),
            host_workers: Arc::clone(&self.host_workers),
            pre,
        })
    }
}

impl HttpApp {
    /// Gives one request, made of bytes, to the app, and gives back all of
    /// its response. Tests use it, and so can a node that sends a request on
    /// for another node.
    ///
    /// # Errors
    ///
    /// When the request is not valid, or as [`HttpApp::handle`].
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        headers: &[(&str, &str)],
        body: Vec<u8>,
    ) -> Result<Answer, Error> {
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(format!("http://{}.localhost{path}", self.id));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let body = http_body_util::Full::new(Bytes::from(body));
        let request = request
            .body(body)
            .map_err(|e| Error::invalid(format!("not a good request: {e}")))?;
        let response = self.handle(request).await?;
        let status = response.status().as_u16();
        let headers = response
            .headers()
            .iter()
            .map(|(n, v)| (n.as_str().to_string(), v.as_bytes().to_vec()))
            .collect();
        let body = response
            .into_body()
            .collect()
            .await
            .map_err(|e| Error::invalid(format!("the app {} failed in its body: {e}", self.id)))?
            .to_bytes()
            .to_vec();
        Ok(Answer {
            status,
            headers,
            body,
        })
    }

    /// Gives one request to a new instance of the app, and gives back its
    /// response. Call it inside a Tokio runtime.
    ///
    /// # Errors
    ///
    /// When the app traps, runs out of fuel, memory or time, or gives no
    /// response.
    pub async fn handle<B>(
        &self,
        request: hyper::Request<B>,
    ) -> Result<hyper::Response<Body>, Error>
    where
        B: HttpBody<Data = Bytes> + Send + 'static,
        B::Error: std::error::Error,
    {
        let slot = Arc::clone(&self.requests)
            .try_acquire_owned()
            .map_err(|e| Error::unavailable("the app runtime is busy".into()).with_source(e))?;
        let mut store = Store::new(
            self.pre.engine(),
            AppCtx::new(&self.id, self.host.clone(), Arc::clone(&self.host_workers)),
        );
        store.limiter(|ctx| &mut ctx.limits);
        store.set_fuel(APP_FUEL).map_err(wasm_error)?;
        store
            .fuel_async_yield_interval(Some(FUEL_SLICE))
            .map_err(wasm_error)?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
        let request = request.map(|body| {
            body.map_err(|error| wasmtime_wasi_http::Error::InternalError(Some(error.to_string())))
        });
        let req = store
            .data_mut()
            .http()
            .new_incoming_request(Scheme::Http, request)
            .map_err(wasm_error)?;
        let out = store
            .data_mut()
            .http()
            .new_response_outparam(sender)
            .map_err(wasm_error)?;
        let pre = self.pre.clone();
        let task = tokio::spawn(tokio::time::timeout(REQUEST_TIME, async move {
            let _slot = slot;
            let proxy = pre.instantiate_async(&mut store).await?;
            proxy
                .wasi_http_incoming_handler()
                .call_handle(&mut store, req, out)
                .await
        }));
        match receiver.await {
            Ok(Ok(response)) => Ok(response.map(Body)),
            Ok(Err(e)) => Err(Error::invalid(format!("the app {} failed: {e:?}", self.id))),
            Err(_) => Err(match task.await {
                Ok(Ok(Ok(()))) => Error::invalid(format!("the app {} gave no response", self.id)),
                Ok(Ok(Err(e))) => Error::invalid(format!("the app {} failed: {e:#}", self.id)),
                Ok(Err(_)) => Error::invalid(format!("the app {} took too long", self.id)),
                Err(e) => Error::invalid(format!("the app {} failed: {e}", self.id)),
            }),
        }
    }
}
