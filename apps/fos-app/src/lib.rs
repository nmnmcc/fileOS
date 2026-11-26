// SPDX-License-Identifier: MIT OR Apache-2.0

//! The SDK for fileOS apps.
//!
//! An app implements [`app::exports::fos::app::events::Guest`] and calls
//! [`app::export_app!`]. The interfaces are in `wit/fos.wit`.

#![allow(unsafe_code, reason = "the code that wit-bindgen makes uses unsafe")]

/// Bindings for the `app` world.
#[allow(missing_docs, clippy::pedantic, reason = "made by wit-bindgen")]
pub mod app {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "app",
        // The WASI types that `wasi:http` uses come with these bindings.
        generate_all,
        pub_export_macro: true,
        export_macro_name: "export_app",
        default_bindings_module: "fos_app::app",
    });
}

/// HTTP for apps, on top of `wasi:http`.
///
/// The kernel gives an app the requests for its own origin. Implement
/// `incoming_handler::Guest` for the app and call [`http::serve`] from it,
/// with a function that makes a [`http::Response`] from a [`http::Request`]:
///
/// ```ignore
/// use fos_app::app::exports::wasi::http::incoming_handler::Guest as Http;
/// use fos_app::app::wasi::http::types::{IncomingRequest, ResponseOutparam};
///
/// impl Http for MyApp {
///     fn handle(request: IncomingRequest, out: ResponseOutparam) {
///         fos_app::http::serve(request, out, |req| {
///             fos_app::http::Response::text(200, format!("hello, {:?}", req.user()))
///         });
///     }
/// }
/// ```
pub mod http {
    use crate::app::wasi::http::types::{
        Fields, IncomingBody, IncomingRequest, Method, OutgoingBody, OutgoingResponse,
        ResponseOutparam,
    };
    use crate::app::wasi::io::streams::StreamError;

    /// The most bytes of a request body that [`serve`] reads.
    pub const MAX_BODY: usize = 64 << 20;

    /// An HTTP request, with all of its body.
    #[derive(Clone, Debug, Default)]
    pub struct Request {
        /// `GET`, `POST` and so on.
        pub method: String,
        /// The path and the query, for example `/photos?page=2`.
        pub path: String,
        /// The headers, with names in lowercase.
        pub headers: Vec<(String, Vec<u8>)>,
        pub body: Vec<u8>,
    }

    impl Request {
        /// The first header with this name, as text.
        #[must_use]
        pub fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(name))
                .and_then(|(_, v)| std::str::from_utf8(v).ok())
        }

        /// The person who made the request, as the kernel says: their DID.
        /// `None` when no one is signed in.
        #[must_use]
        pub fn user(&self) -> Option<&str> {
            self.header("fos-user")
        }
    }

    /// An HTTP response.
    #[derive(Clone, Debug)]
    pub struct Response {
        pub status: u16,
        pub headers: Vec<(String, Vec<u8>)>,
        pub body: Vec<u8>,
    }

    impl Response {
        /// A response with a body of this content type.
        #[must_use]
        pub fn new(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Self {
            Self {
                status,
                headers: vec![("content-type".into(), content_type.as_bytes().to_vec())],
                body: body.into(),
            }
        }

        /// A response with plain text.
        #[must_use]
        pub fn text(status: u16, body: impl Into<String>) -> Self {
            Self::new(status, "text/plain; charset=utf-8", body.into())
        }

        /// A response with HTML.
        #[must_use]
        pub fn html(status: u16, body: impl Into<String>) -> Self {
            Self::new(status, "text/html; charset=utf-8", body.into())
        }
    }

    fn method(method: &Method) -> String {
        match method {
            Method::Get => "GET".into(),
            Method::Head => "HEAD".into(),
            Method::Post => "POST".into(),
            Method::Put => "PUT".into(),
            Method::Delete => "DELETE".into(),
            Method::Connect => "CONNECT".into(),
            Method::Options => "OPTIONS".into(),
            Method::Trace => "TRACE".into(),
            Method::Patch => "PATCH".into(),
            Method::Other(other) => other.clone(),
        }
    }

    fn read_body(request: &IncomingRequest) -> Option<Vec<u8>> {
        let body = request.consume().ok()?;
        let mut data = Vec::new();
        {
            let stream = body.stream().ok()?;
            loop {
                match stream.blocking_read(64 * 1024) {
                    Ok(chunk) => {
                        if data.len() + chunk.len() > MAX_BODY {
                            return None;
                        }
                        data.extend_from_slice(&chunk);
                    }
                    Err(StreamError::Closed) => break,
                    Err(StreamError::LastOperationFailed(_)) => return None,
                }
            }
        }
        IncomingBody::finish(body);
        Some(data)
    }

    /// Reads the request, calls `handle`, and sends its response.
    pub fn serve(
        request: IncomingRequest,
        out: ResponseOutparam,
        handle: impl FnOnce(&Request) -> Response,
    ) {
        let Some(body) = read_body(&request) else {
            send(
                out,
                &Response::text(400, "the request body could not be read\n"),
            );
            return;
        };
        let req = Request {
            method: method(&request.method()),
            path: request.path_with_query().unwrap_or_else(|| "/".into()),
            headers: request.headers().entries(),
            body,
        };
        drop(request);
        send(out, &handle(&req));
    }

    fn send(out: ResponseOutparam, response: &Response) {
        let Ok(headers) = Fields::from_list(&response.headers) else {
            send(out, &Response::text(500, "the app gave a bad header\n"));
            return;
        };
        let outgoing = OutgoingResponse::new(headers);
        if outgoing.set_status_code(response.status).is_err() {
            send(out, &Response::text(500, "the app gave a bad status\n"));
            return;
        }
        let Ok(body) = outgoing.body() else { return };
        ResponseOutparam::set(out, Ok(outgoing));
        if let Ok(stream) = body.write() {
            for chunk in response.body.chunks(4096) {
                if stream.blocking_write_and_flush(chunk).is_err() {
                    break;
                }
            }
        }
        let _ = OutgoingBody::finish(body, None);
    }
}
