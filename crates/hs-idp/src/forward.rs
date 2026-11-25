// SPDX-License-Identifier: AGPL-3.0-only

//! Forward-auth sessions for reverse proxies (Caddy, Traefik, nginx).
//!
//! For every request, the proxy asks if the session cookie is valid. A cookie
//! value is `sub.exp.sig`: the base64url subject, the expiry time in unix
//! seconds, and a base64url HMAC-SHA256 over the first two parts.

use ring::hmac;

use crate::jose::{b64d, b64e};
use crate::percent_encode;

/// The result of a forward-auth check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// The session is valid for this subject.
    Allow {
        /// The signed-in subject.
        sub: String,
    },
    /// Send the browser to this login URL.
    Redirect(String),
}

/// Makes and checks session cookies with a server secret.
pub struct Sessions {
    key: hmac::Key,
}

impl std::fmt::Debug for Sessions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Sessions")
    }
}

impl Sessions {
    /// Sessions signed with a 32-byte secret.
    pub fn new(secret: [u8; 32]) -> Sessions {
        Sessions {
            key: hmac::Key::new(hmac::HMAC_SHA256, &secret),
        }
    }

    /// A cookie value for `sub`, valid for `lifetime` seconds from `now`.
    pub fn issue(&self, sub: &str, now: u64, lifetime: u64) -> String {
        let body = format!("{}.{}", b64e(sub.as_bytes()), now.saturating_add(lifetime));
        let tag = hmac::sign(&self.key, body.as_bytes());
        format!("{body}.{}", b64e(tag.as_ref()))
    }

    /// The subject of a cookie value that is valid and has not expired.
    pub fn verify(&self, value: &str, now: u64) -> Option<String> {
        let (body, sig) = value.rsplit_once('.')?;
        let (sub, exp) = body.split_once('.')?;
        hmac::verify(&self.key, body.as_bytes(), &b64d(sig).ok()?).ok()?;
        if exp.parse::<u64>().ok()? <= now {
            return None;
        }
        String::from_utf8(b64d(sub).ok()?).ok()
    }

    /// Decide what the proxy does with a request: let it through, or redirect
    /// to `login_url?rd=<original_url>`.
    pub fn decide(
        &self,
        cookie_value: Option<&str>,
        now: u64,
        login_url: &str,
        original_url: &str,
    ) -> Decision {
        if let Some(sub) = cookie_value.and_then(|v| self.verify(v, now)) {
            Decision::Allow { sub }
        } else {
            let sep = if login_url.contains('?') { '&' } else { '?' };
            Decision::Redirect(format!(
                "{login_url}{sep}rd={}",
                percent_encode(original_url)
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_and_verify() {
        let s = Sessions::new([1; 32]);
        let c = s.issue("alice", 100, 60);
        assert_eq!(s.verify(&c, 159).as_deref(), Some("alice"));
        assert_eq!(s.verify(&c, 160), None, "expired");
        assert_eq!(Sessions::new([2; 32]).verify(&c, 100), None, "other secret");

        // Changing any part breaks the signature.
        let parts: Vec<&str> = c.split('.').collect();
        let forged = format!("{}.{}.{}", b64e(b"admin"), parts[1], parts[2]);
        assert_eq!(s.verify(&forged, 100), None);
        let extended = format!("{}.{}.{}", parts[0], 999_999, parts[2]);
        assert_eq!(s.verify(&extended, 100), None);
        for junk in ["", ".", "..", "a.b.c", "a.b.c.d", "!!.1.!!"] {
            assert_eq!(s.verify(junk, 100), None);
        }
    }

    #[test]
    fn decide() {
        let s = Sessions::new([1; 32]);
        let c = s.issue("alice", 100, 60);
        assert_eq!(
            s.decide(Some(&c), 120, "https://id.example/login", "https://app/x"),
            Decision::Allow {
                sub: "alice".into()
            }
        );
        assert_eq!(
            s.decide(None, 120, "https://id.example/login", "https://app/x?a=1"),
            Decision::Redirect("https://id.example/login?rd=https%3A%2F%2Fapp%2Fx%3Fa%3D1".into())
        );
        assert!(matches!(
            s.decide(Some(&c), 500, "https://id.example/login?f=1", "u"),
            Decision::Redirect(u) if u == "https://id.example/login?f=1&rd=u"
        ));
    }
}
