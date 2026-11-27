// SPDX-License-Identifier: AGPL-3.0-only

//! Apps as HTTP servers, with `wasi:http`.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use fileos_core::Error;
use wasmtime::Store;
use wasmtime::component::Linker;
use wasmtime_wasi_http::WasiHttpView as _;
use wasmtime_wasi_http::p2::bindings::ProxyPre;
use wasmtime_wasi_http::p2::bindings::http::types::Scheme;
use wasmtime_wasi_http::p2::body::HyperOutgoingBody;

use crate::{APP_FUEL, AppCtx, AppHost, Bundle, Kind, Runtime, add_fileos_to_linker, wasm_error};

/// The most time one request may take.
const REQUEST_TIME: Duration = Duration::from_secs(30);

/// How much fuel an app may use before it lets other work run. With this,
/// the time limit works even when an app does not stop.
const FUEL_SLICE: u64 = 1_000_000;

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
    host: Arc<dyn AppHost>,
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
    pub fn http_app(&self, bundle: &Bundle, host: Arc<dyn AppHost>) -> Result<HttpApp, Error> {
        let component = self.component(bundle, Kind::App)?;
        let mut linker = Linker::new(&self.engine);
        add_fileos_to_linker(&mut linker)?;
        wasmtime_wasi_http::p2::add_to_linker_async(&mut linker).map_err(wasm_error)?;
        let pre = ProxyPre::new(linker.instantiate_pre(&component).map_err(wasm_error)?)
            .map_err(wasm_error)?;
        Ok(HttpApp {
            id: bundle.manifest.id.clone(),
            host,
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
        use http_body_util::BodyExt as _;
        let mut request = hyper::Request::builder()
            .method(method)
            .uri(format!("http://{}.localhost{path}", self.id));
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let body = http_body_util::Full::new(Bytes::from(body)).map_err(
            |never: std::convert::Infallible| -> wasmtime_wasi_http::Error { match never {} },
        );
        let request = request
            .body(body)
            .map_err(|e| Error(format!("not a good request: {e}")))?;
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
            .map_err(|e| Error(format!("the app {} failed in its body: {e}", self.id)))?
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
    ) -> Result<hyper::Response<HyperOutgoingBody>, Error>
    where
        B: hyper::body::Body<Data = Bytes> + Send + 'static,
        B::Error: Into<wasmtime_wasi_http::Error>,
    {
        let mut store = Store::new(
            self.pre.engine(),
            AppCtx::new(&self.id, Arc::clone(&self.host)),
        );
        store.limiter(|ctx| &mut ctx.limits);
        store.set_fuel(APP_FUEL).map_err(wasm_error)?;
        store
            .fuel_async_yield_interval(Some(FUEL_SLICE))
            .map_err(wasm_error)?;
        let (sender, receiver) = tokio::sync::oneshot::channel();
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
            let proxy = pre.instantiate_async(&mut store).await?;
            proxy
                .wasi_http_incoming_handler()
                .call_handle(&mut store, req, out)
                .await
        }));
        match receiver.await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(e)) => Err(Error(format!("the app {} failed: {e:?}", self.id))),
            Err(_) => Err(match task.await {
                Ok(Ok(Ok(()))) => Error(format!("the app {} gave no response", self.id)),
                Ok(Ok(Err(e))) => Error(format!("the app {} failed: {e:#}", self.id)),
                Ok(Err(_)) => Error(format!("the app {} took too long", self.id)),
                Err(e) => Error(format!("the app {} failed: {e}", self.id)),
            }),
        }
    }
}
