// SPDX-License-Identifier: AGPL-3.0-only

//! A typed client for the REST API, over [`hs_net::client`].
//!
//! Each method sends what the REST handlers expect and reads what they send
//! back. Both sides use the same types from [`crate::types`]. The bodies of
//! resources, blobs and event streams may be anything; they go through
//! [`Client::send`], [`Client::download`] and [`Client::events`].

use std::backtrace::Backtrace;
use std::fmt;
use std::sync::Arc;

use http::{Method, StatusCode, header};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use tokio::io::{AsyncWrite, AsyncWriteExt as _};

pub mod types;

use crate::types as t;

/// The largest body read whole. All bodies are read whole except exports,
/// blobs and event streams.
const MAX_BODY: usize = 64 * 1024 * 1024;

/// The characters that stay as they are in a path segment (RFC 3986
/// unreserved).
const SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');

/// One path segment, percent-encoded.
pub fn seg(s: &str) -> String {
    utf8_percent_encode(s, SEGMENT).to_string()
}

/// `/api/spaces/{space}/resources/{type}/{id}` for `type/id`.
///
/// # Errors
///
/// Returns an error if `target` is not `type/id`.
pub fn resource_path(space: &str, target: &str) -> Result<String, Error> {
    let (kind, id) = split_target(target)?;
    Ok(format!(
        "/api/spaces/{}/resources/{}/{}",
        seg(space),
        seg(kind),
        seg(id)
    ))
}

fn split_target(target: &str) -> Result<(&str, &str), Error> {
    if !hs_core::valid_target(target) {
        return Err(Error::local(format!(
            "not a resource: {target} (write TYPE/ID, such as note/shopping)"
        )));
    }
    Ok(target.split_once('/').expect("valid targets have a slash"))
}

/// What went wrong with a request.
#[derive(Clone, Debug)]
pub struct Error {
    /// The status the server sent back, if it answered.
    pub status: Option<StatusCode>,
    /// What went wrong, for people.
    pub message: String,
    context: Arc<ErrorContext>,
}

#[derive(Debug)]
struct ErrorContext {
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
    backtrace: Backtrace,
}

impl Error {
    fn local(message: impl Into<String>) -> Error {
        Self::new(None, message, None)
    }

    fn new(
        status: Option<StatusCode>,
        message: impl Into<String>,
        source: Option<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Error {
        Error {
            status,
            message: message.into(),
            context: Arc::new(ErrorContext {
                source,
                backtrace: Backtrace::capture(),
            }),
        }
    }

    fn caused_by(
        message: impl Into<String>,
        source: impl Into<Box<dyn std::error::Error + Send + Sync>>,
    ) -> Self {
        Self::new(None, message, Some(source.into()))
    }

    /// The backtrace taken when the request failed.
    pub fn backtrace(&self) -> &Backtrace {
        &self.context.backtrace
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.context
            .source
            .as_deref()
            .map(|source| source as &dyn std::error::Error)
    }
}

impl From<hs_net::client::Error> for Error {
    fn from(error: hs_net::client::Error) -> Self {
        Self::caused_by(error.to_string(), error)
    }
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::caused_by(error.to_string(), error)
    }
}

impl From<Error> for String {
    fn from(e: Error) -> String {
        e.message
    }
}

/// A response, read whole.
#[derive(Debug)]
pub struct Reply {
    /// Its status.
    pub status: StatusCode,
    /// Its headers.
    pub headers: http::HeaderMap,
    /// Its body.
    pub body: Vec<u8>,
}

impl Reply {
    /// The body as JSON (`null` if empty).
    ///
    /// # Errors
    ///
    /// Returns an error if the body is not JSON.
    pub fn json(&self) -> Result<serde_json::Value, Error> {
        if self.body.is_empty() {
            return Ok(serde_json::Value::Null);
        }
        self.typed()
    }

    /// The body as one of the API's types.
    ///
    /// # Errors
    ///
    /// Returns an error if the body is not that type.
    pub fn typed<T: DeserializeOwned>(&self) -> Result<T, Error> {
        serde_json::from_slice(&self.body)
            .map_err(|error| Error::caused_by(format!("the server sent bad JSON: {error}"), error))
    }

    /// A header, if it is text.
    pub fn header(&self, name: header::HeaderName) -> Option<&str> {
        self.headers.get(name).and_then(|v| v.to_str().ok())
    }
}

/// A request body and its media type.
pub type Body<'a> = Option<(&'a str, Vec<u8>)>;

/// A node's API, reached with a token.
#[derive(Clone)]
pub struct Client {
    /// `http(s)://host:port`, without a slash at the end.
    pub url: String,
    token: String,
    http: hs_net::client::Client,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl Client {
    /// A client for the node at `url`. With `insecure`, it accepts any TLS
    /// certificate (such as a node's own self-signed one).
    pub fn new(url: String, token: String, insecure: bool) -> Client {
        let http = if insecure {
            hs_net::client::Client::accepting_self_signed()
        } else {
            hs_net::client::Client::new()
        };
        Client { url, token, http }
    }

    fn request(
        &self,
        method: Method,
        path: &str,
        headers: &[(header::HeaderName, String)],
        body: Body<'_>,
    ) -> Result<http::Request<Vec<u8>>, Error> {
        let mut b = http::Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.url))
            .header(header::AUTHORIZATION, format!("Bearer {}", self.token))
            .header(
                header::USER_AGENT,
                concat!("homeserver/", env!("CARGO_PKG_VERSION")),
            );
        for (k, v) in headers {
            b = b.header(k, v);
        }
        let bytes = match body {
            Some((media_type, bytes)) => {
                b = b.header(header::CONTENT_TYPE, media_type);
                bytes
            }
            None => Vec::new(),
        };
        b.body(bytes)
            .map_err(|error| Error::caused_by(error.to_string(), error))
    }

    async fn open(
        &self,
        req: http::Request<Vec<u8>>,
    ) -> Result<http::Response<hs_net::client::Streaming>, Error> {
        self.http.send_streaming(req).await.map_err(|error| {
            Error::caused_by(format!(
                "cannot reach {}: {error}\n(is `hs ctl serve` running there? Commands for a stopped node take --dir.)",
                self.url
            ), error)
        })
    }

    /// Send a request and return whatever comes back.
    ///
    /// # Errors
    ///
    /// Returns an error if the node cannot be reached.
    pub async fn fetch(
        &self,
        method: Method,
        path: &str,
        headers: &[(header::HeaderName, String)],
        body: Body<'_>,
    ) -> Result<Reply, Error> {
        let req = self.request(method, path, headers, body)?;
        let (parts, mut body) = self.open(req).await?.into_parts();
        Ok(Reply {
            status: parts.status,
            headers: parts.headers,
            body: body.read_to_end(MAX_BODY).await?,
        })
    }

    /// Send a request. Any answer other than 2xx (or 304) is an error.
    ///
    /// # Errors
    ///
    /// Returns an error if the node cannot be reached or the request fails.
    pub async fn send(
        &self,
        method: Method,
        path: &str,
        headers: &[(header::HeaderName, String)],
        body: Body<'_>,
    ) -> Result<Reply, Error> {
        let reply = self.fetch(method, path, headers, body).await?;
        self.check(&reply)?;
        Ok(reply)
    }

    /// The error for a reply other than 2xx or 304. The message comes from
    /// its `{"error": …}`.
    ///
    /// # Errors
    ///
    /// Returns that error.
    pub fn check(&self, r: &Reply) -> Result<(), Error> {
        if r.status.is_success() || r.status == StatusCode::NOT_MODIFIED {
            return Ok(());
        }
        Err(self.status_error(r))
    }

    fn status_error(&self, r: &Reply) -> Error {
        let message = serde_json::from_slice::<t::Error>(&r.body)
            .map(|e| e.error)
            .unwrap_or_else(|_| {
                r.status
                    .canonical_reason()
                    .unwrap_or("error")
                    .to_lowercase()
            });
        let url = &self.url;
        let message = match r.status {
            StatusCode::UNAUTHORIZED => format!(
                "{url} did not accept the token ({message}); sign in again with `hs login {url}`"
            ),
            StatusCode::PRECONDITION_FAILED => {
                format!("it changed on the server after you read it ({message})")
            }
            s => format!("{message} ({})", s.as_u16()),
        };
        Error::new(Some(r.status), message, None)
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<T, Error> {
        let body = body
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|error| Error::caused_by("cannot write the request body as JSON", error))?
            .map(|bytes| ("application/json", bytes));
        self.send(method, path, &[], body).await?.typed()
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, Error> {
        self.call(Method::GET, path, None::<&()>).await
    }

    async fn get_with<T: DeserializeOwned>(
        &self,
        path: &str,
        query: &impl Serialize,
    ) -> Result<T, Error> {
        let q = serde_urlencoded::to_string(query)
            .map_err(|error| Error::caused_by(error.to_string(), error))?;
        let path = if q.is_empty() {
            path.to_string()
        } else {
            format!("{path}?{q}")
        };
        self.get(&path).await
    }

    async fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: &impl Serialize,
    ) -> Result<T, Error> {
        self.call(Method::POST, path, Some(body)).await
    }

    async fn delete(&self, path: &str) -> Result<(), Error> {
        self.send(Method::DELETE, path, &[], None).await.map(drop)
    }

    /// Send a GET and give the body to `out` as it comes in. Returns the
    /// response headers.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails or `out` cannot be written.
    pub async fn download(
        &self,
        path: &str,
        out: &mut (impl AsyncWrite + Unpin + Send),
    ) -> Result<http::response::Parts, Error> {
        let req = self.request(Method::GET, path, &[], None)?;
        let (parts, mut body) = self.open(req).await?.into_parts();
        if !parts.status.is_success() {
            let reply = Reply {
                status: parts.status,
                headers: parts.headers,
                body: body.read_to_end(MAX_BODY).await.unwrap_or_default(),
            };
            return Err(self.status_error(&reply));
        }
        while let Some(data) = body.chunk().await? {
            out.write_all(&data).await?;
        }
        out.flush().await?;
        Ok(parts)
    }

    /// Give each line of a `text/event-stream` to `each` as it comes in. Stop
    /// when `each` returns false or the server closes the stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn event_lines(
        &self,
        path: &str,
        mut each: impl FnMut(&str) -> bool,
    ) -> Result<(), Error> {
        let headers = [(header::ACCEPT, "text/event-stream".to_string())];
        let req = self.request(Method::GET, path, &headers, None)?;
        let (parts, mut body) = self.open(req).await?.into_parts();
        if !parts.status.is_success() {
            let reply = Reply {
                status: parts.status,
                headers: parts.headers,
                body: body.read_to_end(MAX_BODY).await.unwrap_or_default(),
            };
            return self.check(&reply);
        }
        let mut pending = Vec::new();
        while let Some(data) = body.chunk().await? {
            pending.extend_from_slice(&data);
            while let Some(end) = pending.iter().position(|b| *b == b'\n') {
                let line: Vec<u8> = pending.drain(..=end).collect();
                let line = String::from_utf8_lossy(&line);
                if !each(line.trim_end_matches(['\n', '\r'])) {
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// Give each change to `each` as it happens (`GET /api/events`). Stop
    /// when `each` returns false or the server closes the stream.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn events(&self, mut each: impl FnMut(t::Event) -> bool) -> Result<(), Error> {
        self.event_lines("/api/events", |line| {
            let Some(data) = line.strip_prefix("data:") else {
                return true;
            };
            match serde_json::from_str(data.trim()) {
                Ok(ev) => each(ev),
                Err(_) => true,
            }
        })
        .await
    }

    // Sessions and the node

    /// `GET /api/session`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn session(&self) -> Result<t::Session, Error> {
        self.get("/api/session").await
    }

    /// `GET /api/node`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn node(&self) -> Result<t::NodeInfo, Error> {
        self.get("/api/node").await
    }

    /// `GET /api/identity`: the owner's DID document.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn identity(&self) -> Result<serde_json::Value, Error> {
        self.get("/api/identity").await
    }

    /// `GET /api/health`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn health(&self) -> Result<t::Health, Error> {
        self.get("/api/health").await
    }

    // Spaces

    /// `GET /api/spaces`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn spaces(&self) -> Result<Vec<t::Space>, Error> {
        self.get("/api/spaces").await
    }

    /// `GET /api/spaces/{space}`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn space(&self, space: &str) -> Result<t::Space, Error> {
        self.get(&format!("/api/spaces/{}", seg(space))).await
    }

    /// `POST /api/spaces`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn create_space(&self, new: &t::NewSpace) -> Result<t::Space, Error> {
        self.post("/api/spaces", new).await
    }

    /// `POST /api/spaces/join`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn join(&self, join: &t::Join) -> Result<t::Space, Error> {
        self.post("/api/spaces/join", join).await
    }

    /// `POST /api/spaces/{space}/members`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn add_member(
        &self,
        space: &str,
        new: &t::NewMember,
    ) -> Result<t::MemberAdded, Error> {
        self.post(&format!("/api/spaces/{}/members", seg(space)), new)
            .await
    }

    /// `DELETE /api/spaces/{space}/members/{node}`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn remove_member(&self, space: &str, node: &str) -> Result<(), Error> {
        self.delete(&format!("/api/spaces/{}/members/{}", seg(space), seg(node)))
            .await
    }

    /// `POST /api/spaces/{space}/handover`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn handover(&self, space: &str, node: &str) -> Result<t::Seq, Error> {
        self.post(
            &format!("/api/spaces/{}/handover", seg(space)),
            &t::Handover {
                node: node.to_string(),
            },
        )
        .await
    }

    /// `POST /api/spaces/{space}/sync`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn pull(&self, space: &str) -> Result<t::Pulled, Error> {
        self.call(
            Method::POST,
            &format!("/api/spaces/{}/sync", seg(space)),
            None::<&()>,
        )
        .await
    }

    // Resources

    /// `GET /api/spaces/{space}/resources?page=1`: one page.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn resources(&self, space: &str, q: &t::ListQuery) -> Result<t::Page, Error> {
        let q = t::ListQuery {
            page: Some(1),
            ..q.clone()
        };
        self.get_with(&format!("/api/spaces/{}/resources", seg(space)), &q)
            .await
    }

    /// `POST /api/spaces/{space}/erase/{type}/{id}`.
    ///
    /// # Errors
    ///
    /// Returns an error if `target` is not `type/id` or the request fails.
    pub async fn erase(&self, space: &str, target: &str) -> Result<(), Error> {
        let (kind, id) = split_target(target)?;
        let path = format!("/api/spaces/{}/erase/{}/{}", seg(space), seg(kind), seg(id));
        self.send(Method::POST, &path, &[], None).await.map(drop)
    }

    /// `DELETE /api/spaces/{space}/resources/{type}/{id}`.
    ///
    /// # Errors
    ///
    /// Returns an error if `target` is not `type/id` or the request fails.
    pub async fn delete_resource(&self, space: &str, target: &str) -> Result<(), Error> {
        self.delete(&resource_path(space, target)?).await
    }

    // Finding things

    /// `GET /api/search`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn search(&self, q: &t::SearchQuery) -> Result<Vec<t::Row>, Error> {
        self.get_with("/api/search", q).await
    }

    /// `GET /api/timeline`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn timeline(&self, q: &t::TimelineQuery) -> Result<Vec<t::Row>, Error> {
        let q = t::TimelineQuery {
            page: None,
            ..q.clone()
        };
        self.get_with("/api/timeline", &q).await
    }

    /// `GET /api/near`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn near(&self, q: &t::NearQuery) -> Result<Vec<t::Row>, Error> {
        self.get_with("/api/near", q).await
    }

    /// `GET /api/related/{space}/{type}/{id}`.
    ///
    /// # Errors
    ///
    /// Returns an error if `target` is not `type/id` or the request fails.
    pub async fn related(
        &self,
        space: &str,
        target: &str,
        q: &t::Limit,
    ) -> Result<Vec<t::Related>, Error> {
        let (kind, id) = split_target(target)?;
        let path = format!("/api/related/{}/{}/{}", seg(space), seg(kind), seg(id));
        self.get_with(&path, q).await
    }

    // Blobs

    /// `POST /api/blobs`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn put_blob(&self, media_type: &str, bytes: Vec<u8>) -> Result<t::Blob, Error> {
        self.send(Method::POST, "/api/blobs", &[], Some((media_type, bytes)))
            .await?
            .typed()
    }

    // Grants

    /// `GET /api/grants`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn grants(&self) -> Result<Vec<t::Grant>, Error> {
        self.get("/api/grants").await
    }

    /// `POST /api/grants`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn create_grant(&self, new: &t::NewGrant) -> Result<t::NewToken, Error> {
        self.post("/api/grants", new).await
    }

    /// `DELETE /api/grants/{grant}`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn revoke_grant(&self, id: &str) -> Result<(), Error> {
        self.delete(&format!("/api/grants/{}", seg(id))).await
    }

    // Replicas

    /// `GET /api/replicas`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn replicas(&self) -> Result<Vec<String>, Error> {
        self.get("/api/replicas").await
    }

    /// `POST /api/replicas/sync`.
    ///
    /// # Errors
    ///
    /// Returns an error if the request fails.
    pub async fn sync_replicas(&self) -> Result<Vec<t::ReplicaSync>, Error> {
        self.call(Method::POST, "/api/replicas/sync", None::<&()>)
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_are_encoded_and_checked() {
        assert_eq!(seg("家 home/x"), "%E5%AE%B6%20home%2Fx");
        assert_eq!(
            resource_path("家", "note/x").unwrap(),
            "/api/spaces/%E5%AE%B6/resources/note/x"
        );
        resource_path("s", "note").unwrap_err();
        resource_path("s", "note/../x").unwrap_err();
    }

    #[test]
    fn queries_leave_out_what_is_not_given() {
        let q = t::SearchQuery {
            q: "a b".into(),
            limit: None,
        };
        assert_eq!(serde_urlencoded::to_string(&q).unwrap(), "q=a+b");
        let q = t::ListQuery {
            kind: Some("note".into()),
            page: Some(1),
            ..t::ListQuery::default()
        };
        assert_eq!(serde_urlencoded::to_string(&q).unwrap(), "type=note&page=1");
    }
}

#[cfg(test)]
mod download_tests {
    use super::*;
    use tokio::io::AsyncReadExt as _;

    #[tokio::test]
    async fn not_modified_is_a_download_error_and_the_future_is_send() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(stream.read_u8().await.unwrap());
            }
            stream
                .write_all(b"HTTP/1.1 304 Not Modified\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
        });
        let client = Client::new(url, String::new(), false);
        let download = tokio::spawn(async move {
            let mut output = Vec::new();
            let error = client.download("/export", &mut output).await.unwrap_err();
            assert_eq!(error.status, Some(StatusCode::NOT_MODIFIED));
            assert_eq!(output, [] as [u8; 0]);
        });
        download.await.unwrap();
        server.await.unwrap();
    }

    #[test]
    fn malformed_response_preserves_the_json_error() {
        use std::error::Error as _;
        let reply = Reply {
            status: StatusCode::OK,
            headers: http::HeaderMap::new(),
            body: b"{".to_vec(),
        };
        let error = reply.typed::<t::Space>().unwrap_err();
        assert!(error.source().unwrap().is::<serde_json::Error>());
    }
}
