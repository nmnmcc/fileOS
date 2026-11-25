// SPDX-License-Identifier: AGPL-3.0-only

//! Small helpers for requests and responses.

use bytes::Bytes;
use hs_net::server::{Body, Request, Response};
use http::{HeaderValue, StatusCode, header};
use serde_json::{Value as Json, json};

/// An error answered as `{"error": "..."}`.
#[derive(Debug)]
pub struct ApiError {
    /// The status.
    pub status: StatusCode,
    /// What went wrong, for people.
    pub message: String,
    /// Where the primary is, for `NotPrimary`.
    pub home: Option<String>,
    source: Option<Box<dyn std::error::Error + Send + Sync>>,
    backtrace: Box<std::backtrace::Backtrace>,
}

impl ApiError {
    /// An error with a status.
    pub fn new(status: StatusCode, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            message: message.into(),
            home: None,
            source: None,
            backtrace: Box::new(std::backtrace::Backtrace::capture()),
        }
    }
    pub(crate) fn with_source(
        mut self,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// The backtrace taken where the error came into the API.
    pub fn backtrace(&self) -> &std::backtrace::Backtrace {
        &self.backtrace
    }

    /// 400.
    pub fn bad(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::BAD_REQUEST, message)
    }
    /// 404.
    pub fn not_found() -> ApiError {
        ApiError::new(StatusCode::NOT_FOUND, "not found")
    }
    /// 401.
    pub fn unauthorized() -> ApiError {
        ApiError::new(StatusCode::UNAUTHORIZED, "sign in first")
    }
    /// 403.
    pub fn forbidden(message: impl Into<String>) -> ApiError {
        ApiError::new(StatusCode::FORBIDDEN, message)
    }
    /// The response.
    pub fn response(&self) -> Response {
        json_status(self.status, &json!({ "error": self.message }))
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_deref()
            .map(|error| error as &dyn std::error::Error)
    }
}

impl From<hs_node::Error> for ApiError {
    fn from(error: hs_node::Error) -> ApiError {
        use hs_node::ErrorKind as E;
        let result = match error.kind() {
            E::Refused => ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, error.message()),
            E::Forbidden => ApiError::forbidden(error.message()),
            E::NotFound => ApiError::new(StatusCode::NOT_FOUND, error.message()),
            E::NotPrimary { space, home } => {
                let mut result = ApiError::new(
                    StatusCode::MISDIRECTED_REQUEST,
                    format!("this node is not the primary of {space}"),
                );
                result.home.clone_from(home);
                result
            }
            E::PreconditionFailed => {
                ApiError::new(StatusCode::PRECONDITION_FAILED, error.message())
            }
            E::Conflict => ApiError::new(StatusCode::CONFLICT, error.message()),
            E::Io => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, error.message()),
        };
        result.with_source(error)
    }
}

impl From<hs_store::Error> for ApiError {
    fn from(error: hs_store::Error) -> ApiError {
        ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, format!("index: {error}"))
            .with_source(error)
    }
}

/// A handler result.
pub type Result<T = Response, E = ApiError> = std::result::Result<T, E>;

/// A response with a status, content type and body.
pub fn respond(status: StatusCode, content_type: &str, body: impl Into<Body>) -> Response {
    let mut r = http::Response::new(body.into());
    *r.status_mut() = status;
    if let Ok(v) = HeaderValue::from_str(content_type) {
        r.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    r
}

/// A JSON response with a status.
pub fn json_status(status: StatusCode, value: &Json) -> Response {
    respond(
        status,
        "application/json",
        serde_json::to_vec(value).unwrap_or_default(),
    )
}

/// A typed JSON response with a status.
pub fn typed<T: serde::Serialize>(status: StatusCode, value: &T) -> Response {
    respond(
        status,
        "application/json",
        serde_json::to_vec(value).unwrap_or_default(),
    )
}

/// A typed request body.
///
/// # Errors
///
/// Answers 400 if the body is not that type.
pub fn body<T: serde::de::DeserializeOwned>(req: &Request) -> Result<T> {
    serde_json::from_slice(req.body()).map_err(|e| ApiError::bad(format!("bad request: {e}")))
}

/// A typed query.
///
/// # Errors
///
/// Answers 400 if the query is not that type.
pub fn params<T: serde::de::DeserializeOwned>(req: &Request) -> Result<T> {
    serde_urlencoded::from_str(req.uri().query().unwrap_or(""))
        .map_err(|e| ApiError::bad(format!("bad query: {e}")))
}

/// A 200 JSON response.
pub fn json_ok(value: &Json) -> Response {
    json_status(StatusCode::OK, value)
}

/// An empty response.
pub fn empty(status: StatusCode) -> Response {
    let mut r = http::Response::new(Body::empty());
    *r.status_mut() = status;
    r
}

/// Set a header. Values that are not valid header text are left out.
pub fn set_header(r: &mut Response, name: impl header::IntoHeaderName, value: &str) {
    if let Ok(v) = HeaderValue::from_str(value) {
        r.headers_mut().insert(name, v);
    }
}

/// A request header as text.
pub fn header(req: &Request, name: impl header::AsHeaderName) -> Option<&str> {
    req.headers().get(name).and_then(|v| v.to_str().ok())
}

/// The request body as JSON.
///
/// # Errors
///
/// Returns a bad-request error when the body is not valid JSON.
pub fn body_json(req: &Request) -> Result<Json> {
    serde_json::from_slice(req.body()).map_err(|e| ApiError::bad(format!("bad JSON: {e}")))
}

/// Decode `%XX`, and `+` as a space if asked (in queries).
pub fn percent_decode(s: &str, plus_is_space: bool) -> String {
    let s = if plus_is_space {
        s.replace('+', " ").into()
    } else {
        std::borrow::Cow::Borrowed(s)
    };
    percent_encoding::percent_decode_str(&s)
        .decode_utf8_lossy()
        .into_owned()
}

/// Everything but the RFC 3986 unreserved characters.
const COMPONENT: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');
const PATH: &percent_encoding::AsciiSet = &COMPONENT.remove(b'/');

/// Percent-encode everything but unreserved characters (and `/` if asked).
pub fn percent_encode(s: &str, keep_slash: bool) -> String {
    let set = if keep_slash { PATH } else { COMPONENT };
    percent_encoding::utf8_percent_encode(s, set).to_string()
}

/// Query parameters, decoded, in order.
pub fn query(req: &Request) -> Vec<(String, String)> {
    parse_form(req.uri().query().unwrap_or(""))
}

/// Parse `a=b&c=d` (queries and form bodies).
pub fn parse_form(s: &str) -> Vec<(String, String)> {
    form_urlencoded::parse(s.as_bytes()).into_owned().collect()
}

/// The first value of a parameter.
pub fn param<'a>(params: &'a [(String, String)], name: &str) -> Option<&'a str> {
    params
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

/// A cookie from the request.
pub fn cookie<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
    req.headers()
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|c| c.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v)
}

/// The origin this request was addressed to (`https://host`), from the
/// Host header and whether TLS was used, or the configured URL.
pub fn origin(req: &Request, tls: bool, configured: Option<&str>) -> String {
    if let Some(u) = configured {
        return u.trim_end_matches('/').to_string();
    }
    let host = header(req, header::HOST)
        .or_else(|| req.uri().authority().map(http::uri::Authority::as_str))
        .unwrap_or("localhost");
    let proto = header(req, "x-forwarded-proto").unwrap_or(if tls { "https" } else { "http" });
    format!("{proto}://{host}")
}

/// Escape text for XML and HTML.
pub fn escape(s: &str) -> String {
    quick_xml::escape::escape(s).into_owned()
}

/// A body from bytes.
pub fn bytes_body(b: Vec<u8>) -> Body {
    Body::Full(Bytes::from(b))
}

/// Base64url without padding.
pub fn b64url(b: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// Decode base64url (with or without padding) or standard base64.
pub fn unb64(s: &str) -> Option<Vec<u8>> {
    use base64::Engine as _;
    let t = s.trim_end_matches('=');
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(t)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(t))
        .ok()
}

/// RFC 7396 JSON Merge Patch.
pub fn merge_patch(target: &mut Json, patch: &Json) {
    match patch {
        Json::Object(p) => {
            if !target.is_object() {
                *target = Json::Object(serde_json::Map::new());
            }
            if let Some(t) = target.as_object_mut() {
                for (k, v) in p {
                    if v.is_null() {
                        t.remove(k);
                    } else {
                        merge_patch(t.entry(k.clone()).or_insert(Json::Null), v);
                    }
                }
            }
        }
        other => *target = other.clone(),
    }
}

/// An HTTP date (RFC 9110 IMF-fixdate) from milliseconds.
pub fn http_date(ms: u64) -> String {
    httpdate::fmt_http_date(std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoding_and_patch() {
        assert_eq!(percent_decode("a%20b+c%e4%bd%a0", true), "a b c你");
        assert_eq!(percent_decode("100%", false), "100%");
        assert_eq!(percent_encode("a b/c", true), "a%20b/c");
        let mut t = json!({"a": 1, "b": {"c": 2, "d": 3}});
        merge_patch(&mut t, &json!({"a": null, "b": {"c": 5}, "e": [1]}));
        assert_eq!(t, json!({"b": {"c": 5, "d": 3}, "e": [1]}));
        assert_eq!(http_date(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(http_date(951_868_800_000), "Wed, 01 Mar 2000 00:00:00 GMT");
    }
}
