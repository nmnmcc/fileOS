// SPDX-License-Identifier: AGPL-3.0-only

//! `COSE_Sign1` (RFC 9052) with `EdDSA`, the signature envelope of every log
//! entry. We use the untagged form; the context always says what it is.

use crate::cbor::{Error, Value};

/// COSE algorithm identifier for `EdDSA`.
pub const ALG_EDDSA: i64 = -8;

/// A `COSE_Sign1` message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sign1 {
    /// Key identifier: the signer's 32-byte public key.
    pub kid: Vec<u8>,
    /// The signed payload.
    pub payload: Vec<u8>,
    /// The Ed25519 signature over [`Sign1::to_be_signed`].
    pub signature: Vec<u8>,
}

fn protected() -> Vec<u8> {
    crate::cbor::map([(1, Value::int(ALG_EDDSA))]).encode()
}

impl Sign1 {
    /// The bytes that are signed: the `Sig_structure` of RFC 9052, section 4.4.
    pub fn to_be_signed(payload: &[u8]) -> Vec<u8> {
        Value::Array(vec![
            Value::Text("Signature1".into()),
            Value::Bytes(protected()),
            Value::Bytes(Vec::new()),
            Value::Bytes(payload.to_vec()),
        ])
        .encode()
    }

    /// Encode as CBOR.
    pub fn encode(&self) -> Vec<u8> {
        Value::Array(vec![
            Value::Bytes(protected()),
            crate::cbor::map([(4, Value::Bytes(self.kid.clone()))]),
            Value::Bytes(self.payload.clone()),
            Value::Bytes(self.signature.clone()),
        ])
        .encode()
    }

    /// Decode from CBOR. Only `EdDSA` is accepted.
    ///
    /// # Errors
    ///
    /// Returns an error if the CBOR, signature structure or algorithm is invalid.
    pub fn decode(bytes: &[u8]) -> Result<Sign1, Error> {
        let value = Value::decode(bytes)?;
        let items = value
            .as_array()
            .ok_or(Error("COSE_Sign1 is not an array"))?;
        let [prot, unprot, payload, signature] = items else {
            return Err(Error("COSE_Sign1 needs four items"));
        };
        if prot.as_bytes() != Some(&protected()[..]) {
            return Err(Error("unsupported COSE protected header"));
        }
        let kid = unprot
            .get(4)
            .and_then(Value::as_bytes)
            .ok_or(Error("missing kid"))?;
        Ok(Sign1 {
            kid: kid.to_vec(),
            payload: payload.as_bytes().ok_or(Error("payload"))?.to_vec(),
            signature: signature.as_bytes().ok_or(Error("signature"))?.to_vec(),
        })
    }
}
