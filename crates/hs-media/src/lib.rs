// SPDX-License-Identifier: AGPL-3.0-only

//! Media metadata and previews.
//!
//! [`probe`] gets an image's type and size from its first bytes. It also
//! reads the EXIF block if there is one (time taken, place, camera,
//! orientation). `imagesize` and `kamadak-exif` read the container formats.
//! [`thumbnail`] decodes JPEG and PNG and writes a small preview. The server
//! runs decoding in a separate process, so a decoder failure does not get out
//! of that process.

mod exif;
mod thumb;

#[doc(inline)]
pub use exif::Exif;
#[doc(inline)]
pub use thumb::{MAX_PIXELS, thumbnail};

use serde_json::{Map, Value as Json};

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// What [`probe`] found out about a file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Media {
    /// The media type, such as `image/jpeg`.
    pub media_type: &'static str,
    /// Width in pixels, as stored (before the orientation is used).
    pub width: u32,
    /// Height in pixels, as stored.
    pub height: u32,
    /// Metadata from the EXIF block, if there is one.
    pub exif: Exif,
}

impl Media {
    /// The fields the index understands: `media_type`, `width`, `height`,
    /// `time`, `place` (`lat`, `lon`), `camera` and `orientation`. Missing
    /// values are left out.
    pub fn to_json(&self) -> Json {
        let mut m = Map::new();
        m.insert("media_type".into(), self.media_type.into());
        if self.width > 0 && self.height > 0 {
            m.insert("width".into(), self.width.into());
            m.insert("height".into(), self.height.into());
        }
        if let Some(t) = &self.exif.time {
            m.insert("time".into(), t.clone().into());
        }
        if let Some((lat, lon)) = self.exif.place {
            m.insert(
                "place".into(),
                serde_json::json!({ "lat": lat, "lon": lon }),
            );
        }
        if let Some(c) = self.exif.camera() {
            m.insert("camera".into(), c.into());
        }
        if let Some(o) = self.exif.orientation {
            m.insert("orientation".into(), o.into());
        }
        Json::Object(m)
    }
}

/// Look at the start of an image to get its media type, size and EXIF metadata.
///
/// Knows JPEG, PNG, GIF, WebP, HEIC and AVIF. The size comes from
/// `imagesize`, the EXIF block from `kamadak-exif`. Returns `None` if the
/// bytes are none of these.
pub fn probe(bytes: &[u8]) -> Option<Media> {
    use ::imagesize::{Compression, ImageType};
    let media_type = match ::imagesize::image_type(bytes).ok()? {
        ImageType::Jpeg => "image/jpeg",
        ImageType::Png => "image/png",
        ImageType::Gif => "image/gif",
        ImageType::Webp => "image/webp",
        ImageType::Heif(Compression::Av1) => "image/avif",
        ImageType::Heif(Compression::Hevc) => "image/heic",
        ImageType::Heif(_) => "image/heif",
        _ => return None,
    };
    let (width, height) = ::imagesize::blob_size(bytes).map_or((0, 0), |s| {
        (
            u32::try_from(s.width).unwrap_or(0),
            u32::try_from(s.height).unwrap_or(0),
        )
    });
    let exif = match media_type {
        "image/gif" => Exif::default(),
        _ => ::exif::Reader::new()
            .read_from_container(&mut std::io::Cursor::new(bytes))
            .map_or_else(|_| Exif::default(), |e| Exif::from_exif(&e)),
    };
    Some(Media {
        media_type,
        width,
        height,
        exif,
    })
}

#[cfg(test)]
mod tests;
