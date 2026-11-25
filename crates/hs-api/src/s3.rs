// SPDX-License-Identifier: AGPL-3.0-only

//! The part of S3 that replicas need, under `/s3/{bucket}/{key}`, so another
//! homeserver (a friend's, or the owner's second one) can hold an
//! encrypted replica.
//!
//! Path-style only: GET, HEAD, PUT and DELETE of objects, and
//! `ListObjectsV2` on a bucket. Requests must carry AWS Signature Version 4.
//! Keys are in the config:
//!
//! ```json
//! "s3_keys": [{ "access_key": "alice", "secret_key": "…", "bucket": "alice" }]
//! ```
//!
//! Each key reaches one bucket, kept under `buckets/<bucket>/` in the data
//! directory. What is stored there is already encrypted by its owner.

use hs_objects::Dir;
use hs_objects::s3::{self, Credentials, Object};
use http::{Method, StatusCode, header};
use serde_json::Value as Json;

use crate::Api;
use crate::util::{ApiError, Result, empty, header as hdr, percent_decode, respond, set_header};
use hs_net::server::{Request, Response};

const SKEW_S: u64 = 15 * 60;
const PAGE: usize = 1000;

fn s3_error(status: StatusCode, code: &str) -> Response {
    respond(
        status,
        "application/xml",
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Error><Code>{code}</Code><Message>{code}</Message></Error>"
        ),
    )
}

/// Check the signature; returns the bucket the key may use.
fn authorize(api: &Api, req: &Request) -> std::result::Result<String, Box<Response>> {
    let denied = || Box::new(s3_error(StatusCode::FORBIDDEN, "AccessDenied"));
    let auth = hdr(req, header::AUTHORIZATION).ok_or_else(denied)?;
    let date = hdr(req, "x-amz-date").ok_or_else(denied)?;
    let t = s3::parse_amz_date(date).ok_or_else(denied)?;
    if t.abs_diff(hs_node::now_ms() / 1000) > SKEW_S {
        return Err(Box::new(s3_error(
            StatusCode::FORBIDDEN,
            "RequestTimeTooSkewed",
        )));
    }
    let field = |name: &str| {
        auth.split(',').find_map(|p| {
            p.trim()
                .split_once(' ')
                .map_or(p.trim(), |(_, v)| v)
                .strip_prefix(name)
        })
    };
    let credential = field("Credential=").ok_or_else(denied)?;
    let mut scope = credential.split('/');
    let access_key = scope.next().unwrap_or("");
    let region = scope.nth(1).unwrap_or("");
    let signed_headers: Vec<&str> = field("SignedHeaders=").unwrap_or("").split(';').collect();
    let keys = api
        .node
        .config()
        .raw
        .get("s3_keys")
        .cloned()
        .unwrap_or(Json::Null);
    let entry = keys
        .as_array()
        .and_then(|a| {
            a.iter()
                .find(|k| k.get("access_key").and_then(Json::as_str) == Some(access_key))
        })
        .ok_or_else(|| Box::new(s3_error(StatusCode::FORBIDDEN, "InvalidAccessKeyId")))?;
    let creds = Credentials {
        access_key: access_key.to_string(),
        secret_key: entry
            .get("secret_key")
            .and_then(Json::as_str)
            .unwrap_or("")
            .to_string(),
        region: region.to_string(),
    };
    let payload_hash = hdr(req, "x-amz-content-sha256").unwrap_or("");
    if payload_hash != "UNSIGNED-PAYLOAD" && payload_hash != s3::sha256_hex(req.body()) {
        return Err(Box::new(s3_error(
            StatusCode::BAD_REQUEST,
            "XAmzContentSHA256Mismatch",
        )));
    }
    let host = hdr(req, header::HOST).unwrap_or("");
    let headers: Vec<(String, String)> = signed_headers
        .iter()
        .filter(|h| !matches!(**h, "host" | "x-amz-date" | "x-amz-content-sha256"))
        .map(|h| (h.to_string(), hdr(req, *h).unwrap_or("").to_string()))
        .collect();
    let query: Vec<(String, String)> = req
        .uri()
        .query()
        .unwrap_or("")
        .split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (percent_decode(k, false), percent_decode(v, false))
        })
        .collect();
    let r = s3::Request {
        method: req.method().as_str(),
        host,
        path: req.uri().path(),
        query: &query,
        payload_hash,
        headers: &headers,
    };
    if !s3::verify(&creds, &r, auth, date) {
        return Err(Box::new(s3_error(
            StatusCode::FORBIDDEN,
            "SignatureDoesNotMatch",
        )));
    }
    Ok(entry
        .get("bucket")
        .and_then(Json::as_str)
        .unwrap_or(access_key)
        .to_string())
}

fn bucket_dir(api: &Api, bucket: &str) -> std::io::Result<Dir> {
    if bucket.is_empty()
        || !bucket
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
        || bucket.starts_with('.')
    {
        return Err(std::io::Error::other("bad bucket name"));
    }
    Dir::open(api.node.dir().join("buckets").join(bucket))
}

fn list(dir: &Dir, bucket: &str, req: &Request) -> Response {
    let q = crate::util::query(req);
    let prefix = crate::util::param(&q, "prefix").unwrap_or("");
    let after = crate::util::param(&q, "continuation-token")
        .or_else(|| crate::util::param(&q, "start-after"))
        .unwrap_or("");
    let names = dir.list("").unwrap_or_default();
    let mut objects = Vec::new();
    let mut next = None;
    for n in names
        .iter()
        .filter(|n| n.starts_with(prefix) && n.as_str() > after)
    {
        if objects.len() == PAGE {
            next = objects.last().map(|o: &Object| o.key.clone());
            break;
        }
        objects.push(Object {
            key: n.clone(),
            size: dir.len(n).ok().flatten().unwrap_or(0),
        });
    }
    respond(
        StatusCode::OK,
        "application/xml",
        s3::list_response(bucket, prefix, &objects, next.as_deref()),
    )
}

pub fn handle(api: &Api, req: &Request) -> Result {
    let allowed = match authorize(api, req) {
        Ok(b) => b,
        Err(r) => return Ok(*r),
    };
    let path = req
        .uri()
        .path()
        .strip_prefix("/s3/")
        .ok_or_else(ApiError::not_found)?;
    let (bucket, key) = path.split_once('/').unwrap_or((path, ""));
    let bucket = percent_decode(bucket, false);
    let key = percent_decode(key, false);
    if bucket != allowed {
        return Ok(s3_error(StatusCode::FORBIDDEN, "AccessDenied"));
    }
    let Ok(dir) = bucket_dir(api, &bucket) else {
        return Ok(s3_error(StatusCode::BAD_REQUEST, "InvalidBucketName"));
    };
    if key.is_empty() {
        return Ok(match *req.method() {
            Method::GET => list(&dir, &bucket, req),
            Method::HEAD | Method::PUT => empty(StatusCode::OK),
            _ => s3_error(StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed"),
        });
    }
    let io = |_: std::io::Error| s3_error(StatusCode::BAD_REQUEST, "InvalidArgument");
    Ok(match *req.method() {
        Method::PUT => match dir.put(&key, req.body()) {
            Ok(()) => {
                let mut r = empty(StatusCode::OK);
                set_header(
                    &mut r,
                    header::ETAG,
                    &format!("\"{}\"", &s3::sha256_hex(req.body())[..32]),
                );
                r
            }
            Err(e) => io(e),
        },
        Method::GET | Method::HEAD => match dir.get(&key) {
            Ok(Some(b)) => {
                let len = b.len() as u64;
                let mut r =
                    match hdr(req, header::RANGE).and_then(|h| hs_objects::parse_range(h, len)) {
                        Some((s, e)) => {
                            let mut r = respond(
                                StatusCode::PARTIAL_CONTENT,
                                "application/octet-stream",
                                b[s as usize..e as usize].to_vec(),
                            );
                            set_header(
                                &mut r,
                                header::CONTENT_RANGE,
                                &format!("bytes {s}-{}/{len}", e - 1),
                            );
                            r
                        }
                        None => respond(StatusCode::OK, "application/octet-stream", b),
                    };
                if req.method() == Method::HEAD {
                    *r.body_mut() = hs_net::server::Body::empty();
                    set_header(&mut r, header::CONTENT_LENGTH, &len.to_string());
                }
                r
            }
            Ok(None) => s3_error(StatusCode::NOT_FOUND, "NoSuchKey"),
            Err(e) => io(e),
        },
        Method::DELETE => match dir.delete(&key) {
            Ok(()) => empty(StatusCode::NO_CONTENT),
            Err(e) => io(e),
        },
        _ => s3_error(StatusCode::METHOD_NOT_ALLOWED, "MethodNotAllowed"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amz_dates() {
        assert_eq!(
            s3::parse_amz_date(&s3::amz_date(1_764_028_800)),
            Some(1_764_028_800)
        );
        assert_eq!(
            s3::parse_amz_date(&s3::amz_date(951_782_400)),
            Some(951_782_400)
        );
        assert_eq!(s3::parse_amz_date("2025-11-25"), None);
    }
}
