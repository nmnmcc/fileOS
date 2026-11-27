// SPDX-License-Identifier: AGPL-3.0-only

//! Sign-in through the gateway, as a browser does it: a passkey is added
//! with a setup code, the person lets the chat app act for them, the app
//! gets DPoP-bound tokens, and its requests reach it with the person in
//! `FileOS-User`. Uses the chat app from `target/apps` (run `apps` first).

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use fileos_auth::b64;
use fileos_auth::dpop::testing::Key;
use fileos_auth::webauthn::testing::Authenticator;
use fileos_node::Node;

struct Answer {
    status: u16,
    headers: String,
    body: String,
}

/// One HTTP/1.1 request to the gateway.
fn http(
    port: u16,
    method: &str,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> Answer {
    let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut req = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    s.write_all(req.as_bytes()).unwrap();
    let mut out = String::new();
    s.read_to_string(&mut out).unwrap();
    let (head, body) = out.split_once("\r\n\r\n").unwrap();
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    // The bodies here are small; a chunked body has its sizes around it.
    Answer {
        status,
        headers: head.to_ascii_lowercase(),
        body: body.to_string(),
    }
}

fn json(text: &str) -> serde_json::Value {
    serde_json::from_str(text.trim()).unwrap_or_else(|e| panic!("{e}: {text}"))
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[tokio::test(flavor = "multi_thread")]
async fn a_person_signs_in_to_an_app_with_a_passkey() {
    let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/apps");
    assert!(apps.join("Chat.app").exists(), "run `apps` first");
    let dir = tempfile::tempdir().unwrap();
    Node::init(dir.path()).unwrap();
    let node = Node::open(dir.path()).unwrap();
    node.create("/chat").unwrap();
    let chat = node.install(&apps.join("Chat.app")).unwrap().id;
    let token = fileos_node::token(dir.path()).unwrap();
    let port = fileos_gateway::start(node.clone(), 0, token.clone())
        .await
        .unwrap();
    let kernel = format!("http://fileos.localhost:{port}");
    let app = format!("http://{chat}.localhost:{port}");
    let mut passkey = Authenticator::new();
    let browser = Key::new();

    tokio::task::spawn_blocking(move || {
        // With no passkey yet, sign-in says what to do.
        let a = http(port, "POST", "fileos.localhost", "/authorize/challenge", &[], "{}");
        assert_eq!(a.status, 400);
        assert!(a.body.contains("fileos identity passkey"), "{}", a.body);

        // The owner makes a setup link with the node token, and adds a passkey.
        let a = http(port, "POST", "fileos.localhost", "/v0/auth/setup", &[("Authorization", &format!("Bearer {token}"))], "");
        assert_eq!(a.status, 200);
        let code = a.body.trim().to_string();
        let a = http(port, "GET", "fileos.localhost", "/setup", &[], "");
        assert!(a.headers.contains("script-src 'self'"));
        let c = json(&http(port, "POST", "fileos.localhost", "/setup/challenge", &[], "{}").body);
        let challenge = c["challenge"].as_str().unwrap().to_string();
        let (client, att) = passkey.create(&b64::decode(&challenge).unwrap(), &kernel, "fileos.localhost");
        let body = serde_json::json!({"code": code, "challenge": challenge, "client_data": b64::encode(&client), "attestation": b64::encode(&att)});
        let a = http(port, "POST", "fileos.localhost", "/setup/finish", &[], &body.to_string());
        assert_eq!(a.status, 200, "{}", a.body);

        // The app sends the browser to /authorize; the page is there.
        let verifier = "v".repeat(50);
        let challenge_pkce = fileos_auth::pkce::challenge(&verifier);
        let redirect = format!("{app}/callback");
        let q = format!("/authorize?response_type=code&client_id={app}&redirect_uri={redirect}&code_challenge={challenge_pkce}&code_challenge_method=S256&state=xyz");
        assert_eq!(http(port, "GET", "fileos.localhost", &q, &[], "").status, 200);
        let bad = q.replace(&format!("client_id={app}"), "client_id=http://evil.localhost");
        assert_eq!(http(port, "GET", "fileos.localhost", &bad, &[], "").status, 400);

        // The person signs in with the passkey.
        let c = json(&http(port, "POST", "fileos.localhost", "/authorize/challenge", &[], "{}").body);
        let challenge = c["challenge"].as_str().unwrap().to_string();
        let ans = passkey.get(&b64::decode(&challenge).unwrap(), &kernel, "fileos.localhost");
        let body = serde_json::json!({
            "client_id": app, "redirect_uri": redirect, "code_challenge": challenge_pkce, "state": "xyz",
            "challenge": challenge, "id": c["ids"][0], "client_data": b64::encode(&ans.client_data),
            "auth_data": b64::encode(&ans.auth_data), "signature": b64::encode(&ans.signature),
        });
        let a = http(port, "POST", "fileos.localhost", "/authorize/finish", &[], &body.to_string());
        assert_eq!(a.status, 200, "{}", a.body);
        let to = json(&a.body)["redirect"].as_str().unwrap().to_string();
        let code = to.split("code=").nth(1).unwrap().split('&').next().unwrap().to_string();
        assert!(to.ends_with("&state=xyz"));

        // The app changes the code for tokens, from its own origin, with DPoP.
        let token_url = format!("{kernel}/token");
        let form = format!("grant_type=authorization_code&code={code}&code_verifier={verifier}&client_id={app}&redirect_uri={redirect}");
        let proof = browser.proof("POST", &token_url, now(), "t1", None);
        let a = http(port, "POST", "fileos.localhost", "/token", &[("Origin", &app), ("Sec-Fetch-Site", "cross-site"), ("Content-Type", "application/x-www-form-urlencoded"), ("DPoP", &proof)], &form);
        assert_eq!(a.status, 200, "{}", a.body);
        assert!(a.headers.contains(&format!("access-control-allow-origin: {app}")));
        let tokens = json(&a.body);
        let access = tokens["access_token"].as_str().unwrap().to_string();

        // The app's own requests now say who it is.
        let path = "/";
        let proof = browser.proof("GET", &format!("{app}{path}"), now(), "r1", Some(&access));
        let a = http(port, "GET", &format!("{chat}.localhost"), path, &[("Authorization", &format!("DPoP {access}")), ("DPoP", &proof)], "");
        assert_eq!(a.status, 200, "{}", a.body);
        assert!(a.body.contains("Signed in: did:key:"), "{}", a.body);
        // With no sign-in, the app gets no person.
        let a = http(port, "GET", &format!("{chat}.localhost"), path, &[], "");
        assert!(a.body.contains("Not signed in"));
        // A stolen token with another key, or the same proof again: 401.
        let thief = Key::new().proof("GET", &format!("{app}{path}"), now(), "r2", Some(&access));
        let a = http(port, "GET", &format!("{chat}.localhost"), path, &[("Authorization", &format!("DPoP {access}")), ("DPoP", &thief)], "");
        assert_eq!(a.status, 401);
        let a = http(port, "GET", &format!("{chat}.localhost"), path, &[("Authorization", &format!("DPoP {access}")), ("DPoP", &proof)], "");
        assert_eq!(a.status, 401);

        // A site that is not an app of this node gets no tokens.
        let proof = browser.proof("POST", &token_url, now(), "t2", None);
        let a = http(port, "POST", "fileos.localhost", "/token", &[("Origin", "http://evil.localhost"), ("DPoP", &proof)], &form.replace(&app, "http://evil.localhost"));
        assert_eq!(a.status, 400);
        assert!(!a.headers.contains("access-control-allow-origin"));

        // Taking back the sign-in ends the access token too.
        let refresh = tokens["refresh_token"].as_str().unwrap();
        let a = http(port, "POST", "fileos.localhost", "/revoke", &[("Origin", &app)], &format!("token={refresh}"));
        assert_eq!(a.status, 200);
        let proof = browser.proof("GET", &format!("{app}{path}"), now(), "r3", Some(&access));
        let a = http(port, "GET", &format!("{chat}.localhost"), path, &[("Authorization", &format!("DPoP {access}")), ("DPoP", &proof)], "");
        assert_eq!(a.status, 401);
    })
    .await
    .unwrap();
    // The passkey is kept on disk.
    assert!(dir.path().join("auth.cbor").exists());
}
