// SPDX-License-Identifier: AGPL-3.0-only

//! Sign-in over real HTTP: an app gets a token with OAuth 2.0 and PKCE, and
//! is told who the person is. Uses the photos app from `target/apps`.

use std::io::{Read as _, Write as _};
use std::net::TcpStream;
use std::path::Path;

use fileos_gateway::signin::challenge_of;
use fileos_node::Node;

/// Sends one HTTP/1.1 request and gives back the status, the headers and
/// the body.
fn http(
    port: u16,
    method: &str,
    host: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &str,
) -> (u16, String, String) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in headers {
        request.push_str(&format!("{k}: {v}\r\n"));
    }
    request.push_str("\r\n");
    request.push_str(body);
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = Vec::new();
    stream.read_to_end(&mut answer).unwrap();
    let answer = String::from_utf8_lossy(&answer).into_owned();
    let (head, body) = answer.split_once("\r\n\r\n").unwrap_or((&answer, ""));
    let status = head.split(' ').nth(1).unwrap().parse().unwrap();
    (status, head.to_string(), body.to_string())
}

fn header<'a>(head: &'a str, name: &str) -> Option<&'a str> {
    head.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

fn enc(text: &str) -> String {
    percent_encoding::utf8_percent_encode(text, percent_encoding::NON_ALPHANUMERIC).to_string()
}

#[test]
fn an_app_gets_a_token_and_the_person() {
    let apps = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/apps");
    assert!(apps.join("Photos.app").exists(), "run `apps` first");
    let dir = tempfile::tempdir().unwrap();
    Node::init(dir.path()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let node = runtime.block_on(async { Node::open(dir.path()).unwrap() });
    let app = node.install(&apps.join("Photos.app")).unwrap().id;
    let token = fileos_node::token(dir.path()).unwrap();
    let port = runtime
        .block_on(fileos_gateway::start(node.clone(), 0, token.clone()))
        .unwrap();
    let host = format!("{app}.localhost:{port}");
    let client = format!("http://{host}");
    let redirect = format!("{client}/back");
    let verifier = "a-verifier-that-is-long-enough-for-pkce-0123456789";

    // Without a token, the app sees no one.
    let (_, _, page) = http(port, "GET", &host, "/", &[], "");
    assert!(page.contains("Not signed in"), "{page}");

    // The person gets a code on the command line.
    let (status, _, code) = http(
        port,
        "POST",
        "fileos.localhost",
        "/v0/sign-in-code",
        &[("Authorization", &format!("Bearer {token}"))],
        "",
    );
    assert_eq!(status, 200);
    let code = code.trim();

    // The app sends the browser to /authorize.
    let query = format!(
        "response_type=code&client_id={}&redirect_uri={}&code_challenge={}&code_challenge_method=S256&state=s1",
        enc(&client),
        enc(&redirect),
        challenge_of(verifier)
    );
    let (status, _, page) = http(
        port,
        "GET",
        "fileos.localhost",
        &format!("/authorize?{query}"),
        &[],
        "",
    );
    assert_eq!(status, 200, "{page}");
    assert!(page.contains(&app));

    // A form from another site is refused.
    let form = format!("{query}&answer=yes&sign_in_code={code}");
    let (status, _, _) = http(
        port,
        "POST",
        "fileos.localhost",
        "/authorize",
        &[
            ("Sec-Fetch-Site", "cross-site"),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        &form,
    );
    assert_eq!(status, 403);
    // The person signs in on the kernel's page.
    let (status, head, _) = http(
        port,
        "POST",
        "fileos.localhost",
        "/authorize",
        &[
            ("Sec-Fetch-Site", "same-origin"),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        &form,
    );
    assert_eq!(status, 303);
    let location = header(&head, "location").unwrap();
    assert!(
        location.starts_with(&format!("{redirect}?code=")),
        "{location}"
    );
    assert!(location.ends_with("&state=s1"));
    let auth_code = location
        .split("code=")
        .nth(1)
        .unwrap()
        .split('&')
        .next()
        .unwrap();

    // The app changes the code for a token, from its own origin.
    let form = format!(
        "grant_type=authorization_code&code={auth_code}&client_id={}&redirect_uri={}&code_verifier={verifier}",
        enc(&client),
        enc(&redirect)
    );
    let (status, head, json) = http(
        port,
        "POST",
        "fileos.localhost",
        "/token",
        &[
            ("Origin", &client),
            ("Content-Type", "application/x-www-form-urlencoded"),
        ],
        &form,
    );
    assert_eq!(status, 200, "{json}");
    assert_eq!(
        header(&head, "access-control-allow-origin"),
        Some(client.as_str())
    );
    let access = json
        .split("\"access_token\":\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap();

    // With the token, the app is told who the person is.
    let (_, _, page) = http(
        port,
        "GET",
        &host,
        "/",
        &[("Authorization", &format!("Bearer {access}"))],
        "",
    );
    assert!(
        page.contains(&format!("Signed in: {}", node.key())),
        "{page}"
    );

    // After /revoke, the token is of no use.
    http(
        port,
        "POST",
        "fileos.localhost",
        "/revoke",
        &[("Origin", &client)],
        &format!("token={access}"),
    );
    let (_, _, page) = http(
        port,
        "GET",
        &host,
        "/",
        &[("Authorization", &format!("Bearer {access}"))],
        "",
    );
    assert!(page.contains("Not signed in"), "{page}");
}
