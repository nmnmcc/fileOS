// SPDX-License-Identifier: AGPL-3.0-only

//! Shell completion. `hs completion SHELL` prints a script that asks `hs`
//! itself what may come next (with `COMPLETE=SHELL` set). So commands and
//! options always match this executable. Spaces, grants and configuration
//! keys come from the node.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use clap::CommandFactory;
use clap_complete::engine::CompletionCandidate;
use clap_complete::env::Shells;

use crate::commands::{self, Global, Target};
use crate::output::short;
use crate::{Cli, CtlCli, Res};

/// The variable that asks `hs` for completions.
const VAR: &str = "COMPLETE";

/// How long to wait for the node before offering nothing.
const PATIENCE: Duration = Duration::from_secs(2);

/// Answer a completion request, if this is one, and exit.
pub fn answer(as_ctl: bool) {
    let factory: fn() -> clap::Command = if as_ctl {
        CtlCli::command
    } else {
        Cli::command
    };
    clap_complete::CompleteEnv::with_factory(factory)
        .var(VAR)
        .complete();
}

/// The script that adds completion of `hs` and `hsctl` to a shell.
pub fn script(shell: clap_complete::Shell) -> Res<Vec<u8>> {
    let name = shell.to_string();
    let shells = Shells::builtins();
    let shell = shells
        .completer(&name)
        .ok_or_else(|| format!("no completion for {name}"))?;
    let mut script = Vec::new();
    for bin in ["hs", "hsctl"] {
        shell
            .write_registration(VAR, bin, bin, bin, &mut script)
            .map_err(|e| e.to_string())?;
    }
    // The first line of a zsh script names the commands it completes.
    let script = String::from_utf8_lossy(&script)
        .replacen("#compdef hs\n", "#compdef hs hsctl\n", 1)
        .replace("#compdef hsctl\n", "");
    Ok(script.into_bytes())
}

/// Spaces, by name.
pub fn spaces() -> Vec<CompletionCandidate> {
    ask(async |t| {
        commands::space_names(t)
            .await
            .into_iter()
            .map(|(id, name)| CompletionCandidate::new(name).help(Some(short(&id, 10).into())))
            .collect()
    })
}

/// Grants, by ID.
pub fn grants() -> Vec<CompletionCandidate> {
    ask(async |t| {
        commands::grants(t)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|g| {
                let help = format!("{} {}", g.role.as_str(), g.label);
                CompletionCandidate::new(g.id).help(Some(help.into()))
            })
            .collect()
    })
}

/// The keys in the configuration.
pub fn config_keys() -> Vec<CompletionCandidate> {
    let path = crate::data_dir(given("dir").map(PathBuf::from)).join("config.json");
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let Ok(serde_json::Value::Object(config)) = serde_json::from_slice(&bytes) else {
        return Vec::new();
    };
    config.keys().map(CompletionCandidate::new).collect()
}

/// Ask the node that the command line would use. On any error, return
/// nothing and print nothing.
fn ask(f: impl AsyncFnOnce(&Target) -> Vec<CompletionCandidate>) -> Vec<CompletionCandidate> {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return Vec::new();
    };
    let g = Global {
        dir: given("dir").map(PathBuf::from),
        server: given("server").and_then(|s| s.into_string().ok()),
        json: false,
    };
    rt.block_on(async {
        let Ok(Ok(target)) = tokio::time::timeout(PATIENCE, g.target()).await else {
            return Vec::new();
        };
        let found = tokio::time::timeout(PATIENCE, f(&target))
            .await
            .unwrap_or_default();
        target.close().await;
        found
    })
}

/// A global option from the command line being completed, else from its
/// environment variable.
fn given(name: &str) -> Option<OsString> {
    let args: Vec<OsString> = std::env::args_os().collect();
    option(&args, name).or_else(|| std::env::var_os(format!("HS_{}", name.to_uppercase())))
}

/// `--name VALUE` or `--name=VALUE`.
fn option(args: &[OsString], name: &str) -> Option<OsString> {
    let flag = format!("--{name}");
    let joined = format!("--{name}=");
    args.iter().enumerate().find_map(|(i, a)| {
        let a = a.to_str()?;
        if a == flag {
            args.get(i + 1).cloned()
        } else {
            a.strip_prefix(&joined).map(OsString::from)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_from_the_command_line() {
        let args: Vec<OsString> = ["hs", "--", "hs", "--dir", "/d", "--server=s", "ls", ""]
            .into_iter()
            .map(OsString::from)
            .collect();
        assert_eq!(option(&args, "dir"), Some("/d".into()));
        assert_eq!(option(&args, "server"), Some("s".into()));
        assert_eq!(option(&args, "json"), None);
    }

    #[test]
    fn every_shell_has_a_script() {
        use clap::ValueEnum as _;
        for shell in clap_complete::Shell::value_variants() {
            let script = script(*shell).unwrap();
            assert!(String::from_utf8(script).unwrap().contains("COMPLETE"));
        }
    }
}
