// SPDX-License-Identifier: AGPL-3.0-only

//! Signed node-to-node requests.
//!
//! Requests between nodes carry an RFC 9421 HTTP Message Signature made with
//! the node's Ed25519 key, and an RFC 9530 `Content-Digest` of the body. The
//! signature covers the method, authority, path, query and digest. So no one
//! can send the request again for another resource or with another body.
//! The `keyid` is the node's key ID, so the verifier sees who signed without
//! looking anything up. The caller decides whether to trust that key.

use std::fmt;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hs_core::PublicKey;
use hs_identity::Keypair;
use http::HeaderValue;
use http::request::Parts;
use ring::digest;
use sfv::{BareItem, Dictionary, InnerList, Integer, Item, Key, ListEntry, Parameters};

/// The signature label used by this module.
const LABEL: &str = "sig1";

/// Components every signature must cover.
const REQUIRED: [&str; 4] = ["@method", "@authority", "@path", "@query"];

/// Why a request signature was not accepted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "http signature: {}", self.0)
    }
}

impl std::error::Error for Error {}

fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

/// The RFC 9530 `Content-Digest` value of a body: `sha-256=:<base64>:`.
pub fn content_digest(body: &[u8]) -> String {
    format!(
        "sha-256=:{}:",
        STANDARD.encode(digest::digest(&digest::SHA256, body))
    )
}

/// Sign a request: add `Content-Digest` (for a non-empty body),
/// `Signature-Input` and `Signature` headers. `created` is Unix seconds.
///
/// # Errors
///
/// Returns an error if a needed signature component cannot be read from the
/// request or written as a header.
pub fn sign(parts: &mut Parts, body: &[u8], key: &Keypair, created: u64) -> Result<(), Error> {
    let mut components: Vec<String> = REQUIRED
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
    if !body.is_empty() {
        let value = HeaderValue::from_str(&content_digest(body)).or(err("digest header"))?;
        parts.headers.insert("content-digest", value);
        components.push("content-digest".into());
    }
    let string = |s: String| sfv::String::from_string(s).or(err("not an sf-string"));
    let mut params = Parameters::new();
    params.insert(
        key_of("created"),
        BareItem::Integer(Integer::try_from(created).or(err("time"))?),
    );
    params.insert(
        key_of("keyid"),
        BareItem::String(string(key.public().to_id())?),
    );
    params.insert(key_of("alg"), BareItem::String(string("ed25519".into())?));
    let list = InnerList::with_params(
        components
            .iter()
            .map(|c| Ok(Item::new(string(c.clone())?)))
            .collect::<Result<_, Error>>()?,
        params,
    );
    let input = serialize_inner_list(&list);
    let base = signature_base(parts, &components, &input)?;
    let sig = key.sign(base.as_bytes());
    let header = |s: String| HeaderValue::from_str(&s).or(err("signature header"));
    parts
        .headers
        .insert("signature-input", header(format!("{LABEL}={input}"))?);
    parts.headers.insert(
        "signature",
        header(format!("{LABEL}=:{}:", STANDARD.encode(sig)))?,
    );
    Ok(())
}

/// Verify a signed request and return the signer's key.
///
/// It checks that:
/// - the `sig1` signature covers the method, authority, path, query and (for
///   a non-empty body) the content digest;
/// - `created` is within `max_skew` seconds of `now`;
/// - the digest matches `body`;
/// - the signature is valid for the key named by `keyid`.
///
/// # Errors
///
/// Returns an error if needed components are missing, the signature is not
/// in its valid time window, or the check fails.
pub fn verify(parts: &Parts, body: &[u8], now: u64, max_skew: u64) -> Result<PublicKey, Error> {
    let input =
        joined_header(parts, "signature-input").ok_or(Error("no Signature-Input".into()))?;
    let sigs = joined_header(parts, "signature").ok_or(Error("no Signature".into()))?;
    let input = parse_dictionary(&input)?;
    let sigs = parse_dictionary(&sigs)?;
    let Some(ListEntry::InnerList(list)) = input.get(LABEL) else {
        return err("no sig1 in Signature-Input");
    };
    let Some(ListEntry::Item(Item {
        bare_item: BareItem::ByteSequence(sig),
        ..
    })) = sigs.get(LABEL)
    else {
        return err("no sig1 in Signature");
    };
    let mut components = Vec::with_capacity(list.items.len());
    for item in &list.items {
        match &item.bare_item {
            BareItem::String(s) if item.params.is_empty() => {
                components.push(s.as_str().to_string());
            }
            _ => return err("unsupported component"),
        }
    }
    for c in REQUIRED {
        if !components.iter().any(|x| x == c) {
            return err(format!("{c} not covered"));
        }
    }
    let mut created = None;
    let mut keyid = None;
    let mut expires = None;
    for (name, value) in &list.params {
        match (name.as_str(), value) {
            ("created", BareItem::Integer(n)) => created = Some(i64::from(*n)),
            ("expires", BareItem::Integer(n)) => expires = Some(i64::from(*n)),
            ("keyid", BareItem::String(s)) => keyid = Some(s.as_str()),
            ("alg", BareItem::String(s)) if s.as_str() == "ed25519" => {}
            ("alg", _) => return err("unsupported alg"),
            _ => {}
        }
    }
    let created = created.ok_or(Error("no created".into()))?;
    let now_i = i64::try_from(now).or(err("time"))?;
    if created.abs_diff(now_i) > max_skew {
        return err("created out of range");
    }
    if expires.is_some_and(|e| e < now_i) {
        return err("expired");
    }
    let key = keyid
        .and_then(PublicKey::from_id)
        .ok_or(Error("bad keyid".into()))?;

    let digest_header = joined_header(parts, "content-digest");
    if !body.is_empty() || digest_header.is_some() {
        if !components.iter().any(|x| x == "content-digest") {
            return err("content-digest not covered");
        }
        check_digest(digest_header.as_deref(), body)?;
    }

    let input = serialize_inner_list(list);
    let base = signature_base(parts, &components, &input)?;
    if hs_identity::verify(&key, base.as_bytes(), sig) {
        Ok(key)
    } else {
        err("bad signature")
    }
}

fn check_digest(header: Option<&str>, body: &[u8]) -> Result<(), Error> {
    let header = header.ok_or(Error("no Content-Digest".into()))?;
    let dict = parse_dictionary(header)?;
    let Some(ListEntry::Item(Item {
        bare_item: BareItem::ByteSequence(want),
        ..
    })) = dict.get("sha-256")
    else {
        return err("no sha-256 digest");
    };
    let got = digest::digest(&digest::SHA256, body);
    if hs_core::ct_eq(want, got.as_ref()) {
        Ok(())
    } else {
        err("digest mismatch")
    }
}

/// An RFC 9651 dictionary. If a key comes more than once, the last one wins.
fn parse_dictionary(s: &str) -> Result<Dictionary, Error> {
    sfv::Parser::new(s)
        .parse::<Dictionary>()
        .map_err(|e| Error(format!("structured field: {e}")))
}

/// The serialized inner list, as used in `Signature-Input` and in the
/// `@signature-params` line of the signature base.
fn serialize_inner_list(list: &InnerList) -> String {
    let mut ser = sfv::ListSerializer::new();
    let mut inner = ser.inner_list();
    inner.items(&list.items);
    let _ = inner.finish().parameters(&list.params);
    ser.finish().unwrap_or_default()
}

fn key_of(name: &'static str) -> Key {
    sfv::KeyRef::constant(name).to_owned()
}

/// All values of a header joined by `, `, as RFC 9421 §2.1 says.
fn joined_header(parts: &Parts, name: &str) -> Option<String> {
    let mut values = parts.headers.get_all(name).iter().peekable();
    values.peek()?;
    let mut out = Vec::new();
    for v in values {
        out.push(v.to_str().ok()?.trim().to_string());
    }
    Some(out.join(", "))
}

/// Build the RFC 9421 §2.5 signature base.
fn signature_base(parts: &Parts, components: &[String], params: &str) -> Result<String, Error> {
    let mut base = String::new();
    for c in components {
        let value = match c.as_str() {
            "@method" => parts.method.as_str().to_string(),
            "@authority" => authority(parts)?,
            "@path" => match parts.uri.path() {
                "" => "/".to_string(),
                p => p.to_string(),
            },
            "@query" => format!("?{}", parts.uri.query().unwrap_or("")),
            "@target-uri" => parts.uri.to_string(),
            "@scheme" => parts
                .uri
                .scheme_str()
                .ok_or(Error("no scheme".into()))?
                .into(),
            "@request-target" => parts
                .uri
                .path_and_query()
                .map(|p| p.as_str().to_string())
                .ok_or(Error("no path".into()))?,
            s if s.starts_with('@') => return err(format!("unsupported component {s}")),
            name => {
                if name.bytes().any(|b| b.is_ascii_uppercase()) {
                    return err("component names must be lowercase");
                }
                joined_header(parts, name).ok_or(Error(format!("missing header {name}")))?
            }
        };
        if value.contains('\n') || value.contains('\r') {
            return err("newline in component");
        }
        base.push_str(&format!("\"{c}\": {value}\n"));
    }
    base.push_str(&format!("\"@signature-params\": {params}"));
    Ok(base)
}

/// The target authority, lowercase, from the URI or else the Host header.
fn authority(parts: &Parts) -> Result<String, Error> {
    let a = match parts.uri.authority() {
        Some(a) => a.as_str().to_string(),
        None => parts
            .headers
            .get(http::header::HOST)
            .and_then(|h| h.to_str().ok())
            .ok_or(Error("no authority".into()))?
            .trim()
            .to_string(),
    };
    Ok(a.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(uri: &str, body: &[u8], key: &Keypair) -> Parts {
        let (mut parts, ()) = http::Request::post(uri).body(()).unwrap().into_parts();
        sign(&mut parts, body, key, 1_000_000).unwrap();
        parts
    }

    #[test]
    fn round_trip() {
        let key = Keypair::from_seed([3; 32]);
        let body = b"{\"hello\": \"world\"}";
        let parts = request("https://Node.Example:8443/inbox?a=1", body, &key);
        let input = parts.headers["signature-input"].to_str().unwrap();
        assert!(input.starts_with(
            "sig1=(\"@method\" \"@authority\" \"@path\" \"@query\" \"content-digest\");created=1000000;keyid=\""
        ));
        assert!(input.ends_with("\";alg=\"ed25519\""));
        assert_eq!(verify(&parts, body, 1_000_010, 60), Ok(key.public()));
        // Too old, or from the future.
        verify(&parts, body, 1_000_100, 60).unwrap_err();
        verify(&parts, body, 999_000, 60).unwrap_err();

        // Empty body: no digest, still verifies.
        let parts = request("https://node.example/x", b"", &key);
        assert!(!parts.headers.contains_key("content-digest"));
        assert_eq!(verify(&parts, b"", 1_000_000, 60), Ok(key.public()));
        verify(&parts, b"x", 1_000_000, 60).unwrap_err();
    }

    #[test]
    fn host_header_authority() {
        let key = Keypair::from_seed([4; 32]);
        let (mut parts, ()) = http::Request::get("/a?b")
            .header("host", "node.example")
            .body(())
            .unwrap()
            .into_parts();
        sign(&mut parts, b"", &key, 50).unwrap();
        assert_eq!(verify(&parts, b"", 50, 5), Ok(key.public()));
        parts
            .headers
            .insert("host", HeaderValue::from_static("other.example"));
        verify(&parts, b"", 50, 5).unwrap_err();
    }

    #[test]
    fn tampering_fails() {
        let key = Keypair::from_seed([3; 32]);
        let body = b"payload";
        let parts = request("https://node.example/inbox?a=1", body, &key);

        verify(&parts, b"payloae", 1_000_000, 60).unwrap_err();

        let mut p = parts.clone();
        p.uri = "https://node.example/outbox?a=1".parse().unwrap();
        verify(&p, body, 1_000_000, 60).unwrap_err();

        let mut p = parts.clone();
        p.uri = "https://node.example/inbox?a=2".parse().unwrap();
        verify(&p, body, 1_000_000, 60).unwrap_err();

        let mut p = parts.clone();
        p.method = http::Method::PUT;
        verify(&p, body, 1_000_000, 60).unwrap_err();

        // A new digest for a new body: the signature no longer matches.
        let mut p = parts.clone();
        p.headers.insert(
            "content-digest",
            HeaderValue::from_str(&content_digest(b"evil")).unwrap(),
        );
        verify(&p, b"evil", 1_000_000, 60).unwrap_err();

        // Another key that says it has the same keyid.
        let other = Keypair::from_seed([5; 32]);
        let forged = request("https://node.example/inbox?a=1", body, &other);
        let mut p = parts.clone();
        p.headers
            .insert("signature", forged.headers["signature"].clone());
        verify(&p, body, 1_000_000, 60).unwrap_err();

        // Headers full of junk do not cause a panic.
        let mut p = parts.clone();
        p.headers.insert(
            "signature-input",
            HeaderValue::from_static("sig1=(\"@method"),
        );
        verify(&p, body, 1_000_000, 60).unwrap_err();
    }

    #[test]
    fn content_digest_value() {
        // RFC 9530 §B.1 example body.
        assert_eq!(
            content_digest(b"{\"hello\": \"world\"}"),
            "sha-256=:X48E9qOokqqrvdts8nOJRJN3OWDUoyWxBf7kbu9DBPE=:"
        );
    }

    /// RFC 9421 Appendix B.2.6: signing the test request with test-key-ed25519.
    #[test]
    fn rfc9421_b26() {
        let seed = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode("n4Ni-HpISpVObnQMW0wOhCKROaIKqKtW_2ZYb2p9KcU")
            .unwrap();
        let key = Keypair::from_seed(seed.try_into().unwrap());
        let public = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode("JrQLj5P_89iXES9-vFgrIy29clF9CC_oPPsw3c5D0bs")
            .unwrap();
        assert_eq!(key.public().0.to_vec(), public);

        let (parts, ()) = http::Request::post("/foo?param=Value&Pet=dog")
            .header("host", "example.com")
            .header("date", "Tue, 20 Apr 2021 02:07:55 GMT")
            .header("content-type", "application/json")
            .header(
                "content-digest",
                "sha-512=:WZDPaVn/7XgHaAy8pmojAkGWoRx2UFChF41A2svX+TaPm+AbwAgBWnrIiYllu7BNNyealdVLvRwEmTHWXvJwew==:",
            )
            .header("content-length", "18")
            .body(())
            .unwrap()
            .into_parts();
        let components: Vec<String> = [
            "date",
            "@method",
            "@path",
            "@authority",
            "content-type",
            "content-length",
        ]
        .iter()
        .map(std::string::ToString::to_string)
        .collect();
        let mut params = Parameters::new();
        params.insert(
            key_of("created"),
            BareItem::Integer(Integer::constant(1_618_884_473)),
        );
        params.insert(
            key_of("keyid"),
            BareItem::String(sfv::StringRef::constant("test-key-ed25519").to_owned()),
        );
        let list = InnerList::with_params(
            components
                .iter()
                .map(|c| Item::new(sfv::String::from_string(c.clone()).unwrap()))
                .collect(),
            params,
        );
        let input = serialize_inner_list(&list);
        let base = signature_base(&parts, &components, &input).unwrap();
        assert_eq!(
            base,
            "\"date\": Tue, 20 Apr 2021 02:07:55 GMT\n\
             \"@method\": POST\n\
             \"@path\": /foo\n\
             \"@authority\": example.com\n\
             \"content-type\": application/json\n\
             \"content-length\": 18\n\
             \"@signature-params\": (\"date\" \"@method\" \"@path\" \"@authority\" \
             \"content-type\" \"content-length\");created=1618884473\
             ;keyid=\"test-key-ed25519\""
        );
        let expected = STANDARD
            .decode(
                "wqcAqbmYJ2ji2glfAMaRy4gruYYnx2nEFN2HN6jrnDnQCK1u02Gb04v9EDgwUPiu4A0w6vuQv5lIp5WPpBKRCw==",
            )
            .unwrap();
        // Ed25519 is deterministic: we make the exact signature in the RFC.
        assert_eq!(key.sign(base.as_bytes()), expected);
        assert!(hs_identity::verify(
            &key.public(),
            base.as_bytes(),
            &expected
        ));

        // The header from the RFC comes back unchanged through the parser and
        // serializer.
        let header = format!("sig-b26={input}");
        let dict = parse_dictionary(&header).unwrap();
        let Some(ListEntry::InnerList(parsed)) = dict.get("sig-b26") else {
            panic!("no member");
        };
        assert_eq!(serialize_inner_list(parsed), input);
    }
}
