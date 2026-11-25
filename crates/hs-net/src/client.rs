// SPDX-License-Identifier: AGPL-3.0-only

//! A small HTTP client for talking to other nodes, push services and ACME
//! servers. One connection per request, HTTP/1.1, TLS through rustls with
//! the ring backend and the Mozilla root certificates.
//!
//! Nodes are authenticated by their signatures (requests are signed with
//! HTTP Message Signatures and log entries are signed), not by their TLS
//! certificates, so a node's own self-signed certificate is accepted when
//! the client is built with [`Client::accepting_self_signed`].

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::body::Incoming;
use hyper_util::rt::TokioIo;
use rustls::pki_types::ServerName;
use tokio::net::TcpStream;

/// The largest response body accepted.
pub const MAX_RESPONSE: usize = 256 * 1024 * 1024;

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

impl crate::acme::Transport for Client {
    fn send(
        &self,
        req: http::Request<Vec<u8>>,
    ) -> impl Future<Output = Result<http::Response<Vec<u8>>, String>> + Send {
        Client::send(self, req)
    }
}

impl Client {
    /// A client that checks certificates against the Mozilla roots.
    ///
    /// # Panics
    ///
    /// Panics if the TLS provider cannot configure the supported protocol versions.
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

    /// A client that accepts any certificate. Use only where the peer is
    /// authenticated some other way (signed requests and signed entries).
    ///
    /// # Panics
    ///
    /// Panics if the TLS provider cannot configure the supported protocol versions.
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
    /// Returns an error for an unsupported URL, network or TLS failure, timeout or oversized body.
    pub async fn send(
        &self,
        req: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Vec<u8>>, String> {
        tokio::time::timeout(self.timeout, async {
            let (parts, body) = self.open(req).await?.into_parts();
            let body = Limited::new(body, MAX_RESPONSE)
                .collect()
                .await
                .map_err(|e| e.to_string())?
                .to_bytes();
            Ok(http::Response::from_parts(parts, body.to_vec()))
        })
        .await
        .map_err(|_| "timed out".to_string())?
    }

    /// Send a request and return as soon as the response headers arrive;
    /// the body is read as it comes, with no size or time limit (exports,
    /// downloads and event streams).
    ///
    /// # Errors
    ///
    /// Returns an error for an unsupported URL, or a network or TLS failure
    /// or timeout before the headers arrive.
    pub async fn send_streaming(
        &self,
        req: http::Request<Vec<u8>>,
    ) -> Result<http::Response<Streaming>, String> {
        let response = tokio::time::timeout(self.timeout, self.open(req))
            .await
            .map_err(|_| "timed out".to_string())??;
        Ok(response.map(Streaming))
    }

    async fn open(&self, req: http::Request<Vec<u8>>) -> Result<http::Response<Incoming>, String> {
        let uri = req.uri().clone();
        let host = uri.host().ok_or("no host in URL")?.to_string();
        let https = match uri.scheme_str() {
            Some("https") => true,
            Some("http") => false,
            _ => return Err(format!("unsupported URL {uri}")),
        };
        let port = uri.port_u16().unwrap_or(if https { 443 } else { 80 });
        let tcp = TcpStream::connect((host.trim_matches(['[', ']']), port))
            .await
            .map_err(|e| format!("{host}: {e}"))?;
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
        // HTTP/1.1 wants origin-form on the wire.
        let path = uri.path_and_query().map_or("/", |p| p.as_str());
        parts.uri = path.parse().map_err(|_| "bad path")?;
        let req = http::Request::from_parts(parts, Full::new(Bytes::from(body)));
        if https {
            let name = ServerName::try_from(host.trim_matches(['[', ']']).to_string())
                .map_err(|e| e.to_string())?;
            let tls = self
                .tls
                .connect(name, tcp)
                .await
                .map_err(|e| format!("{host}: TLS: {e}"))?;
            exchange(TokioIo::new(tls), req).await
        } else {
            exchange(TokioIo::new(tcp), req).await
        }
    }
}

async fn exchange<IO>(
    io: IO,
    req: http::Request<Full<Bytes>>,
) -> Result<http::Response<Incoming>, String>
where
    IO: hyper::rt::Read + hyper::rt::Write + Unpin + Send + 'static,
{
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io)
        .await
        .map_err(|e| e.to_string())?;
    tokio::spawn(async move {
        let _ = conn.await;
    });
    sender.send_request(req).await.map_err(|e| e.to_string())
}

/// A response body read as it arrives.
#[derive(Debug)]
pub struct Streaming(Incoming);

impl Streaming {
    /// The next piece of the body; `None` at its end.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection fails.
    pub async fn chunk(&mut self) -> Result<Option<Bytes>, String> {
        loop {
            match self.0.frame().await {
                None => return Ok(None),
                Some(Err(e)) => return Err(e.to_string()),
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
    /// Returns an error if the connection fails or the body is longer.
    pub async fn read_to_end(&mut self, limit: usize) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        while let Some(data) = self.chunk().await? {
            if out.len() + data.len() > limit {
                return Err("response too large".into());
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
            let config = if tls {
                let (cert, key) = self_signed(&["localhost".into()]).unwrap();
                Some(tls_config(&cert, &key).unwrap())
            } else {
                None
            };
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
                assert!(Client::new().send(req).await.is_err());
            }
            let _ = stop_tx.send(());
            server.await.unwrap();
        }
    }
}
