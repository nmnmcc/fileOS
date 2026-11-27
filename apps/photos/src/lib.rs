// SPDX-License-Identifier: AGPL-3.0-only

//! The fileOS photos app.
//!
//! It watches `/photos`. For each JPEG or PNG file that changes, it reads the
//! size of the image from its header and keeps it in the local key-value
//! store, under `photo:<path>`, as `<width>x<height>`. The index is on this
//! node only and can be made again at any time.
//!
//! At its own origin, it gives a page with the photos it found, the photos
//! themselves, and upload into `/photos`.

use std::fmt::Write as _;

use fileos_app::app::exports::fileos::app::events::Guest;
use fileos_app::app::exports::wasi::http::incoming_handler::Guest as Http;
use fileos_app::app::fileos::app::{fs, kv, log};
use fileos_app::app::wasi::http::types::{IncomingRequest, ResponseOutparam};
use fileos_app::http::{Request, Response};

struct Photos;

/// The pages of the app, at its own origin: the photos it found, the photos
/// themselves, and upload.
impl Http for Photos {
    fn handle(request: IncomingRequest, out: ResponseOutparam) {
        fileos_app::http::serve(request, out, route);
    }
}

/// The folder of the photos.
const ROOT: &str = "/photos";

fn route(req: &Request) -> Response {
    let (path, query) = req.path.split_once('?').unwrap_or((&req.path, ""));
    match (req.method.as_str(), path) {
        ("GET", "/") => gallery(req),
        ("GET", "/style.css") => Response::new(200, "text/css; charset=utf-8", STYLE),
        ("GET", "/app.js") => Response::new(200, "text/javascript; charset=utf-8", SCRIPT),
        ("GET", "/photo") => photo(query),
        ("POST", "/.well-known/fileos/schedule/reindex")
            if req.header("fileos-event") == Some("schedule") =>
        {
            reindex()
        }
        ("PUT", upload) => match upload.strip_prefix("/upload/") {
            Some(name) if valid_name(name) => match fs::write(&format!("{ROOT}/{name}"), &req.body)
            {
                Ok(()) => Response::text(201, "uploaded\n"),
                Err(e) => Response::text(500, format!("not uploaded: {e:?}\n")),
            },
            _ => Response::text(
                400,
                "a name has letters, digits, - _ . and ends in .jpg, .jpeg or .png\n",
            ),
        },
        ("GET", _) => Response::text(404, "not found\n"),
        _ => Response::text(405, "only GET and PUT\n"),
    }
}

/// Indexes every photo again, from the files. The kernel asks for this once
/// a day (see the manifest).
fn reindex() -> Response {
    let mut count = 0;
    for name in fs::list_dir(ROOT).unwrap_or_default() {
        let path = format!("{ROOT}/{name}");
        index(&path);
        count += 1;
    }
    Response::text(200, format!("indexed {count} files\n"))
}

/// All the photos that the index has, with their size.
fn gallery(req: &Request) -> Response {
    let mut items = String::new();
    for key in kv::keys("photo:") {
        let path = key.trim_start_matches("photo:");
        let size = kv::get(&key).map(|v| String::from_utf8_lossy(&v).into_owned());
        let _ = write!(
            items,
            "<figure><img src=\"/photo?path={}\" alt=\"\"><figcaption>{} <small>{}</small></figcaption></figure>",
            escape(&percent_encode(path)),
            escape(path),
            escape(&size.unwrap_or_default())
        );
    }
    let who = req.user().map_or_else(
        || "Not signed in".to_string(),
        |u| format!("Signed in: {}", escape(u)),
    );
    Response::html(
        200,
        format!(
            "<!doctype html><meta charset=utf-8><meta name=viewport content=\"width=device-width\">\
             <link rel=stylesheet href=/style.css><script src=/app.js defer></script><title>Photos</title>\
             <header>{who}</header><h1>Photos</h1>\
             <p><input type=file id=upload accept=\"image/jpeg,image/png\" multiple></p>\
             <div class=grid>{items}</div>"
        ),
    )
}

/// The bytes of one photo, by its path under `/photos`.
fn photo(query: &str) -> Response {
    let path = query
        .split('&')
        .find_map(|p| p.strip_prefix("path="))
        .map_or_else(String::new, percent_decode);
    let inside = path.strip_prefix("/photos/").is_some_and(|rest| {
        !rest
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    });
    if !inside {
        return Response::text(400, "only photos under /photos\n");
    }
    let content_type = if path.to_ascii_lowercase().ends_with(".png") {
        "image/png"
    } else {
        "image/jpeg"
    };
    match fs::read(&path) {
        Ok(bytes) => Response::new(200, content_type, bytes),
        Err(fs::Error::NotFound) => Response::text(404, "not found\n"),
        Err(e) => Response::text(500, format!("cannot read it: {e:?}\n")),
    }
}

/// A name for an upload: letters, digits, `-`, `_` and `.`, ending in an
/// image type that the app reads.
fn valid_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    !name.starts_with('.')
        && name.len() <= 128
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        && [".jpg", ".jpeg", ".png"]
            .iter()
            .any(|ext| lower.ends_with(ext))
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'/' => {
                char::from(b).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%')
            .then(|| bytes.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        if let Some(byte) = hex {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The style of the page, as a file of its own: the kernel's default content
/// security policy lets in no style or script inside the page.
const STYLE: &str = "body{font:16px/1.5 system-ui,sans-serif;margin:0 auto;padding:1rem;max-width:60rem}\
header{color:#666;font-size:.9em}.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(10rem,1fr));gap:1rem}\
figure{margin:0}img{width:100%;height:10rem;object-fit:cover;border-radius:.5rem;background:#8882}\
figcaption{font-size:.85em;overflow-wrap:anywhere}\
@media(prefers-color-scheme:dark){body{background:#111;color:#eee}}";

/// Upload: each chosen file is sent with PUT, then the page is loaded again.
const SCRIPT: &str = "document.getElementById('upload').addEventListener('change', async (e) => {\
for (const file of e.target.files) {\
const name = file.name.replace(/[^A-Za-z0-9._-]/g, '_');\
const r = await fetch('/upload/' + encodeURIComponent(name), { method: 'PUT', body: file });\
if (!r.ok) alert(await r.text());\
}\
location.reload();\
});";

/// Text that is safe to put into HTML.
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

impl Guest for Photos {
    fn on_change(path: String) {
        index(&path);
    }
}

/// Keeps the size of the photo at `path` in the index, or takes it out when
/// the file is gone.
fn index(path: &str) {
    let extension = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    if !matches!(extension.as_deref(), Some("jpg" | "jpeg" | "png")) {
        return;
    }
    let key = format!("photo:{path}");
    match fs::read(path) {
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

fileos_app::app::export_app!(Photos);

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
    fn upload_names_and_paths_are_checked() {
        use super::{percent_decode, percent_encode, valid_name};
        assert!(valid_name("cat.png"));
        assert!(valid_name("IMG_0001.JPG"));
        assert!(!valid_name("../x.png"));
        assert!(!valid_name(".hidden.png"));
        assert!(!valid_name("notes.txt"));
        assert_eq!(
            percent_decode(&percent_encode("/photos/a b.png")),
            "/photos/a b.png"
        );
    }

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
