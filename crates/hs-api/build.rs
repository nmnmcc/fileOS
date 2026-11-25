// SPDX-License-Identifier: AGPL-3.0-only

//! Embed a web app: the static files in the directory `HS_WEB_DIR` names.
//! Without one, a page pointing to the command line stands in.

use std::{
    env,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

fn main() {
    println!("cargo::rerun-if-env-changed=HS_WEB_DIR");
    let manifest = match env::var_os("HS_WEB_DIR") {
        Some(dir) => web_files(Path::new(&dir)),
        None => format!(
            r#"&[File {{ path: "index.html", media_type: {:?}, immutable: false, bytes: {MISSING:?}.as_bytes(), br: None }}]"#,
            media_type("index.html")
        ),
    };
    let target = PathBuf::from(env::var_os("OUT_DIR").expect("set by cargo")).join("web_files.rs");
    // A redundant write can trigger another editor check.
    if !fs::read(&target).is_ok_and(|current| current == manifest.as_bytes()) {
        fs::write(target, manifest).expect("OUT_DIR is writable");
    }
}

fn web_files(dir: &Path) -> String {
    println!("cargo::rerun-if-changed={}", dir.display());

    let mut files = Vec::new();
    walk(dir, dir, &mut files);
    files.sort();
    assert!(
        files.iter().any(|f| f == "index.html"),
        "no index.html in {}",
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
    out
}

/// The page served when this build has no web app.
const MISSING: &str = "<!doctype html>\n<meta charset=\"utf-8\">\n<title>homeserver</title>\n<p>This homeserver has no web app yet. Use the <code>hs</code> command line: <code>hs login</code>, then <code>hs --help</code>.</p>\n";

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

/// Whether a part of the path is a content digest (`app.685a9f0a1f.wasm`,
/// `assets.de8aa0f5a1/`), so the file never changes under that path.
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
