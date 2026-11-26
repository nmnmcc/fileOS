// SPDX-License-Identifier: AGPL-3.0-only

//! The fileOS photos app.
//!
//! It watches `/photos`. For each JPEG or PNG file that changes, it reads the
//! size of the image from its header and keeps it in the local key-value
//! store, under `photo:<path>`, as `<width>x<height>`. The index is on this
//! node only and can be made again at any time.
//!
//! At its own origin, it gives a page with the photos it found.

use std::fmt::Write as _;

use fos_app::app::exports::fos::app::events::Guest;
use fos_app::app::exports::wasi::http::incoming_handler::Guest as Http;
use fos_app::app::fos::app::{fs, kv, log};
use fos_app::app::wasi::http::types::{IncomingRequest, ResponseOutparam};
use fos_app::http::{Request, Response};

struct Photos;

/// The page of the app, at its own origin: the photos it found.
impl Http for Photos {
    fn handle(request: IncomingRequest, out: ResponseOutparam) {
        fos_app::http::serve(request, out, page);
    }
}

fn page(req: &Request) -> Response {
    if req.method != "GET" {
        return Response::text(405, "only GET\n");
    }
    if req.path != "/" {
        return Response::text(404, "not found\n");
    }
    let mut rows = String::new();
    for key in kv::keys("photo:") {
        let size = kv::get(&key).map(|v| String::from_utf8_lossy(&v).into_owned());
        let _ = write!(
            rows,
            "<li>{} <small>{}</small></li>",
            escape(key.trim_start_matches("photo:")),
            escape(&size.unwrap_or_default())
        );
    }
    let who = req.user().map_or_else(|| "no one".to_string(), escape);
    Response::html(
        200,
        format!(
            "<!doctype html><meta charset=utf-8><title>Photos</title>\
             <h1>Photos</h1><p>Signed in: {who}</p><ul>{rows}</ul>"
        ),
    )
}

/// Text that is safe to put into HTML.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Guest for Photos {
    fn on_change(path: String) {
        let extension = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
        if !matches!(extension.as_deref(), Some("jpg" | "jpeg" | "png")) {
            return;
        }
        let key = format!("photo:{path}");
        match fs::read(&path) {
            Ok(bytes) => match image_size(&bytes) {
                Some((w, h)) => {
                    kv::set(&key, format!("{w}x{h}").as_bytes());
                    log::info(&format!("indexed {path}: {w}x{h}"));
                }
                None => log::info(&format!("{path} is not an image this app can read")),
            },
            // The file was deleted.
            Err(fs::Error::NotFound) => kv::delete(&key),
            Err(e) => log::info(&format!("cannot read {path}: {e:?}")),
        }
    }
}

fos_app::app::export_app!(Photos);

/// The width and height of a PNG or JPEG image, from its header.
#[must_use]
pub fn image_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") && bytes.get(12..16) == Some(b"IHDR") {
        let w = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
        let h = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
        return Some((w, h));
    }
    if !bytes.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    // Walk the JPEG segments to the first "start of frame".
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        let len = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
        let is_sof = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
        if is_sof {
            let h = u16::from_be_bytes(bytes.get(i + 5..i + 7)?.try_into().ok()?);
            let w = u16::from_be_bytes(bytes.get(i + 7..i + 9)?.try_into().ok()?);
            return Some((u32::from(w), u32::from(h)));
        }
        i += 2 + len;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::image_size;

    #[test]
    fn reads_png_and_jpeg_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\x0dIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_size(&png), Some((640, 480)));

        // SOI, an APP0 segment of 4 bytes, then SOF0 with height 300 and width 400.
        let jpeg = [
            0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 17, 8, 1, 44, 1, 144,
        ];
        assert_eq!(image_size(&jpeg), Some((400, 300)));
        assert_eq!(image_size(b"not an image"), None);
    }
}
