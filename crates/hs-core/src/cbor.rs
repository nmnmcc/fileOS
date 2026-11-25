// SPDX-License-Identifier: AGPL-3.0-only

//! Deterministic CBOR (RFC 8949, section 4.2), the small subset we use.
//!
//! Supported: unsigned and negative integers, byte strings, text strings,
//! arrays, maps, `false`, `true` and `null`. No floats, tags or indefinite
//! lengths. Encoding is always deterministic: shortest integer forms and map
//! keys sorted by their encoded bytes. Decoding rejects anything that is not
//! in that form, so every value has exactly one encoding.

use std::fmt;

/// A CBOR value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    /// An unsigned integer.
    Uint(u64),
    /// A negative integer, stored as `-1 - n`.
    Nint(u64),
    /// A byte string.
    Bytes(Vec<u8>),
    /// A UTF-8 text string.
    Text(String),
    /// An array.
    Array(Vec<Value>),
    /// A map. Keys are unique; order is canonicalized on encoding.
    Map(Vec<(Value, Value)>),
    /// A boolean.
    Bool(bool),
    /// `null`.
    Null,
}

/// A decoding error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error(pub &'static str);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "cbor: {}", self.0)
    }
}

impl std::error::Error for Error {}

const MAX_DEPTH: usize = 32;

impl Value {
    /// An integer value, positive or negative.
    pub fn int(n: i64) -> Value {
        if n >= 0 {
            Value::Uint(n as u64)
        } else {
            Value::Nint((-1 - n) as u64)
        }
    }

    /// Encode deterministically.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        self.encode_into(&mut out);
        out
    }

    fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Value::Uint(n) => head(out, 0, *n),
            Value::Nint(n) => head(out, 1, *n),
            Value::Bytes(b) => {
                head(out, 2, b.len() as u64);
                out.extend_from_slice(b);
            }
            Value::Text(s) => {
                head(out, 3, s.len() as u64);
                out.extend_from_slice(s.as_bytes());
            }
            Value::Array(items) => {
                head(out, 4, items.len() as u64);
                for item in items {
                    item.encode_into(out);
                }
            }
            Value::Map(entries) => {
                let mut encoded: Vec<(Vec<u8>, Vec<u8>)> = entries
                    .iter()
                    .map(|(k, v)| (k.encode(), v.encode()))
                    .collect();
                encoded.sort_by(|a, b| a.0.cmp(&b.0));
                encoded.dedup_by(|a, b| a.0 == b.0);
                head(out, 5, encoded.len() as u64);
                for (k, v) in encoded {
                    out.extend_from_slice(&k);
                    out.extend_from_slice(&v);
                }
            }
            Value::Bool(false) => out.push(0xf4),
            Value::Bool(true) => out.push(0xf5),
            Value::Null => out.push(0xf6),
        }
    }

    /// Decode exactly one value that fills `bytes`.
    ///
    /// # Errors
    ///
    /// Returns an error for malformed, noncanonical or unsupported CBOR, or trailing bytes.
    pub fn decode(bytes: &[u8]) -> Result<Value, Error> {
        let mut pos = 0;
        let value = decode_at(bytes, &mut pos, 0)?;
        if pos != bytes.len() {
            return Err(Error("trailing bytes"));
        }
        Ok(value)
    }

    /// Decode one value from the start of `bytes`, returning it and its length.
    ///
    /// # Errors
    ///
    /// Returns an error for truncated, noncanonical or unsupported CBOR.
    pub fn decode_prefix(bytes: &[u8]) -> Result<(Value, usize), Error> {
        let mut pos = 0;
        let value = decode_at(bytes, &mut pos, 0)?;
        Ok((value, pos))
    }

    /// Look up an integer key in a map.
    pub fn get(&self, key: i64) -> Option<&Value> {
        let key = Value::int(key);
        match self {
            Value::Map(entries) => entries.iter().find(|(k, _)| *k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The value as an unsigned integer.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Uint(n) => Some(*n),
            _ => None,
        }
    }

    /// The value as a signed integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Uint(n) => i64::try_from(*n).ok(),
            Value::Nint(n) => i64::try_from(*n).ok().map(|n| -1 - n),
            _ => None,
        }
    }

    /// The value as bytes.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            _ => None,
        }
    }

    /// The value as text.
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }

    /// The value as an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }
}

/// Build a map with integer keys.
pub fn map<const N: usize>(entries: [(i64, Value); N]) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(k, v)| (Value::int(k), v))
            .collect(),
    )
}

fn head(out: &mut Vec<u8>, major: u8, n: u64) {
    let m = major << 5;
    if n < 24 {
        out.push(m | n as u8);
    } else if u8::try_from(n).is_ok() {
        out.extend_from_slice(&[m | 24, n as u8]);
    } else if u16::try_from(n).is_ok() {
        out.push(m | 25);
        out.extend_from_slice(&(n as u16).to_be_bytes());
    } else if u32::try_from(n).is_ok() {
        out.push(m | 26);
        out.extend_from_slice(&(n as u32).to_be_bytes());
    } else {
        out.push(m | 27);
        out.extend_from_slice(&n.to_be_bytes());
    }
}

fn take<'a>(bytes: &'a [u8], pos: &mut usize, n: usize) -> Result<&'a [u8], Error> {
    let end = pos.checked_add(n).ok_or(Error("length overflow"))?;
    let slice = bytes.get(*pos..end).ok_or(Error("unexpected end"))?;
    *pos = end;
    Ok(slice)
}

fn read_head(bytes: &[u8], pos: &mut usize) -> Result<(u8, u8, u64), Error> {
    let first = take(bytes, pos, 1)?[0];
    let major = first >> 5;
    let info = first & 0x1f;
    let n = match info {
        0..=23 => u64::from(info),
        24 => {
            let n = u64::from(take(bytes, pos, 1)?[0]);
            if n < 24 {
                return Err(Error("non-shortest integer"));
            }
            n
        }
        25 => {
            let n = u64::from(u16::from_be_bytes(
                take(bytes, pos, 2)?.try_into().unwrap_or([0; 2]),
            ));
            if u8::try_from(n).is_ok() {
                return Err(Error("non-shortest integer"));
            }
            n
        }
        26 => {
            let n = u64::from(u32::from_be_bytes(
                take(bytes, pos, 4)?.try_into().unwrap_or([0; 4]),
            ));
            if u16::try_from(n).is_ok() {
                return Err(Error("non-shortest integer"));
            }
            n
        }
        27 => {
            let n = u64::from_be_bytes(take(bytes, pos, 8)?.try_into().unwrap_or([0; 8]));
            if u32::try_from(n).is_ok() {
                return Err(Error("non-shortest integer"));
            }
            n
        }
        _ => return Err(Error("unsupported additional information")),
    };
    Ok((major, info, n))
}

fn decode_at(bytes: &[u8], pos: &mut usize, depth: usize) -> Result<Value, Error> {
    if depth > MAX_DEPTH {
        return Err(Error("nested too deeply"));
    }
    let (major, info, n) = read_head(bytes, pos)?;
    let len = |n: u64| usize::try_from(n).map_err(|_| Error("length overflow"));
    Ok(match major {
        0 => Value::Uint(n),
        1 => Value::Nint(n),
        2 => Value::Bytes(take(bytes, pos, len(n)?)?.to_vec()),
        3 => {
            let raw = take(bytes, pos, len(n)?)?;
            Value::Text(String::from_utf8(raw.to_vec()).map_err(|_| Error("invalid UTF-8"))?)
        }
        4 => {
            let n = len(n)?;
            if n > bytes.len() {
                return Err(Error("unexpected end"));
            }
            let mut items = Vec::with_capacity(n);
            for _ in 0..n {
                items.push(decode_at(bytes, pos, depth + 1)?);
            }
            Value::Array(items)
        }
        5 => {
            let n = len(n)?;
            if n > bytes.len() {
                return Err(Error("unexpected end"));
            }
            let mut entries = Vec::with_capacity(n);
            let mut last_key: Option<&[u8]> = None;
            for _ in 0..n {
                let start = *pos;
                let key = decode_at(bytes, pos, depth + 1)?;
                let key_bytes = &bytes[start..*pos];
                if let Some(last) = last_key
                    && key_bytes <= last
                {
                    return Err(Error("map keys not in canonical order"));
                }
                last_key = Some(key_bytes);
                let value = decode_at(bytes, pos, depth + 1)?;
                entries.push((key, value));
            }
            Value::Map(entries)
        }
        7 => match info {
            20 => Value::Bool(false),
            21 => Value::Bool(true),
            22 => Value::Null,
            _ => return Err(Error("unsupported simple value")),
        },
        _ => return Err(Error("unsupported major type")),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_canonical_order() {
        let v = Value::Map(vec![
            (Value::Text("b".into()), Value::int(-500)),
            (Value::int(10), Value::Bytes(vec![1, 2, 3])),
            (
                Value::int(1),
                Value::Array(vec![Value::Null, Value::Bool(true)]),
            ),
        ]);
        let bytes = v.encode();
        let back = Value::decode(&bytes).unwrap();
        // Keys come back sorted: 1, 10, "b".
        assert_eq!(back.get(1), v.get(1));
        assert_eq!(back.encode(), bytes);
        assert_eq!(bytes[1], 0x01);
    }

    #[test]
    fn rejects_non_shortest() {
        assert!(Value::decode(&[0x18, 0x05]).is_err());
        assert!(Value::decode(&[0x19, 0x00, 0xff]).is_err());
        assert_eq!(Value::decode(&[0x18, 0x18]).unwrap(), Value::Uint(24));
    }

    #[test]
    fn rejects_unsorted_maps_and_garbage() {
        // {2: 0, 1: 0}
        assert!(Value::decode(&[0xa2, 0x02, 0x00, 0x01, 0x00]).is_err());
        assert!(Value::decode(&[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]).is_err());
        assert!(Value::decode(&[0x01, 0x02]).is_err());
    }

    #[test]
    fn integers() {
        for n in [
            0i64,
            23,
            24,
            255,
            256,
            65535,
            65536,
            -1,
            -24,
            -25,
            -1000,
            i64::MAX,
            i64::MIN,
        ] {
            let back = Value::decode(&Value::int(n).encode()).unwrap();
            assert_eq!(back.as_i64(), Some(n));
        }
    }
}
