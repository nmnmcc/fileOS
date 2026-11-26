// SPDX-License-Identifier: AGPL-3.0-only

//! The fileOS photos app.
//!
//! It watches `/photos`. For each JPEG or PNG file that changes, it reads the
//! size of the image from its header and keeps it in the local key-value
//! store, under `photo:<path>`, as `<width>x<height>`. The index is on this
//! node only and can be made again at any time.

use fos_app::app::exports::fos::app::events::Guest;
use fos_app::app::fos::app::{fs, kv, log};

struct Photos;

impl Guest for Photos {
    fn on_change(path: String) {
        let lower = path.to_ascii_lowercase();
        if !(lower.ends_with(".jpg") || lower.ends_with(".jpeg") || lower.ends_with(".png")) {
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
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0, 4, 0, 0, 0xFF, 0xC0, 0, 17, 8, 1, 44, 1, 144];
        assert_eq!(image_size(&jpeg), Some((400, 300)));
        assert_eq!(image_size(b"not an image"), None);
    }
}
