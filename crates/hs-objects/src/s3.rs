// SPDX-License-Identifier: AGPL-3.0-only

//! Sign S3 requests and parse S3 listings.
//!
//! The API supports AWS Signature
//! Version 4 for signing requests (client) and checking them (server), and the
//! `ListObjectsV2` response. Sans-IO: callers send the requests.

use ring::{digest, hmac};

/// S3 credentials.
#[derive(Clone)]
pub struct Credentials {
    /// Access key ID.
    pub access_key: String,
    /// Secret access key.
    pub secret_key: String,
    /// Region, such as `us-east-1` (Garage uses `garage`).
    pub region: String,
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Credentials")
            .field("region", &self.region)
            .finish_non_exhaustive()
    }
}

/// Lowercase hex SHA-256.
pub fn sha256_hex(data: &[u8]) -> String {
    hs_core::Hash::from_slice(digest::digest(&digest::SHA256, data).as_ref())
        .map(|h| h.to_hex())
        .unwrap_or_default()
}

fn hmac(key: &[u8], data: &str) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), data.as_bytes())
        .as_ref()
        .to_vec()
}

fn hex(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(bytes)
}

/// Everything but the RFC 3986 unreserved characters.
const UNRESERVED: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'_')
    .remove(b'.')
    .remove(b'~');
const PATH: &percent_encoding::AsciiSet = &UNRESERVED.remove(b'/');

/// Percent-encode as `SigV4` requires (`/` kept in paths).
pub fn uri_encode(s: &str, keep_slash: bool) -> String {
    let set = if keep_slash { PATH } else { UNRESERVED };
    percent_encoding::utf8_percent_encode(s, set).to_string()
}

/// The `SigV4` timestamp format, `YYYYMMDDTHHMMSSZ`.
const AMZ_DATE: &[time::format_description::BorrowedFormatItem<'_>] =
    time::macros::format_description!("[year][month][day]T[hour][minute][second]Z");

/// Format a Unix time (seconds) as `YYYYMMDDTHHMMSSZ`.
pub fn amz_date(unix: u64) -> String {
    i64::try_from(unix)
        .ok()
        .and_then(|u| time::OffsetDateTime::from_unix_timestamp(u).ok())
        .and_then(|t| t.format(AMZ_DATE).ok())
        .unwrap_or_default()
}

/// Unix seconds from `YYYYMMDDTHHMMSSZ`.
pub fn parse_amz_date(s: &str) -> Option<u64> {
    let t = time::PrimitiveDateTime::parse(s, AMZ_DATE).ok()?;
    u64::try_from(t.assume_utc().unix_timestamp()).ok()
}

/// A request to sign or check.
pub struct Request<'a> {
    /// HTTP method.
    pub method: &'a str,
    /// Host header value.
    pub host: &'a str,
    /// Path, already percent-encoded as sent.
    pub path: &'a str,
    /// Query parameters, not encoded.
    pub query: &'a [(String, String)],
    /// Lowercase hex SHA-256 of the body, or `UNSIGNED-PAYLOAD`.
    pub payload_hash: &'a str,
    /// Extra headers to sign (lowercase names), besides host and x-amz-*.
    pub headers: &'a [(String, String)],
}

impl std::fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .finish_non_exhaustive()
    }
}

fn canonical(req: &Request<'_>, date: &str) -> (String, String) {
    let mut query: Vec<(String, String)> = req
        .query
        .iter()
        .map(|(k, v)| (uri_encode(k, false), uri_encode(v, false)))
        .collect();
    query.sort();
    let query = query
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("&");
    let mut headers: Vec<(String, String)> = vec![
        ("host".into(), req.host.into()),
        ("x-amz-content-sha256".into(), req.payload_hash.into()),
        ("x-amz-date".into(), date.into()),
    ];
    headers.extend(
        req.headers
            .iter()
            .map(|(k, v)| (k.to_ascii_lowercase(), v.trim().to_string())),
    );
    headers.sort();
    headers.dedup_by(|a, b| a.0 == b.0);
    let signed: Vec<&str> = headers.iter().map(|(k, _)| k.as_str()).collect();
    let canon_headers: String = headers.iter().map(|(k, v)| format!("{k}:{v}\n")).collect();
    let signed = signed.join(";");
    (
        format!(
            "{}\n{}\n{}\n{}\n{}\n{}",
            req.method, req.path, query, canon_headers, signed, req.payload_hash
        ),
        signed,
    )
}

fn signature(creds: &Credentials, req: &Request<'_>, date: &str) -> (String, String, String) {
    let day = &date[..8];
    let scope = format!("{day}/{}/s3/aws4_request", creds.region);
    let (canon, signed) = canonical(req, date);
    let to_sign = format!(
        "AWS4-HMAC-SHA256\n{date}\n{scope}\n{}",
        sha256_hex(canon.as_bytes())
    );
    let k = hmac(format!("AWS4{}", creds.secret_key).as_bytes(), day);
    let k = hmac(&k, &creds.region);
    let k = hmac(&k, "s3");
    let k = hmac(&k, "aws4_request");
    (hex(&hmac(&k, &to_sign)), scope, signed)
}

/// Sign a request. Returns the headers to add: `authorization`,
/// `x-amz-date` and `x-amz-content-sha256`.
pub fn sign(creds: &Credentials, req: &Request<'_>, unix_time: u64) -> Vec<(String, String)> {
    let date = amz_date(unix_time);
    let (sig, scope, signed) = signature(creds, req, &date);
    vec![
        (
            "authorization".into(),
            format!(
                "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed}, Signature={sig}",
                creds.access_key
            ),
        ),
        ("x-amz-date".into(), date),
        ("x-amz-content-sha256".into(), req.payload_hash.into()),
    ]
}

/// Check a signed request on the server side. `authorization` and `date` are
/// the received header values; `headers` must include every header the
/// client listed as signed (other than host and x-amz-*).
pub fn verify(creds: &Credentials, req: &Request<'_>, authorization: &str, date: &str) -> bool {
    let Some(rest) = authorization.strip_prefix("AWS4-HMAC-SHA256 ") else {
        return false;
    };
    let mut credential = None;
    let mut given = None;
    for part in rest.split(',') {
        let part = part.trim();
        if let Some(c) = part.strip_prefix("Credential=") {
            credential = Some(c);
        } else if let Some(s) = part.strip_prefix("Signature=") {
            given = Some(s);
        }
    }
    let (Some(credential), Some(given)) = (credential, given) else {
        return false;
    };
    if !credential.starts_with(&format!("{}/", creds.access_key)) || date.len() < 8 {
        return false;
    }
    let (sig, _, _) = signature(creds, req, date);
    hs_core::ct_eq(sig.as_bytes(), given.as_bytes())
}

/// One object in a listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Object {
    /// The key.
    pub key: String,
    /// Size in bytes.
    pub size: u64,
}

fn xml_escape(s: &str) -> std::borrow::Cow<'_, str> {
    quick_xml::escape::escape(s)
}

/// Build a `ListObjectsV2` response.
pub fn list_response(bucket: &str, prefix: &str, objects: &[Object], next: Option<&str>) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ListBucketResult xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Name>{}</Name><Prefix>{}</Prefix><KeyCount>{}</KeyCount><MaxKeys>1000</MaxKeys><IsTruncated>{}</IsTruncated>",
        xml_escape(bucket),
        xml_escape(prefix),
        objects.len(),
        next.is_some()
    );
    if let Some(n) = next {
        out.push_str(&format!(
            "<NextContinuationToken>{}</NextContinuationToken>",
            xml_escape(n)
        ));
    }
    for o in objects {
        out.push_str(&format!(
            "<Contents><Key>{}</Key><Size>{}</Size></Contents>",
            xml_escape(&o.key),
            o.size
        ));
    }
    out.push_str("</ListBucketResult>");
    out
}

/// Parse a `ListObjectsV2` response: the objects and the continuation token.
pub fn parse_list(xml: &str) -> (Vec<Object>, Option<String>) {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut path: Vec<String> = Vec::new();
    let mut text = String::new();
    let mut objects = Vec::new();
    let mut next = None;
    let mut key = None;
    let mut size = 0;
    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                path.push(e.local_name().as_ref().to_string());
                text.clear();
            }
            Ok(Event::Text(t)) => text.push_str(&t.xml10_content()),
            Ok(Event::GeneralRef(r)) => match r.resolve_char_ref() {
                Ok(Some(c)) => text.push(c),
                _ => text.push_str(
                    quick_xml::escape::resolve_xml_entity(&r.xml10_content()).unwrap_or(""),
                ),
            },
            Ok(Event::End(_)) => {
                let names: Vec<&str> = path.iter().map(String::as_str).collect();
                match names[..] {
                    [.., "Contents", "Key"] => key = Some(std::mem::take(&mut text)),
                    [.., "Contents", "Size"] => size = text.trim().parse().unwrap_or(0),
                    [.., "NextContinuationToken"] => next = Some(std::mem::take(&mut text)),
                    [.., "Contents"] => {
                        if let Some(k) = key.take() {
                            objects.push(Object { key: k, size });
                        }
                        size = 0;
                    }
                    _ => {}
                }
                path.pop();
                text.clear();
            }
            Ok(Event::Eof) | Err(_) => break,
            _ => {}
        }
    }
    (objects, next)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_hides_s3_keys() {
        let credentials = Credentials {
            access_key: "example-access".into(),
            secret_key: "example-secret".into(),
            region: "example-region".into(),
        };
        let rendered = format!("{credentials:?}");
        assert!(rendered.contains("example-region"));
        assert!(!rendered.contains("example-access"));
        assert!(!rendered.contains("example-secret"));
    }

    #[test]
    fn aws_example_signature() {
        // From the AWS SigV4 documentation ("GET Object" example).
        let creds = Credentials {
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            region: "us-east-1".into(),
        };
        let empty = sha256_hex(b"");
        let req = Request {
            method: "GET",
            host: "examplebucket.s3.amazonaws.com",
            path: "/test.txt",
            query: &[],
            payload_hash: &empty,
            headers: &[("range".into(), "bytes=0-9".into())],
        };
        let date = "20130524T000000Z";
        let (sig, _, signed) = signature(&creds, &req, date);
        assert_eq!(signed, "host;range;x-amz-content-sha256;x-amz-date");
        assert_eq!(
            sig,
            "f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        );
        let headers = sign(&creds, &req, 1_369_353_600);
        assert_eq!(headers[1].1, date);
        assert!(verify(&creds, &req, &headers[0].1, date));
        assert!(!verify(
            &creds,
            &Request {
                path: "/other",
                ..req
            },
            &headers[0].1,
            date
        ));
    }

    #[test]
    fn listing_round_trip() {
        let objs = vec![
            Object {
                key: "a/b&c".into(),
                size: 3,
            },
            Object {
                key: "d".into(),
                size: 0,
            },
        ];
        let xml = list_response("bucket", "", &objs, Some("tok"));
        assert_eq!(parse_list(&xml), (objs, Some("tok".into())));
        assert_eq!(amz_date(0), "19700101T000000Z");
    }
}
