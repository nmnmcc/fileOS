// SPDX-License-Identifier: AGPL-3.0-only

//! Access tokens: claims in CBOR, signed by the node key.
//!
//! `<base64url of the CBOR claims>.<base64url of the ed25519 signature>`.
//! Only the node that made a token reads it, so the format is ours; it is
//! not a JWT, so that no one takes it for one.

use fileos_core::{PublicKey, SecretKey, Signature};
use serde::{Deserialize, Serialize};

use crate::{Error, b64};

/// What an access token says.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    /// The person: a DID.
    pub sub: String,
    /// The app the token is for: its origin, such as
    /// `http://photos.localhost:7341`.
    pub aud: String,
    /// The thumbprint of the `DPoP` key the token is bound to.
    pub jkt: String,
    /// When it ends, in seconds since 1970.
    pub exp: u64,
}

const CONTEXT: &[u8] = b"fileOS 2026 access token\0";

/// Makes a token.
pub fn issue(claims: &Claims, key: &SecretKey) -> String {
    let body = fileos_core::encode(claims);
    let sig = key.sign(&[CONTEXT, &body].concat());
    format!("{}.{}", b64::encode(&body), b64::encode(&sig.0))
}

/// Reads a token that `key` made, if it has not ended at `now`.
///
/// # Errors
///
/// When the token is not good, was not made by `key`, or has ended.
pub fn check(token: &str, key: PublicKey, now: u64) -> Result<Claims, Error> {
    let bad = || Error("the access token is not good".into());
    let (body, sig) = token.split_once('.').ok_or_else(bad)?;
    let body = b64::decode(body).ok_or_else(bad)?;
    let sig: [u8; 64] = b64::decode(sig)
        .and_then(|s| s.try_into().ok())
        .ok_or_else(bad)?;
    if !key.verify(&[CONTEXT, &body].concat(), &Signature(sig)) {
        return Err(bad());
    }
    let claims: Claims = fileos_core::decode(&body).map_err(|_e| bad())?;
    if claims.exp <= now {
        return Err(Error("the access token has ended".into()));
    }
    Ok(claims)
}
