// SPDX-License-Identifier: AGPL-3.0-only

//! A small HTTP client for talking to other nodes and push services.
//!
//! It uses one connection per request and HTTP/1.1. TLS goes through rustls,
//! with the ring backend and the Mozilla root certificates.
//!
//! Nodes prove who they are with signatures, not with their TLS
//! certificates: requests are signed with HTTP Message Signatures, and log
//! entries are signed too. So a client made with
//! [`Client::accepting_self_signed`] accepts a node's own self-signed
//! certificate.

use std::backtrace::Backtrace;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;

/// The largest response body that the non-streaming transport accepts.
///
/// It is as large as the biggest object the server keeps, so federation
/// reads fit too.
pub const MAX_RESPONSE: usize = 256 * 1024 * 1024;

/// A transport error and the error that first caused it.
#[derive(Debug)]
pub struct Error {
    message: String,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
    backtrace: Box<Backtrace>,
}

impl Error {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
            backtrace: Box::new(Backtrace::capture()),
        }
    }

    fn caused_by(
        message: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        let mut error = Self::new(message);
        error.source = Some(source.into());
        error
    }

    /// The backtrace taken where the transport error was made.
    pub fn backtrace(&self) -> &Backtrace {
        &self.backtrace
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|source| source as &dyn std::error::Error)
    }
}

impl From<hyper::Error> for Error {
    fn from(error: hyper::Error) -> Self {
        Self::caused_by(error.to_string(), error)
    }
}

impl From<tokio::time::error::Elapsed> for Error {
    fn from(error: tokio::time::error::Elapsed) -> Self {
        Self::caused_by("timed out", error)
    }
}

impl From<Error> for String {
    fn from(error: Error) -> Self {
        error.message
    }
}

/// An HTTP client.
#[derive(Clone)]
pub struct Client {
    tls: tokio_rustls::TlsConnector,
    timeout: Duration,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("timeout", &self.timeout)
            .finish_non_exhaustive()
    }
}

impl Default for Client {
    fn default() -> Self {
        Client::new()
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

impl Client {
    /// A client that checks certificates against the Mozilla roots.
    ///
    /// # Panics
    ///
    /// Panics if the TLS provider cannot set up the supported protocol
    /// versions.
    pub fn new() -> Client {
        let roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };

        let config = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .expect("ring supports the default versions")
            .with_root_certificates(roots)
            .with_no_client_auth();
        Client::with_config(config)
    }

    /// A client that accepts any certificate. Use it only where the peer
    /// proves who it is in some other way (signed requests and signed
    /// entries).
    ///
    /// # Panics
    ///
    /// Panics if the TLS provider cannot set up the supported protocol
    /// versions.
    pub fn accepting_self_signed() -> Client {
        let config = rustls::ClientConfig::builder_with_provider(provider())
            .with_safe_default_protocol_versions()
            .expect("ring supports the default versions")
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AnyCert(provider())))
            .with_no_client_auth();
        Client::with_config(config)
    }

    fn with_config(mut config: rustls::ClientConfig) -> Client {
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Client {
            tls: tokio_rustls::TlsConnector::from(Arc::new(config)),
            timeout: Duration::from_secs(30),
        }
    }

    /// Send a request and read the whole response.
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported URL, a network or TLS error, a
    /// timeout, or a body that is too large.
    pub async fn send(
        &self,
        req: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>, Error> {
        tokio::time::timeout(self.timeout, async {
            let (parts, body) = self.open(req).await?.into_parts();
            let body = Limited::new(body, MAX_RESPONSE)
                .collect()
                .await
                .map_err(|error| Error::caused_by(error.to_string(), error))?
                .to_bytes();
            Ok(http::Response::from_parts(parts, body.to_vec()))
        })
        .await?
    }

    /// Send a request and return as soon as the response headers come. The
    /// body is read as it comes, with no size or time limit (for exports,
    /// downloads and event streams).
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported URL, or for a network or TLS error
    /// or timeout before the headers come.
    pub async fn send_streaming(
        &self,
        req: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Streaming>, Error> {
        let response = tokio::time::timeout(self.timeout, self.open(req)).await??;
        Ok(response.map(Streaming))
    }

    async fn open(&self, req: http::Request<Vec<u8>>) -> Result<http::Response<Incoming>, Error> {
        let uri = req.uri().clone();
        let host = uri
            .host()
            .ok_or_else(|| Error::new("no host in URL"))?
            .to_string();
        let https = match uri.scheme_str() {
            Some("https") => true,
            Some("http") => false,
            _ => return Err(Error::new(format!("unsupported URL {uri}"))),
        };
        let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
        let tcp = TcpStream::connect((host.trim_matches(['[', ']']), port))
            .await
            .map_err(|error| Error::caused_by(format!("{host}: {error}"), error))?;
        let _ = tcp.set_nodelay(true);
        let (mut parts, body) = req.into_parts();
        if !parts.headers.contains_key(http::header::HOST) {
            let value = match uri.port_u16() {
                Some(p) => format!("{host}:{p}"),
                None => host.clone(),
            };
            if let Ok(v) = http::HeaderValue::from_str(&value) {
                parts.headers.insert(http::header::HOST, v);
            }
        }
        // HTTP/1.1 needs the origin-form on the wire.
        let path = uri.path_and_query().map_or("/", |p| p.as_str());
        parts.uri = path
            .parse()
            .map_err(|error| Error::caused_by("bad path", error))?;
        let req = http::Request::from_parts(parts, Full::new(Bytes::from(body)));
        if https {
            let name = ServerName::try_from(host.trim_matches(['[', ']']).to_string())
                .map_err(|error| Error::caused_by(error.to_string(), error))?;
            let tls = self
                .tls
                .connect(name, tcp)
                .await
                .map_err(|error| Error::caused_by(format!("{host}: TLS: {error}"), error))?;
            exchange(TokioIo::new(tls), req).await
        } else {
            exchange(TokioIo::new(tcp), req).await
        }
    }
}

async fn exchange<IO>(
    io: IO,
    req: http::Request<Full<Bytes>>,
) -> Result<http::Response<Incoming>, Error>
where
    IO: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    Ok(sender.send_request(req).await?)
}

/// A response body, read as it comes in.
#[derive(Debug)]
pub struct Streaming(Incoming);

impl Streaming {
    /// The next part of the body, or `None` at its end.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    pub async fn chunk(&mut self) -> Result<Option<Bytes>, Error> {
        loop {
            match self.0.frame().await {
                None => return Ok(None),
                Some(Err(error)) => return Err(error.into()),
                Some(Ok(frame)) => {
                    if let Ok(data) = frame.into_data() {
                        return Ok(Some(data));
                    }
                }
            }
        }
    }

    /// The rest of the body, up to `limit` bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails or the body is longer than
    /// `limit`.
    pub async fn read_to_end(&mut self, limit: usize) -> Result<Vec<u8>, Error> {
        let mut out = Vec::new();
        while let Some(data) = self.chunk().await? {
            if out.len() + data.len() > limit {
                return Err(Error::new("response too large"));
            }
            out.extend_from_slice(&data);
        }
        Ok(out)
    }
}

#[derive(Debug)]
struct AnyCert(Arc<rustls::crypto::CryptoProvider>);

impl rustls::client::danger::ServerCertVerifier for AnyCert {
    fn verify_server_cert(
        &self,
        _: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::{Body, Handler, Peer, Request, Response, self_signed, serve, tls_config};

    struct Echo;

    impl Handler for Echo {
        fn handle(&self, req: Request, peer: Peer) -> impl Future<Output = Response> + Send {
            let text = format!(
                "{} {} {} tls={}",
                req.method(),
                req.uri().path(),
                req.body().len(),
                peer.tls
            );
            std::future::ready(http::Response::new(Body::from(text)))
        }
    }

    #[tokio::test]
    async fn round_trip_plain_and_tls() {
        for tls in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let config = tls.then(|| {
                let (cert, key) = self_signed(&["localhost".into()]).unwrap();
                tls_config(&cert, &key).unwrap()
            });
            let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
            let server = tokio::spawn(serve(listener, Arc::new(Echo), config, async {
                let _ = stop_rx.await;
            }));
            let scheme = if tls { "https" } else { "http" };
            let req = http::Request::post(format!("{scheme}://localhost:{port}/x?y=1"))
                .body(b"abc".to_vec())
                .unwrap();
            let resp = Client::accepting_self_signed().send(req).await.unwrap();
            assert_eq!(
                String::from_utf8(resp.into_body()).unwrap(),
                format!("POST /x 3 tls={tls}")
            );
            if tls {
                // The normal client refuses a self-signed certificate.
                let req = http::Request::get(format!("https://localhost:{port}/"))
                    .body(Vec::new())
                    .unwrap();
                Client::new().send(req).await.unwrap_err();
            }
            let _ = stop_tx.send(());
            server.await.unwrap();
        }
    }
}
