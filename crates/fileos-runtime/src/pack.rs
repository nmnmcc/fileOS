// SPDX-License-Identifier: AGPL-3.0-only

//! `.app` files.
//!
//! An `.app` file is a zip with no compression in the zip itself:
//!
//! - `manifest.toml`: the metadata, as it is, so tools can read it with no
//!   unpacking.
//! - `payload.tar.zst`: every other file of the bundle, in a tar, compressed
//!   with zstd.
//!
//! The zip has a directory of its entries at the end, so the metadata is
//! found with no need to read the payload.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Component, Path};

use fileos_core::Error;

use crate::Bundle;

/// The files that are metadata, kept as they are in the zip.
const METADATA: [&str; 1] = ["manifest.toml"];

/// The name of the payload in the zip.
const PAYLOAD: &str = "payload.tar.zst";

/// The most bytes that an `.app` file may unpack to, so that a small file
/// cannot take all the memory.
const MAX_UNPACKED: u64 = 512 << 20;

fn zip_error(e: &dyn std::fmt::Display) -> Error {
    Error(format!("not a good .app file: {e}"))
}

/// Says if a path in an archive is a plain relative path, with no `..`.
fn safe(path: &str) -> bool {
    !path.is_empty()
        && Path::new(path)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

impl Bundle {
    /// The bundle as an `.app` file.
    ///
    /// # Errors
    ///
    /// When the archive cannot be made.
    pub fn pack(&self) -> Result<Vec<u8>, Error> {
        let io = |e: std::io::Error| Error(format!("cannot make the .app file: {e}"));
        let mut tar = tar::Builder::new(Vec::new());
        for (path, data) in &self.files {
            let name = path.trim_start_matches('/');
            if METADATA.contains(&name) {
                continue;
            }
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            // No time, owner or other details: the same bundle always gives
            // the same file.
            header.set_mtime(0);
            tar.append_data(&mut header, name, data.as_slice())
                .map_err(io)?;
        }
        let tar = tar.into_inner().map_err(io)?;
        let payload = ruzstd::encoding::compress_to_vec(
            tar.as_slice(),
            ruzstd::encoding::CompressionLevel::Fastest,
        );

        let mut out = Vec::new();
        let mut zip = rawzip::ZipArchiveWriter::new(&mut out);
        let entries = METADATA
            .iter()
            .filter_map(|name| Some((*name, self.files.get(&format!("/{name}"))?.as_slice())))
            .chain([(PAYLOAD, payload.as_slice())]);
        for (name, data) in entries {
            let (mut entry, config) = zip
                .new_file(name)
                .compression_method(rawzip::CompressionMethod::STORE)
                .start()
                .map_err(|e| zip_error(&e))?;
            let mut writer = config.wrap(&mut entry);
            writer.write_all(data).map_err(io)?;
            let (_, descriptor) = writer.finish().map_err(|e| zip_error(&e))?;
            entry.finish(descriptor).map_err(|e| zip_error(&e))?;
        }
        zip.finish().map_err(|e| zip_error(&e))?;
        Ok(out)
    }

    /// Reads a bundle from an `.app` file.
    ///
    /// # Errors
    ///
    /// When the bytes are not a good `.app` file, or [`Bundle::from_files`]
    /// fails.
    pub fn unpack(bytes: &[u8]) -> Result<Self, Error> {
        let zip = rawzip::ZipArchive::from_slice(bytes).map_err(|e| zip_error(&e))?;
        let mut files = BTreeMap::new();
        let mut left = MAX_UNPACKED;
        let mut entries = zip.entries();
        while let Some(header) = entries.next_entry().map_err(|e| zip_error(&e))? {
            if header.is_dir() {
                continue;
            }
            let name = header
                .file_path()
                .try_normalize()
                .map_err(|e| zip_error(&e))?
                .as_ref()
                .to_string();
            if header.compression_method() != rawzip::CompressionMethod::STORE {
                return Err(zip_error(&format!("{name} is compressed in the zip")));
            }
            if !safe(&name) {
                return Err(zip_error(&format!("{name:?} is not a plain path")));
            }
            let entry = zip
                .get_entry(header.wayfinder())
                .map_err(|e| zip_error(&e))?;
            let mut data = Vec::new();
            entry
                .verifying_reader(entry.data())
                .take(left)
                .read_to_end(&mut data)
                .map_err(|e| zip_error(&e))?;
            left -= data.len() as u64;
            if name == PAYLOAD {
                left = unpack_payload(&data, left, &mut files)?;
            } else {
                files.insert(format!("/{name}"), data);
            }
        }
        Self::from_files(files)
    }

    /// Reads a bundle from an `.app` file, or from an `.app` folder (which
    /// is handy while making an app).
    ///
    /// # Errors
    ///
    /// When the path cannot be read, or is not a good bundle.
    pub fn read(path: &Path) -> Result<Self, Error> {
        if path.is_dir() {
            return Self::read_dir(path);
        }
        let bytes = std::fs::read(path).map_err(|e| Error(format!("{}: {e}", path.display())))?;
        Self::unpack(&bytes).map_err(|e| Error(format!("{}: {e}", path.display())))
    }

    /// Writes the files of the bundle into a directory.
    ///
    /// # Errors
    ///
    /// When a file cannot be written.
    pub fn write_dir(&self, dir: &Path) -> Result<(), Error> {
        for (path, data) in &self.files {
            let target = dir.join(path.trim_start_matches('/'));
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| Error(format!("{}: {e}", parent.display())))?;
            }
            std::fs::write(&target, data)
                .map_err(|e| Error(format!("{}: {e}", target.display())))?;
        }
        Ok(())
    }
}

/// Puts the files of the payload into `files`. Gives the bytes that may
/// still be unpacked.
fn unpack_payload(
    payload: &[u8],
    mut left: u64,
    files: &mut BTreeMap<String, Vec<u8>>,
) -> Result<u64, Error> {
    let mut source = payload;
    let decoder =
        ruzstd::decoding::StreamingDecoder::new(&mut source).map_err(|e| zip_error(&e))?;
    let mut tar = Vec::new();
    decoder
        .take(left)
        .read_to_end(&mut tar)
        .map_err(|e| zip_error(&e))?;
    left -= tar.len() as u64;
    let mut archive = tar::Archive::new(tar.as_slice());
    for entry in archive.entries().map_err(|e| zip_error(&e))? {
        let mut entry = entry.map_err(|e| zip_error(&e))?;
        let kind = entry.header().entry_type();
        if kind.is_dir() {
            continue;
        }
        let path = entry.path().map_err(|e| zip_error(&e))?;
        let name = path.to_string_lossy().into_owned();
        if !kind.is_file() {
            return Err(zip_error(&format!("{name} is not a plain file")));
        }
        if !safe(&name) || METADATA.contains(&name.as_str()) {
            return Err(zip_error(&format!("{name:?} may not be in the payload")));
        }
        let mut data = Vec::new();
        entry.read_to_end(&mut data).map_err(|e| zip_error(&e))?;
        files.insert(format!("/{name}"), data);
    }
    Ok(left)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bundle() -> Bundle {
        let manifest = "id = \"t\"\nname = \"T\"\nkind = \"app\"\nversion = \"1\"\nbuild = 1\n\
                        requires = \"fileos:app@0.1.0\"\nmain = \"bin/app.wasm\"\n";
        Bundle::from_files(
            [
                ("/manifest.toml".to_string(), manifest.as_bytes().to_vec()),
                ("/bin/app.wasm".to_string(), vec![0; 10_000]),
                (
                    "/resources/locales/en.toml".to_string(),
                    b"hello = \"Hello\"".to_vec(),
                ),
            ]
            .into(),
        )
        .unwrap()
    }

    #[test]
    fn an_app_file_goes_back_to_the_same_bundle() {
        let bundle = bundle();
        let file = bundle.pack().unwrap();
        // The same bundle always gives the same file.
        assert_eq!(file, bundle.pack().unwrap());
        // The payload is compressed: 10 000 zero bytes take much less room.
        assert!(file.len() < 2_000);
        let back = Bundle::unpack(&file).unwrap();
        assert_eq!(back.files, bundle.files);
        assert_eq!(back.hash(), bundle.hash());
    }

    #[test]
    fn the_metadata_is_not_compressed() {
        let file = bundle().pack().unwrap();
        let zip = rawzip::ZipArchive::from_slice(&file).unwrap();
        let mut names = Vec::new();
        let mut entries = zip.entries();
        while let Some(header) = entries.next_entry().unwrap() {
            assert_eq!(
                header.compression_method(),
                rawzip::CompressionMethod::STORE
            );
            names.push(
                header
                    .file_path()
                    .try_normalize()
                    .unwrap()
                    .as_ref()
                    .to_string(),
            );
        }
        assert_eq!(names, ["manifest.toml", "payload.tar.zst"]);
        let text = String::from_utf8_lossy(&file);
        assert!(text.contains("main = \"bin/app.wasm\""));
    }

    #[test]
    fn bad_files_are_not_taken() {
        Bundle::unpack(b"not a zip").unwrap_err();
        assert!(!safe("../x"));
        assert!(!safe("/etc/passwd"));
        assert!(safe("bin/app.wasm"));
    }
}
