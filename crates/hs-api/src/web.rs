// SPDX-License-Identifier: AGPL-3.0-only

//! The web app, built into the executable (see `build.rs` for where it comes
//! from). `/` and any other top-level path without a dot get `index.html`.
//! Files named by their content are cached for a year; the rest are checked
//! each time. A brotli copy goes to clients that take it.

use std::sync::OnceLock;

use http::{Method, StatusCode, header};

use crate::util::{ApiError, Result, empty, header as hdr, respond, set_header};
use hs_net::server::Request;

struct File {
    path: &'static str,
    media_type: &'static str,
    /// Named by a digest of its content.
    immutable: bool,
    bytes: &'static [u8],
    br: Option<&'static [u8]>,
}

const FILES: &[File] = include!(concat!(env!("OUT_DIR"), "/web_files.rs"));

/// The web app runs WebAssembly, and its pages show `blob:` and `data:`
/// images.
const CSP: &str = "default-src 'self'; img-src 'self' blob: data:; media-src 'self' blob:; style-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'";

/// `ETag`s, made once for each file.
fn etag(index: usize) -> &'static str {
    static TAGS: OnceLock<Vec<OnceLock<String>>> = OnceLock::new();
    let tags = TAGS.get_or_init(|| FILES.iter().map(|_| OnceLock::new()).collect());
    tags[index].get_or_init(|| format!("\"{}\"", &blake3::hash(FILES[index].bytes).to_hex()[..16]))
}

/// Whether `Accept-Encoding` takes `name` (and not at `q=0`).
fn accepts(req: &Request, name: &str) -> bool {
    hdr(req, header::ACCEPT_ENCODING).is_some_and(|v| {
        v.split(',').any(|item| {
            let mut parts = item.split(';').map(str::trim);
            parts.next() == Some(name)
                && !parts.any(|p| {
                    p.strip_prefix("q=")
                        .and_then(|q| q.trim().parse::<f32>().ok())
                        .is_some_and(|q| q == 0.0)
                })
        })
    })
}

fn find(segs: &[&str]) -> Option<usize> {
    let path = segs.join("/");
    let path = if path.is_empty() { "index.html" } else { &path };
    FILES
        .iter()
        .position(|f| f.path == path)
        .or_else(|| match segs {
            // App routes; unknown paths with an extension are missing files.
            [one] if !one.contains('.') => FILES.iter().position(|f| f.path == "index.html"),
            _ => None,
        })
}

pub fn handle(req: &Request, segs: &[&str]) -> Result {
    let index = find(segs).ok_or_else(ApiError::not_found)?;
    let file = &FILES[index];
    let br = file.br.filter(|_| accepts(req, "br"));
    let tag = match br {
        Some(_) => format!("{}-br\"", etag(index).trim_end_matches('"')),
        None => etag(index).to_owned(),
    };
    let mut r =
        if hdr(req, header::IF_NONE_MATCH).is_some_and(|v| v.split(',').any(|t| t.trim() == tag)) {
            empty(StatusCode::NOT_MODIFIED)
        } else if req.method() == Method::HEAD {
            let mut r = empty(StatusCode::OK);
            set_header(&mut r, header::CONTENT_TYPE, file.media_type);
            r
        } else {
            respond(
                StatusCode::OK,
                file.media_type,
                bytes::Bytes::from_static(br.unwrap_or(file.bytes)),
            )
        };
    if br.is_some() {
        set_header(&mut r, header::CONTENT_ENCODING, "br");
    }
    if file.br.is_some() {
        set_header(&mut r, header::VARY, "accept-encoding");
    }
    set_header(&mut r, header::ETAG, &tag);
    set_header(
        &mut r,
        header::CACHE_CONTROL,
        if file.immutable {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        },
    );
    set_header(&mut r, header::CONTENT_SECURITY_POLICY, CSP);
    set_header(&mut r, header::X_FRAME_OPTIONS, "DENY");
    set_header(&mut r, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_app_and_its_files() {
        let index = FILES.iter().position(|f| f.path == "index.html");
        assert!(index.is_some());
        assert_eq!(find(&[]), index);
        assert_eq!(find(&["settings"]), index);
        assert_eq!(find(&["missing.js"]), None);
        assert_eq!(find(&["a", "b"]), None);
        for (i, f) in FILES.iter().enumerate() {
            let segs: Vec<&str> = f.path.split('/').collect();
            assert_eq!(find(&segs), Some(i), "{}", f.path);
        }
    }
}
