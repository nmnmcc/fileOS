// SPDX-License-Identifier: AGPL-3.0-only

//! The HTTP server: HTTP/1.1 and HTTP/2 over TCP, with or without TLS.
//!
//! Handlers see a request whose body has already been read (up to a limit)
//! and return a response whose body is either complete or a stream of
//! chunks (for Server-Sent Events and large files).

use std::convert::Infallible;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use bytes::Bytes;
use http_body::Frame;
use http_body_util::{BodyExt, Limited};
use hyper::service::service_fn;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

/// The largest request body accepted.
///
/// Accommodates file uploads while capping the bytes buffered per request.
pub const MAX_BODY: usize = 256 * 1024 * 1024;

/// A response body: complete, or a stream of chunks that ends when the
/// sender is dropped.
pub enum Body {
    /// The whole body.
    Full(Bytes),
    /// Chunks as they come.
    Stream(mpsc::Receiver<Bytes>),
    /// Chunks from a producer that can fail after the headers are sent.
    Fallible(mpsc::Receiver<std::io::Result<Bytes>>),
}

impl std::fmt::Debug for Body {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Body").finish_non_exhaustive()
    }
}

impl Body {
    /// An empty body.
    pub fn empty() -> Body {
        Body::Full(Bytes::new())
    }
}

impl From<Vec<u8>> for Body {
    fn from(v: Vec<u8>) -> Body {
        Body::Full(Bytes::from(v))
    }
}

impl From<String> for Body {
    fn from(s: String) -> Body {
        Body::Full(Bytes::from(s))
    }
}

impl From<&'static str> for Body {
    fn from(s: &'static str) -> Body {
        Body::Full(Bytes::from_static(s.as_bytes()))
    }
}

impl From<Bytes> for Body {
    fn from(b: Bytes) -> Body {
        Body::Full(b)
    }
}

impl http_body::Body for Body {
    type Data = Bytes;
    type Error = std::io::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        match &mut *self {
            Body::Full(b) => {
                if b.is_empty() {
                    Poll::Ready(None)
                } else {
                    Poll::Ready(Some(Ok(Frame::data(std::mem::take(b)))))
                }
            }
            Body::Stream(rx) => rx.poll_recv(cx).map(|c| c.map(|b| Ok(Frame::data(b)))),
            Body::Fallible(rx) => rx
                .poll_recv(cx)
                .map(|chunk| chunk.map(|result| result.map(Frame::data))),
        }
    }

    fn is_end_stream(&self) -> bool {
        matches!(self, Body::Full(b) if b.is_empty())
    }

    fn size_hint(&self) -> http_body::SizeHint {
        match self {
            Body::Full(b) => http_body::SizeHint::with_exact(b.len() as u64),
            Body::Stream(_) | Body::Fallible(_) => http_body::SizeHint::default(),
        }
    }
}

/// A request with its body read.
pub type Request = http::Request<Bytes>;
/// A response.
pub type Response = http::Response<Body>;

/// Where a request came from.
#[derive(Clone, Copy, Debug)]
pub struct Peer {
    /// The remote address.
    pub addr: SocketAddr,
    /// Whether the connection used TLS.
    pub tls: bool,
}

/// Something that answers requests.
pub trait Handler: Send + Sync + 'static {
    /// Answer one request.
    fn handle(&self, req: Request, peer: Peer) -> impl Future<Output = Response> + Send;
}

fn too_large() -> Response {
    let mut r = http::Response::new(Body::from("request body too large\n"));
    *r.status_mut() = http::StatusCode::PAYLOAD_TOO_LARGE;
    r
}

/// Serve one accepted connection (plain or TLS) until it closes.
pub async fn serve_connection<H: Handler, IO>(io: IO, handler: Arc<H>, peer: Peer)
where
    IO: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let service = service_fn(move |req: http::Request<hyper::body::Incoming>| {
        let handler = Arc::clone(&handler);
        async move {
            let (parts, body) = req.into_parts();
            let resp = match Limited::new(body, MAX_BODY).collect().await {
                Ok(b) => {
                    handler
                        .handle(http::Request::from_parts(parts, b.to_bytes()), peer)
                        .await
                }
                Err(_) => too_large(),
            };
            Ok::<_, Infallible>(resp)
        }
    });
    let _ = Builder::new(TokioExecutor::new())
        .serve_connection_with_upgrades(TokioIo::new(io), service)
        .await;
}

/// Accept connections until the future `stop` completes. With `tls`, every
/// connection must complete a TLS handshake first.
pub async fn serve<H: Handler>(
    listener: TcpListener,
    handler: Arc<H>,
    tls: Option<Arc<rustls::ServerConfig>>,
    stop: impl Future<Output = ()>,
) {
    let acceptor = tls.map(tokio_rustls::TlsAcceptor::from);
    tokio::pin!(stop);
    loop {
        let (tcp, addr) = tokio::select! {
            () = &mut stop => return,
            a = listener.accept() => if let Ok(a) = a { a } else {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                continue;
            },
        };
        let _ = tcp.set_nodelay(true);
        let handler = Arc::clone(&handler);
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            match acceptor {
                None => serve_connection(tcp, handler, Peer { addr, tls: false }).await,
                Some(a) => {
                    let Ok(Ok(tls)) =
                        tokio::time::timeout(std::time::Duration::from_secs(10), a.accept(tcp))
                            .await
                    else {
                        return;
                    };
                    serve_connection(tls, handler, Peer { addr, tls: true }).await;
                }
            }
        });
    }
}

/// A self-signed certificate for `names`: (certificate PEM, key PEM).
///
/// # Errors
///
/// Returns an error if the names are invalid or certificate generation fails.
pub fn self_signed(names: &[String]) -> Result<(String, String), String> {
    let c = rcgen::generate_simple_self_signed(names.to_vec()).map_err(|e| e.to_string())?;
    Ok((c.cert.pem(), c.signing_key.serialize_pem()))
}

/// A TLS server configuration from PEM text, offering HTTP/2 and HTTP/1.1.
///
/// # Errors
///
/// Returns an error if the PEM, key pair or TLS configuration is invalid.
pub fn tls_config(
    cert_chain_pem: &str,
    key_pem: &str,
) -> Result<Arc<rustls::ServerConfig>, String> {
    let certs: Vec<CertificateDer<'static>> =
        CertificateDer::pem_slice_iter(cert_chain_pem.as_bytes())
            .collect::<Result<_, _>>()
            .map_err(|e| format!("certificate: {e}"))?;
    let key = PrivateKeyDer::from_pem_slice(key_pem.as_bytes()).map_err(|e| format!("key: {e}"))?;
    let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|e| e.to_string())?
    .with_no_client_auth()
    .with_single_cert(certs, key)
    .map_err(|e| e.to_string())?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(Arc::new(config))
}
