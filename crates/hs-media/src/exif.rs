// SPDX-License-Identifier: AGPL-3.0-only

//! The EXIF fields the index uses, read with `kamadak-exif`.

use exif::{In, Tag, Value};
use time::macros::format_description;

/// The EXIF fields homeserver uses.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Exif {
    /// When the photo was taken, ISO 8601: in UTC (`...Z`) if the camera
    /// recorded its offset, otherwise local time without a zone.
    pub time: Option<String>,
    /// Latitude and longitude in degrees (south and west negative).
    pub place: Option<(f64, f64)>,
    /// The camera maker.
    pub make: Option<String>,
    /// The camera model.
    pub model: Option<String>,
    /// How to turn the stored pixels upright (1 to 8, 1 being upright).
    pub orientation: Option<u8>,
}

impl Exif {
    /// Read a TIFF structure (the body of an EXIF block, after `Exif\0\0`).
    /// Fields that are missing or malformed stay `None`.
    pub fn parse(tiff: &[u8]) -> Exif {
        exif::Reader::new()
            .read_raw(tiff.to_vec())
            .map_or_else(|_| Exif::default(), |e| Exif::from_exif(&e))
    }

    /// Pick the fields homeserver uses out of what `kamadak-exif` read.
    pub(crate) fn from_exif(e: &exif::Exif) -> Exif {
        let value = |tag| e.get_field(tag, In::PRIMARY).map(|f| &f.value);
        let text = |tag| match value(tag)? {
            Value::Ascii(v) => {
                let s = String::from_utf8_lossy(v.first()?);
                let s = s.trim_matches(|c: char| c == '\0' || c.is_whitespace());
                (!s.is_empty()).then(|| s.to_string())
            }
            _ => None,
        };
        // Three unsigned rationals (degrees, minutes, seconds) as degrees,
        // rounded to 1e-7 degrees (about 1 cm) to drop float noise.
        let degrees = |tag| match value(tag)? {
            Value::Rational(v) if v.len() == 3 && v.iter().all(|r| r.denom != 0) => {
                let d = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
                let d = (d * 1e7).round() / 1e7;
                d.is_finite().then_some(d)
            }
            _ => None,
        };
        let place = match (degrees(Tag::GPSLatitude), degrees(Tag::GPSLongitude)) {
            (Some(mut lat), Some(mut lon)) => {
                if text(Tag::GPSLatitudeRef).as_deref() == Some("S") {
                    lat = -lat;
                }
                if text(Tag::GPSLongitudeRef).as_deref() == Some("W") {
                    lon = -lon;
                }
                ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon))
                    .then_some((lat, lon))
            }
            _ => None,
        };
        let orientation = match value(Tag::Orientation) {
            Some(Value::Short(v)) => v
                .first()
                .and_then(|&o| u8::try_from(o).ok())
                .filter(|o| (1..=8).contains(o)),
            _ => None,
        };
        Exif {
            time: text(Tag::DateTimeOriginal)
                .or_else(|| text(Tag::DateTime))
                .and_then(|d| iso8601(&d, text(Tag::OffsetTimeOriginal).as_deref())),
            place,
            make: text(Tag::Make),
            model: text(Tag::Model),
            orientation,
        }
    }

    /// `Make Model`, or just the model when it already names the maker.
    pub fn camera(&self) -> Option<String> {
        match (&self.make, &self.model) {
            (Some(make), Some(model)) => {
                if model.to_lowercase().starts_with(&make.to_lowercase()) {
                    Some(model.clone())
                } else {
                    Some(format!("{make} {model}"))
                }
            }
            (Some(one), None) | (None, Some(one)) => Some(one.clone()),
            (None, None) => None,
        }
    }
}

/// `YYYY:MM:DD HH:MM:SS` (and an optional `+HH:MM` offset) as ISO 8601.
fn iso8601(dt: &str, offset: Option<&str>) -> Option<String> {
    let local = time::PrimitiveDateTime::parse(
        dt.get(..19)?,
        format_description!("[year]:[month]:[day] [hour]:[minute]:[second]"),
    )
    .ok()?;
    let offset = offset.and_then(|o| {
        time::UtcOffset::parse(
            o.trim(),
            format_description!("[offset_hour sign:mandatory]:[offset_minute]"),
        )
        .ok()
    });
    match offset {
        None => local
            .format(format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second]"
            ))
            .ok(),
        Some(o) => local
            .assume_offset(o)
            .to_offset(time::UtcOffset::UTC)
            .format(format_description!(
                "[year]-[month]-[day]T[hour]:[minute]:[second]Z"
            ))
            .ok(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(
            iso8601("2024:01:01 01:30:00", Some("+02:00")).as_deref(),
            Some("2023-12-31T23:30:00Z")
        );
        assert_eq!(
            iso8601("2024:02:28 23:00:00", Some("-01:30")).as_deref(),
            Some("2024-02-29T00:30:00Z")
        );
        assert_eq!(
            iso8601("2024:06:01 12:00:00", None).as_deref(),
            Some("2024-06-01T12:00:00")
        );
        assert_eq!(iso8601("0000:00:00 00:00:00", None), None);
        assert_eq!(iso8601("    :  :     :  :  ", None), None);
    }
}
