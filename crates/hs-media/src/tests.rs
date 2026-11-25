// SPDX-License-Identifier: AGPL-3.0-only

//! Tests with images built in memory.

use super::*;
use zune_jpeg::zune_core::bit_depth::BitDepth;
use zune_jpeg::zune_core::colorspace::ColorSpace;
use zune_jpeg::zune_core::options::EncoderOptions;

/// A TIFF field: tag, type, count and value bytes (in file byte order).
struct F(u16, u16, u32, Vec<u8>);

/// Build a TIFF structure with IFD0, an Exif IFD and a GPS IFD.
fn tiff(le: bool) -> Vec<u8> {
    let u16b = |v: u16| if le { v.to_le_bytes() } else { v.to_be_bytes() }.to_vec();
    let u32b = |v: u32| if le { v.to_le_bytes() } else { v.to_be_bytes() }.to_vec();
    let text = |s: &str| [s.as_bytes(), b"\0"].concat();
    let rat = |v: [(u32, u32); 3]| {
        v.iter()
            .flat_map(|&(n, d)| [u32b(n), u32b(d)].concat())
            .collect::<Vec<u8>>()
    };
    let size = |n: usize| 2 + n * 12 + 4;
    // Layout: header, IFD0 (5 fields), Exif IFD (2), GPS IFD (4), data.
    let (ifd0, exif, gps) = (8, 8 + size(5), 8 + size(5) + size(2));
    let data_start = gps + size(4);
    let ifds = vec![
        vec![
            F(0x010F, 2, 6, text("Canon")),
            F(0x0110, 2, 13, text("Canon EOS R5")),
            F(0x0112, 3, 1, [u16b(6), vec![0, 0]].concat()),
            F(0x8769, 4, 1, u32b(exif as u32)),
            F(0x8825, 4, 1, u32b(gps as u32)),
        ],
        vec![
            F(0x9003, 2, 20, text("2024:05:01 10:00:00")),
            F(0x9011, 2, 7, text("+02:00")),
        ],
        vec![
            F(1, 2, 2, text("S")),
            F(2, 5, 3, rat([(33, 1), (51, 1), (5400, 100)])),
            F(3, 2, 2, text("E")),
            F(4, 5, 3, rat([(151, 1), (12, 1), (3600, 100)])),
        ],
    ];
    let mut out = if le {
        b"II*\0".to_vec()
    } else {
        b"MM\0*".to_vec()
    };
    out.extend(u32b(ifd0 as u32));
    let mut data: Vec<u8> = Vec::new();
    for fields in &ifds {
        out.extend(u16b(fields.len() as u16));
        for F(tag, ty, count, value) in fields {
            out.extend(u16b(*tag));
            out.extend(u16b(*ty));
            out.extend(u32b(*count));
            if value.len() <= 4 {
                let mut v = value.clone();
                v.resize(4, 0);
                out.extend(v);
            } else {
                out.extend(u32b((data_start + data.len()) as u32));
                data.extend(value);
            }
        }
        out.extend(u32b(0));
    }
    assert_eq!(out.len(), data_start);
    out.extend(data);
    out
}

/// A JPEG of `w`×`h` whose left half is red and right half blue.
fn jpeg(w: u16, h: u16, exif: Option<Vec<u8>>) -> Vec<u8> {
    let mut px = Vec::new();
    for _ in 0..h {
        for x in 0..w {
            px.extend(if x < w / 2 { [255, 0, 0] } else { [0, 0, 255] });
        }
    }
    let mut out = Vec::new();
    let mut e = jpeg_encoder::Encoder::new(&mut out, 90);
    if let Some(t) = exif {
        e.add_app_segment(1, [b"Exif\0\0".as_slice(), &t].concat())
            .unwrap();
    }
    e.encode(&px, w, h, jpeg_encoder::ColorType::Rgb).unwrap();
    out
}

fn png(w: usize, h: usize, cs: ColorSpace) -> Vec<u8> {
    let px = vec![128u8; w * h * cs.num_components()];
    let mut out = Vec::new();
    zune_png::PngEncoder::new(&px, EncoderOptions::new(w, h, cs, BitDepth::Eight))
        .encode(&mut out)
        .unwrap();
    out
}

#[test]
fn exif_both_byte_orders() {
    for le in [true, false] {
        let e = Exif::parse(&tiff(le));
        assert_eq!(e.time.as_deref(), Some("2024-05-01T08:00:00Z"));
        let (lat, lon) = e.place.unwrap();
        assert!((lat + 33.865).abs() < 1e-9, "{lat}");
        assert!((lon - 151.21).abs() < 1e-9, "{lon}");
        assert_eq!(e.camera().as_deref(), Some("Canon EOS R5"));
        assert_eq!(e.orientation, Some(6));
    }
}

#[test]
fn jpeg_probe_and_json() {
    let m = probe(&jpeg(40, 20, Some(tiff(false)))).unwrap();
    assert_eq!((m.media_type, m.width, m.height), ("image/jpeg", 40, 20));
    assert_eq!(
        m.to_json(),
        serde_json::json!({
            "media_type": "image/jpeg", "width": 40, "height": 20,
            "time": "2024-05-01T08:00:00Z",
            "place": {"lat": -33.865, "lon": 151.21},
            "camera": "Canon EOS R5", "orientation": 6,
        })
    );
    let plain = probe(&jpeg(8, 8, None)).unwrap();
    assert_eq!(
        plain.to_json(),
        serde_json::json!({"media_type": "image/jpeg", "width": 8, "height": 8})
    );
}

#[test]
fn other_formats() {
    let m = probe(&png(7, 5, ColorSpace::RGB)).unwrap();
    assert_eq!((m.media_type, m.width, m.height), ("image/png", 7, 5));

    let gif = b"GIF89a\x0a\x00\x14\x00\x00\x00\x00;";
    let m = probe(gif).unwrap();
    assert_eq!((m.media_type, m.width, m.height), ("image/gif", 10, 20));

    let riff = |chunk: &[u8], body: &[u8]| {
        let mut b = b"RIFF\0\0\0\0WEBP".to_vec();
        b.extend(chunk);
        b.extend((body.len() as u32).to_le_bytes());
        b.extend(body);
        b
    };
    let vp8 = riff(b"VP8 ", &[0, 0, 0, 0x9D, 0x01, 0x2A, 100, 0, 50, 0]);
    assert_eq!(probe(&vp8).map(|m| (m.width, m.height)), Some((100, 50)));
    // VP8L: width-1 = 99, height-1 = 49 packed in 14-bit fields.
    let v = 99u32 | 49 << 14;
    let vp8l = riff(b"VP8L", &[[0x2F].as_slice(), &v.to_le_bytes()].concat());
    assert_eq!(probe(&vp8l).map(|m| (m.width, m.height)), Some((100, 50)));
    let vp8x = riff(b"VP8X", &[0, 0, 0, 0, 0xFF, 0x0F, 0, 0x7F, 0x00, 0]);
    assert_eq!(probe(&vp8x).map(|m| (m.width, m.height)), Some((4096, 128)));

    // HEIC: ftyp, then meta > iprp > ipco > two ispe (a thumbnail and the image).
    let bx = |ty: &[u8], body: &[u8]| [&((body.len() + 8) as u32).to_be_bytes(), ty, body].concat();
    let ispe = |w: u32, h: u32| {
        bx(
            b"ispe",
            &[[0; 4].as_slice(), &w.to_be_bytes(), &h.to_be_bytes()].concat(),
        )
    };
    let ipco = bx(b"ipco", &[ispe(320, 240), ispe(4032, 3024)].concat());
    let meta = bx(b"meta", &[[0; 4].as_slice(), &bx(b"iprp", &ipco)].concat());
    let heic = [bx(b"ftyp", b"heic\0\0\0\0mif1heic"), meta].concat();
    let m = probe(&heic).unwrap();
    assert_eq!(
        (m.media_type, m.width, m.height),
        ("image/heic", 4032, 3024)
    );

    assert_eq!(probe(b"hello"), None);
}

#[test]
fn hostile_input_does_not_panic() {
    let samples = [
        jpeg(16, 16, Some(tiff(true))),
        png(4, 4, ColorSpace::RGBA),
        b"RIFF\0\0\0\0WEBPVP8X\xff\xff\xff\xff".to_vec(),
        b"\0\0\0\x01ftypheic\xff\xff\xff\xff\xff\xff\xff\xff".to_vec(),
    ];
    for s in &samples {
        for cut in 0..s.len() {
            let _ = probe(&s[..cut]);
            let _ = thumbnail(&s[..cut], 8);
        }
        let mut flipped = s.clone();
        for i in 0..flipped.len() {
            flipped[i] ^= 0xA5;
            let _ = probe(&flipped);
            flipped[i] ^= 0xA5;
        }
    }
    for i in 0..tiff(true).len() {
        let mut t = tiff(true);
        t[i] = 0xFF;
        let _ = Exif::parse(&t);
    }
}

#[test]
fn thumbnails() {
    // 40×20 with orientation 6 (rotate 90° clockwise): becomes 5×10 at max 10.
    let (bytes, ty) = thumbnail(&jpeg(40, 20, Some(tiff(true))), 10).unwrap();
    assert_eq!(ty, "image/jpeg");
    let m = probe(&bytes).unwrap();
    assert_eq!((m.width, m.height), (5, 10));
    // Red was on the left, so after turning clockwise it is on top.
    let mut d = zune_jpeg::JpegDecoder::new(zune_jpeg::zune_core::bytestream::ZCursor::new(&bytes));
    let px = d.decode().unwrap();
    let (top, bottom) = (&px[..3], &px[px.len() - 3..]);
    assert!(top[0] > 200 && top[2] < 60, "{top:?}");
    assert!(bottom[2] > 200 && bottom[0] < 60, "{bottom:?}");

    // Small images are not enlarged.
    let (bytes, _) = thumbnail(&jpeg(8, 4, None), 100).unwrap();
    assert_eq!(probe(&bytes).map(|m| (m.width, m.height)), Some((8, 4)));

    // PNG with alpha stays PNG; without alpha becomes JPEG.
    let (bytes, ty) = thumbnail(&png(30, 60, ColorSpace::RGBA), 6).unwrap();
    assert_eq!(ty, "image/png");
    assert_eq!(probe(&bytes).map(|m| (m.width, m.height)), Some((3, 6)));
    let (_, ty) = thumbnail(&png(30, 60, ColorSpace::Luma), 6).unwrap();
    assert_eq!(ty, "image/jpeg");

    assert_eq!(thumbnail(b"GIF89a\x01\x00\x01\x00", 10), None);
    assert_eq!(thumbnail(&jpeg(8, 8, None), 0), None);
}
