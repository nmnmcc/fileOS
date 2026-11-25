// SPDX-License-Identifier: AGPL-3.0-only

//! Talking to a running node over its HTTP API, as the web app and other
//! clients do, and remembering the nodes one has signed in to.
//!
//! Sign-ins are kept in `$XDG_CONFIG_HOME/homeserver/cli.json` (only the
//! owner can read it): for each server URL a token, and which one is the
//! default. `--server` (or `$HS_SERVER`) picks another, and `$HS_TOKEN`
//! supplies a token without saving it.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;

use serde_json::{Value as Json, json};

use crate::Res;

/// The address `hs ctl serve` listens on unless told otherwise.
pub const LOCAL_URL: &str = "http://127.0.0.1:8080";

// Saved sign-ins

#[derive(Default)]
struct Saved {
    default: Option<String>,
    servers: BTreeMap<String, Login>,
}

#[derive(Clone)]
struct Login {
    token: String,
    /// Accept any TLS certificate (a node's own self-signed one).
    insecure: bool,
}

fn config_path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| ".".into())).join(".config")
        });
    base.join("homeserver/cli.json")
}

impl Saved {
    fn load() -> Res<Saved> {
        let path = config_path();
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Saved::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let v: Json =
            serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        let servers = v["servers"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(url, s)| {
                Some((
                    url.clone(),
                    Login {
                        token: s["token"].as_str()?.to_string(),
                        insecure: s["insecure"].as_bool().unwrap_or(false),
                    },
                ))
            })
            .collect();
        Ok(Saved {
            default: v["default"].as_str().map(str::to_string),
            servers,
        })
    }

    fn save(&self) -> Res {
        let path = config_path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let servers: serde_json::Map<String, Json> = self
            .servers
            .iter()
            .map(|(url, l)| {
                (
                    url.clone(),
                    json!({ "token": l.token, "insecure": l.insecure }),
                )
            })
            .collect();
        let text = serde_json::to_string_pretty(&json!({
            "default": self.default,
            "servers": servers,
        }))
        .unwrap_or_default();
        // Written beside, then moved over, so a crash leaves the old file.
        let tmp = path.with_extension("json.tmp");
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
        let mut f = options
            .open(&tmp)
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
        f.write_all(text.as_bytes())
            .and_then(|()| f.write_all(b"\n"))
            .map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))
    }
}

/// `http://host:port` without a trailing slash; `host:port` means `http`
/// for loopback addresses and `https` for anything else.
pub fn normalize_url(url: &str) -> Res<String> {
    let url = url.trim().trim_end_matches('/');
    let url = if url.contains("://") {
        url.to_string()
    } else if url.starts_with("localhost") || url.starts_with("127.") || url.starts_with("[::1]") {
        format!("http://{url}")
    } else {
        format!("https://{url}")
    };
    let uri: http::Uri = url
        .parse()
        .map_err(|error| format!("not a URL: {url}: {error}"))?;
    if !matches!(uri.scheme_str(), Some("http" | "https")) || uri.host().is_none() {
        return Err(format!("not an http(s) URL: {url}"));
    }
    if uri.path() != "/" && !uri.path().is_empty() {
        return Err(format!("give the server's address without a path: {url}"));
    }
    Ok(url)
}

/// The server picked by `--server`, `$HS_SERVER` or the saved default, if
/// any.
pub fn configured(server: Option<&str>) -> Res<Option<Server>> {
    let saved = Saved::load()?;
    let env_token = std::env::var("HS_TOKEN").ok().filter(|t| !t.is_empty());
    let url = match server {
        Some(s) => normalize_url(s)?,
        None => match &saved.default {
            Some(d) => d.clone(),
            None => return Ok(None),
        },
    };
    let login = saved.servers.get(&url).cloned();
    let token = match (env_token, &login) {
        (Some(t), _) => t,
        (None, Some(l)) => l.token.clone(),
        (None, None) => {
            return Err(format!(
                "no token for {url}; sign in with `hs login {url}` or set HS_TOKEN"
            ));
        }
    };
    Ok(Some(Server::new(
        url,
        token,
        login.is_some_and(|l| l.insecure),
    )))
}

/// The server, or a message saying how to pick one.
pub fn required(server: Option<&str>) -> Res<Server> {
    configured(server)?.ok_or_else(|| {
        "this needs a running node: sign in to one with `hs login URL` (or pass --server)".into()
    })
}

/// Remember a sign-in and make it the default.
pub fn save_login(url: &str, token: &str, insecure: bool) -> Res {
    let mut saved = Saved::load()?;
    saved.servers.insert(
        url.to_string(),
        Login {
            token: token.to_string(),
            insecure,
        },
    );
    saved.default = Some(url.to_string());
    saved.save()
}

/// Whether any sign-in is saved.
pub fn has_logins() -> Res<bool> {
    Ok(!Saved::load()?.servers.is_empty())
}

/// Forget a sign-in (the default one without `url`). Returns the URL.
pub fn forget_login(url: Option<&str>) -> Res<Option<String>> {
    let mut saved = Saved::load()?;
    let url = match url {
        Some(u) => normalize_url(u)?,
        None => match saved.default.clone() {
            Some(d) => d,
            None => return Ok(None),
        },
    };
    if saved.servers.remove(&url).is_none() {
        return Ok(None);
    }
    if saved.default.as_deref() == Some(&url) {
        saved.default = saved.servers.keys().next().cloned();
    }
    saved.save()?;
    Ok(Some(url))
}

/// The saved sign-ins: URL and whether it is the default.
pub fn logins() -> Res<Vec<(String, bool)>> {
    let saved = Saved::load()?;
    Ok(saved
        .servers
        .keys()
        .map(|u| (u.clone(), saved.default.as_deref() == Some(u)))
        .collect())
}

/// Read a token: from a prompt without echo on a terminal, else one line
/// of stdin.
pub fn read_token() -> Res<String> {
    let stdin = std::io::stdin();
    let line = if stdin.is_terminal() {
        eprint!("Token: ");
        let _ = std::io::stderr().flush();
        let line = without_echo(|| {
            let mut s = String::new();
            stdin.read_line(&mut s).map(|_| s)
        });
        eprintln!();
        line
    } else {
        let mut s = String::new();
        stdin.read_line(&mut s).map(|_| s)
    }
    .map_err(|e| e.to_string())?;
    let token = line.trim().to_string();
    if token.is_empty() {
        return Err("no token given".into());
    }
    Ok(token)
}

#[cfg(unix)]
fn without_echo<T>(read: impl FnOnce() -> T) -> T {
    use rustix::termios::{LocalModes, OptionalActions, tcgetattr, tcsetattr};
    let fd = std::io::stdin();
    let Ok(before) = tcgetattr(&fd) else {
        return read();
    };
    let mut quiet = before.clone();
    quiet.local_modes.remove(LocalModes::ECHO);
    let _ = tcsetattr(&fd, OptionalActions::Now, &quiet);
    let out = read();
    let _ = tcsetattr(&fd, OptionalActions::Now, &before);
    out
}

#[cfg(not(unix))]
fn without_echo<T>(read: impl FnOnce() -> T) -> T {
    read()
}

// Requests

/// A node reached over HTTP, with a token.
pub use hs_client::Client as Server;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_get_a_scheme_and_lose_the_slash() {
        assert_eq!(
            normalize_url("localhost:8080/").unwrap(),
            "http://localhost:8080"
        );
        assert_eq!(
            normalize_url("127.0.0.1:8080").unwrap(),
            "http://127.0.0.1:8080"
        );
        assert_eq!(
            normalize_url("hs.example.org").unwrap(),
            "https://hs.example.org"
        );
        assert_eq!(
            normalize_url("http://hs.example.org").unwrap(),
            "http://hs.example.org"
        );
        normalize_url("ftp://x").unwrap_err();
        normalize_url("https://x/api").unwrap_err();
    }
}
