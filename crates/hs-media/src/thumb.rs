// SPDX-License-Identifier: AGPL-3.0-only

//! Thumbnails: decode JPEG or PNG, shrink with a box filter, turn upright
//! from the EXIF orientation, and encode as JPEG (or PNG when the image has
//! transparency).

use zune_jpeg::JpegDecoder;
use zune_jpeg::zune_core::bit_depth::BitDepth;
use zune_jpeg::zune_core::bytestream::ZCursor;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::{DecoderOptions, EncoderOptions};
use zune_png::{PngDecoder, PngEncoder};

/// The largest image, in pixels, that [`thumbnail`] decodes. Bigger images
/// would need too much memory on a small server.
pub const MAX_PIXELS: u64 = 64_000_000;

/// JPEG quality of thumbnails.
const QUALITY: u8 = 80;

/// Decoded pixels, 8 bits per sample.
struct Image {
    w: usize,
    h: usize,
    /// 1 (gray), 2 (gray and alpha), 3 (RGB) or 4 (RGBA).
    channels: usize,
    px: Vec<u8>,
}

/// Make a preview of a JPEG or PNG no larger than `max_side` on either side
/// (never enlarged), upright according to its EXIF orientation.
///
/// Returns the encoded bytes and their media type (`image/jpeg`, or
/// `image/png` for images with transparency), or `None` if the format is not
/// supported, the image is larger than [`MAX_PIXELS`] or it does not decode.
///
/// # Panics
///
/// A decoder bug can panic. Servers processing external images should call
/// this function in a separate process.
pub fn thumbnail(bytes: &[u8], max_side: u32) -> Option<(Vec<u8>, &'static str)> {
    let media = crate::probe(bytes)?;
    if max_side == 0
        || media.width == 0
        || media.height == 0
        || u64::from(media.width) * u64::from(media.height) > MAX_PIXELS
    {
        return None;
    }
    let max_side = usize::try_from(max_side.min(u32::from(u16::MAX))).ok()?;
    let image = match media.media_type {
        "image/jpeg" => decode_jpeg(bytes),
        "image/png" => decode_png(bytes),
        _ => None,
    }?;
    let small = shrink(image, max_side);
    let upright = orient(small, media.exif.orientation.unwrap_or(1));
    encode(&upright)
}

fn options() -> DecoderOptions {
    let side = usize::try_from(MAX_PIXELS)
        .unwrap_or(usize::MAX)
        .min(65_535);
    DecoderOptions::default()
        .set_max_width(side)
        .set_max_height(side)
}

fn decode_jpeg(bytes: &[u8]) -> Option<Image> {
    let mut d = JpegDecoder::new_with_options(
        ZCursor::new(bytes),
        options().jpeg_set_out_colorspace(ColorSpace::RGB),
    );
    d.decode_headers().ok()?;
    let (w, h) = d.dimensions()?;
    let px = d.decode().ok()?;
    image(w, h, px)
}

fn decode_png(bytes: &[u8]) -> Option<Image> {
    let mut d =
        PngDecoder::new_with_options(ZCursor::new(bytes), options().png_set_strip_to_8bit(true));
    d.decode_headers().ok()?;
    let (w, h) = d.dimensions()?;
    let px = d.decode().ok()?.u8()?;
    image(w, h, px)
}

fn image(w: usize, h: usize, px: Vec<u8>) -> Option<Image> {
    let n = w.checked_mul(h)?;
    if n == 0 || !px.len().is_multiple_of(n) {
        return None;
    }
    let channels = px.len() / n;
    (1..=4)
        .contains(&channels)
        .then_some(Image { w, h, channels, px })
}

/// Shrink so neither side exceeds `max`, averaging each output pixel's box.
fn shrink(src: Image, max: usize) -> Image {
    let long = src.w.max(src.h);
    if long <= max {
        return src;
    }
    let w = (src.w * max / long).max(1);
    let h = (src.h * max / long).max(1);
    let c = src.channels;
    let mut px = vec![0u8; w * h * c];
    let mut sum = vec![0u64; c];
    for y in 0..h {
        let (y0, y1) = (y * src.h / h, ((y + 1) * src.h / h).max(y * src.h / h + 1));
        for x in 0..w {
            let (x0, x1) = (x * src.w / w, ((x + 1) * src.w / w).max(x * src.w / w + 1));
            sum.fill(0);
            for sy in y0..y1 {
                let row = &src.px[(sy * src.w + x0) * c..(sy * src.w + x1) * c];
                for p in row.chunks_exact(c) {
                    for (s, v) in sum.iter_mut().zip(p) {
                        *s += u64::from(*v);
                    }
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as u64;
            let out = &mut px[(y * w + x) * c..(y * w + x + 1) * c];
            for (o, s) in out.iter_mut().zip(&sum) {
                *o = u8::try_from((s + count / 2) / count).unwrap_or(u8::MAX);
            }
        }
    }
    Image {
        w,
        h,
        channels: c,
        px,
    }
}

/// Apply an EXIF orientation (1 to 8) so the image is upright.
fn orient(src: Image, o: u8) -> Image {
    let (w, h, c) = (src.w, src.h, src.channels);
    if !(2..=8).contains(&o) {
        return src;
    }
    let (ow, oh) = if o >= 5 { (h, w) } else { (w, h) };
    let mut px = Vec::with_capacity(src.px.len());
    for y in 0..oh {
        for x in 0..ow {
            let (sx, sy) = match o {
                2 => (w - 1 - x, y),
                3 => (w - 1 - x, h - 1 - y),
                4 => (x, h - 1 - y),
                5 => (y, x),
                6 => (y, h - 1 - x),
                7 => (w - 1 - y, h - 1 - x),
                _ => (w - 1 - y, x),
            };
            let i = (sy * w + sx) * c;
            px.extend_from_slice(&src.px[i..i + c]);
        }
    }
    Image {
        w: ow,
        h: oh,
        channels: c,
        px,
    }
}

fn encode(img: &Image) -> Option<(Vec<u8>, &'static str)> {
    let mut out = Vec::new();
    match img.channels {
        1 | 3 => {
            let ty = if img.channels == 1 {
                jpeg_encoder::ColorType::Luma
            } else {
                jpeg_encoder::ColorType::Rgb
            };
            jpeg_encoder::Encoder::new(&mut out, QUALITY)
                .encode(
                    &img.px,
                    u16::try_from(img.w).ok()?,
                    u16::try_from(img.h).ok()?,
                    ty,
                )
                .ok()?;
            Some((out, "image/jpeg"))
        }
        _ => {
            let cs = if img.channels == 2 {
                ColorSpace::LumaA
            } else {
                ColorSpace::RGBA
            };
            let opts = EncoderOptions::new(img.w, img.h, cs, BitDepth::Eight);
            PngEncoder::new(&img.px, opts).encode(&mut out).ok()?;
            Some((out, "image/png"))
        }
    }
}
