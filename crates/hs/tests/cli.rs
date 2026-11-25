// SPDX-License-Identifier: AGPL-3.0-only

//! The command line against a node: its data directory while stopped, and
//! its API while running.

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::Arc;

use hs_api::Api;
use hs_node::Node;

/// Run `hs` with its own config and data homes; returns (ok, stdout, stderr).
fn hs(home: &Path, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> (bool, String, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_hs"))
        .args(args)
        .env("XDG_CONFIG_HOME", home.join("config"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env_remove("HS_DIR")
        .env_remove("HS_SERVER")
        .env_remove("HS_TOKEN")
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stdin.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Like `hs`, but off the runtime that serves the node.
async fn run(
    home: &Path,
    args: &[&str],
    stdin: &str,
    env: &[(&str, &str)],
) -> (bool, String, String) {
    let home = home.to_path_buf();
    let args: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
    let stdin = stdin.to_string();
    let env: Vec<(String, String)> = env
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect();
    tokio::task::spawn_blocking(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        hs(&home, &args, &stdin, &env)
    })
    .await
    .unwrap()
}

/// Expect success; returns stdout.
async fn ok(home: &Path, args: &[&str], stdin: &str) -> String {
    let (ok, out, err) = run(home, args, stdin, &[]).await;
    assert!(ok, "hs {args:?} failed: {err}");
    out
}

#[tokio::test(flavor = "multi_thread")]
async fn works_with_a_running_node() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    let dir = home.join("node");
    let made = Node::init(&dir, "Test", None).await.unwrap();
    let api = Api::new(Arc::new(made.node));
    api.start();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(hs_net::server::serve(
        listener,
        Arc::clone(&api),
        None,
        std::future::pending(),
    ));

    // Nothing to talk to yet.
    let (signed_in, _, err) = run(home, &["status"], "", &[]).await;
    assert!(!signed_in);
    assert!(err.contains("hs login"), "{err}");
    let (accepted, _, err) = run(home, &["login", &url], "wrong\n", &[]).await;
    assert!(!accepted);
    assert!(err.contains("did not accept"), "{err}");

    let out = ok(home, &["login", &url], &format!("{}\n", made.owner_token)).await;
    assert!(out.contains("Signed in to Test"), "{out}");
    let saved = std::fs::read_to_string(home.join("config/homeserver/cli.json")).unwrap();
    assert!(saved.contains(&made.owner_token));
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(home.join("config/homeserver/cli.json"))
            .unwrap()
            .permissions()
            .mode()
    };
    #[cfg(unix)]
    assert_eq!(mode & 0o777, 0o600);
    assert!(ok(home, &["status"], "").await.contains("Node       Test"));

    // Spaces.
    let id = ok(home, &["space", "create", "笔记"], "").await;
    let id = id.trim();
    assert_eq!(id.len(), 64);
    assert!(ok(home, &["space", "list"], "").await.contains("笔记"));
    assert!(
        ok(home, &["space", "show", &id[..10]], "")
            .await
            .contains(id)
    );

    // Resources.
    let out = ok(
        home,
        &["put", "笔记", "note/shopping"],
        r#"{"title": "Shopping", "text": "milk"}"#,
    )
    .await;
    assert!(out.starts_with("created note/shopping"), "{out}");
    let (again, _, err) = run(home, &["put", "笔记", "note/shopping", "--new"], "{}", &[]).await;
    assert!(!again);
    assert!(err.contains("changed on the server"), "{err}");
    let got: serde_json::Value =
        serde_json::from_str(&ok(home, &["get", "笔记", "note/shopping"], "").await).unwrap();
    assert_eq!(got["text"], "milk");

    let out = ok(
        home,
        &["patch", "笔记", "note/shopping", r#"{"done": true}"#],
        "",
    )
    .await;
    assert!(out.starts_with("updated"), "{out}");

    let (edited, out, err) = run(
        home,
        &["edit", "笔记", "note/shopping"],
        "",
        &[("VISUAL", "sed -i.bak s/milk/eggs/")],
    )
    .await;
    assert!(edited, "{err}");
    assert!(out.starts_with("updated"), "{out}");
    let got: serde_json::Value =
        serde_json::from_str(&ok(home, &["get", "笔记", "note/shopping"], "").await).unwrap();
    assert_eq!(
        (got["text"].as_str(), got["done"].as_bool()),
        (Some("eggs"), Some(true))
    );

    let list = ok(home, &["ls", "笔记"], "").await;
    assert!(list.contains("note/shopping"), "{list}");
    let list: serde_json::Value =
        serde_json::from_str(&ok(home, &["--json", "ls", "笔记", "note"], "").await).unwrap();
    assert_eq!(list.as_array().map(Vec::len), Some(1));
    let found = ok(home, &["search", "eggs"], "").await;
    assert!(
        found.contains("note/shopping") && found.contains("笔记"),
        "{found}"
    );

    std::fs::write(home.join("hello.txt"), "hello").unwrap();
    let up = ok(
        home,
        &["upload", "笔记", home.join("hello.txt").to_str().unwrap()],
        "",
    )
    .await;
    assert!(up.starts_with("file/") && up.contains("hello.txt"), "{up}");

    ok(home, &["rm", "笔记", "note/shopping"], "").await;
    let (gone, _, _) = run(home, &["get", "笔记", "note/shopping"], "", &[]).await;
    assert!(!gone);

    // Tokens go through the API while the node runs.
    let token = ok(
        home,
        &[
            "ctl",
            "token",
            "create",
            "--role",
            "reader",
            "--space",
            "笔记",
            "--expires-in",
            "1d",
        ],
        "",
    )
    .await;
    assert_ne!(token.trim(), "");
    let tokens = ok(home, &["ctl", "token", "list"], "").await;
    assert!(
        tokens.contains("reader") && tokens.contains("笔记"),
        "{tokens}"
    );

    // A reader may look but not write.
    let reader = [("HS_TOKEN", token.trim())];
    let (read, _, err) = run(home, &["ls", "笔记"], "", &reader).await;
    assert!(read, "{err}");
    let (wrote, _, _) = run(home, &["put", "笔记", "note/x"], "{}", &reader).await;
    assert!(!wrote);

    assert!(ok(home, &["logout"], "").await.contains(&url));
    let (signed_in, _, _) = run(home, &["status"], "", &[]).await;
    assert!(!signed_in);
}

#[test]
fn looks_after_a_stopped_node() {
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    let dir = home.join("data/homeserver");
    let (made, out, err) = hs(home, &["ctl", "init", "--name", "Test"], "", &[]);
    assert!(made, "{err}");
    assert!(out.contains("signed in to http://127.0.0.1:8080"), "{out}");

    // The node is not running, so its directory is used directly.
    let (ok, out, err) = hs(home, &["space", "create", "Photos"], "", &[]);
    assert!(ok, "{err}");
    assert_eq!(out.trim().len(), 64);
    let (ok, out, _) = hs(home, &["space", "list"], "", &[]);
    assert!(ok && out.contains("Photos"), "{out}");
    let (ok, out, _) = hs(home, &["ctl", "token", "list"], "", &[]);
    assert!(ok && out.contains("owner"), "{out}");
    let (ok, out, _) = hs(home, &["--json", "identity"], "", &[]);
    assert!(ok && out.contains("did:hs:"), "{out}");
    assert!(dir.join("config.json").exists());

    // The API's description needs no node at all.
    let (ok, out, _) = hs(home, &["openapi"], "", &[]);
    let doc: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(ok && doc["paths"]["/api/spaces"].is_object(), "{out}");

    // Completion: the script calls `hs` back, which answers from its own
    // commands and from the node.
    let (ok, out, _) = hs(home, &["completion", "fish"], "", &[]);
    assert!(
        ok && out.contains("COMPLETE") && out.contains("hsctl"),
        "{out}"
    );
    let complete = |line: &[&str]| {
        let args: Vec<&str> = ["--", "hs"].iter().chain(line).copied().collect();
        let (ok, out, err) = hs(home, &args, "", &[("COMPLETE", "fish")]);
        assert!(ok, "{err}");
        out
    };
    let commands = complete(&[""]);
    assert!(
        commands.contains("ctl") && !commands.contains("internal"),
        "{commands}"
    );
    assert!(complete(&["ctl", ""]).contains("rebuild-index"));
    assert!(complete(&["ls", ""]).contains("Photos"));
    assert!(complete(&["ctl", "config", "get", ""]).contains("name"));

    // Commands only a running node answers say so.
    let (ok, _, err) = hs(home, &["ls", "Photos"], "", &[]);
    assert!(!ok);
    assert!(err.contains("cannot reach"), "{err}");
}

/// Started as `hsctl`, the program is `hs ctl`.
#[cfg(unix)]
#[test]
fn hsctl_is_hs_ctl() {
    let home = tempfile::tempdir().unwrap();
    let link = home.path().join("hsctl");
    std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_hs"), &link).unwrap();
    let out = Command::new(&link).arg("--help").output().unwrap();
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(
        help.contains("Usage: hsctl") && help.contains("rebuild-index"),
        "{help}"
    );
    assert!(!help.contains("timeline"), "{help}");
}
