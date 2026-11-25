// SPDX-License-Identifier: AGPL-3.0-only

//! Embed the web app: `crates/hs-ui` as `web-build` packages it, or the
//! directory in `HS_WEB_DIR`. Release builds (`HS_REQUIRE_NSH_WEB`) must name
//! one. Without a package, a page that says how to make one stands in, so the
//! server builds and tests without the NSH tools.

use std::{
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

fn main() {
    let manifest = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let root = manifest.join("../..");
    println!("cargo::rerun-if-env-changed=HS_WEB_DIR");
    println!("cargo::rerun-if-env-changed=HS_REQUIRE_NSH_WEB");
    let release = env::var_os("HS_REQUIRE_NSH_WEB").is_some();
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("set by cargo"));
    let package = root.join("crates/hs-ui/.nsh/web/homeserver");
    let dir = match env::var_os("HS_WEB_DIR") {
        Some(dir) => PathBuf::from(dir),
        None if release => panic!("release builds require an explicit HS_WEB_DIR"),
        None if package.join("index.html").is_file() => package,
        None => {
            // Notice when a package appears.
            println!(
                "cargo::rerun-if-changed={}",
                root.join("crates/hs-ui/.nsh").display()
            );
            let dir = out_dir.join("web-missing");
            fs::create_dir_all(&dir).expect("OUT_DIR is writable");
            fs::write(dir.join("index.html"), MISSING).expect("OUT_DIR is writable");
            dir
        }
    };
    println!("cargo::rerun-if-changed={}", dir.display());

    let mut files = Vec::new();
    walk(&dir, &dir, &mut files);
    files.sort();
    assert!(
        files.iter().any(|f| f == "index.html"),
        "no index.html in {}",
        dir.display()
    );
    assert!(
        !release
            || (files
                .iter()
                .any(|f| f.starts_with("nsh-start.") && f.ends_with(".js"))
                && files.iter().any(|f| f.ends_with(".wasm"))),
        "{} does not contain a Native Shell browser bundle",
        dir.display()
    );

    let mut out = String::from("&[\n");
    for path in &files {
        let full = literal(&dir.join(path));
        let br = dir.join(format!("{path}.br"));
        let br = if br.is_file() {
            format!("Some(include_bytes!({}))", literal(&br))
        } else {
            "None".to_owned()
        };
        let _ = writeln!(
            out,
            "    File {{ path: {path:?}, media_type: {:?}, immutable: {}, bytes: include_bytes!({full}), br: {br} }},",
            media_type(path),
            named_by_content(path),
        );
    }
    out.push(']');
    let target = out_dir.join("web_files.rs");
    fs::write(target, out).expect("OUT_DIR is writable");
}

/// The page served when this build has no web app.
const MISSING: &str = "<!doctype html>\n<meta charset=\"utf-8\">\n<title>homeserver</title>\n<p>This build of homeserver has no web app. Run <code>web-build</code>, then build again.</p>\n";

/// `path`, made absolute, as a Rust string literal.
fn literal(path: &Path) -> String {
    let path = path.canonicalize().expect("exists");
    format!("{:?}", path.to_str().expect("a UTF-8 path"))
}

/// Every file under `dir`, as `/`-separated paths, without the compressed
/// copies (`.br` rides along with its file; `.gz` is left out, as every
/// browser that takes gzip takes brotli over HTTPS).
fn walk(base: &Path, dir: &Path, files: &mut Vec<String>) {
    for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("{}: {e}", dir.display())) {
        let path = entry.expect("readable").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            walk(base, &path, files);
        } else if !name.ends_with(".br") && !name.ends_with(".gz") {
            let rel = path.strip_prefix(base).expect("under base");
            let parts: Vec<_> = rel.iter().map(|p| p.to_string_lossy()).collect();
            files.push(parts.join("/"));
        }
    }
}

/// Whether a part of the path is a content digest (`nsh.685a9f0a1f.wasm`,
/// `browser.de8aa0f5a1/`), so the file never changes under that path.
fn named_by_content(path: &str) -> bool {
    path.split(['/', '.'])
        .any(|part| part.len() >= 8 && part.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn media_type(path: &str) -> &'static str {
    match path.rsplit_once('.').map_or("", |(_, ext)| ext) {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "webmanifest" => "application/manifest+json",
        "wasm" => "application/wasm",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "txt" => "text/plain; charset=utf-8",
        _ => "application/octet-stream",
    }
}
