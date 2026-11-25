// SPDX-License-Identifier: AGPL-3.0-only

//! Resource version conditions evaluated by the primary before appending.

use hs_core::{Hash, cbor::Value};
use hs_net::server::Request;
use http::header;

use crate::util::header as hdr;

#[derive(Default)]
pub(crate) struct Conditions {
    if_match: Option<String>,
    if_none_match: Option<String>,
    expected: Option<Hash>,
    must_exist: bool,
}

impl Conditions {
    pub(crate) fn from_request(req: &Request, expected: Option<Hash>, must_exist: bool) -> Self {
        Self {
            if_match: hdr(req, header::IF_MATCH).map(str::to_owned),
            if_none_match: hdr(req, header::IF_NONE_MATCH).map(str::to_owned),
            expected,
            must_exist,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.if_match.is_none()
            && self.if_none_match.is_none()
            && self.expected.is_none()
            && !self.must_exist
    }

    pub(crate) fn check(&self, current: Option<Hash>) -> hs_node::Result<()> {
        let changed = || hs_node::Error::precondition_failed("the resource has changed");
        if self.expected.is_some() && self.expected != current {
            return Err(changed());
        }
        let tag = current.as_ref().map(crate::rest::etag);
        if let Some(expected) = &self.if_match
            && !tag
                .as_deref()
                .is_some_and(|tag| contains(expected, tag, false))
        {
            return Err(changed());
        }
        if let Some(excluded) = &self.if_none_match
            && tag
                .as_deref()
                .is_some_and(|tag| contains(excluded, tag, true))
        {
            return Err(hs_node::Error::precondition_failed(
                "the resource already exists",
            ));
        }
        if self.must_exist && current.is_none() {
            return Err(hs_node::Error::resource_missing());
        }
        Ok(())
    }

    pub(crate) fn encode(&self) -> Value {
        let text = |value: &Option<String>| {
            value
                .as_ref()
                .map_or(Value::Null, |s| Value::Text(s.clone()))
        };
        Value::Array(vec![
            text(&self.if_match),
            text(&self.if_none_match),
            self.expected
                .map_or(Value::Null, |hash| Value::Bytes(hash.0.to_vec())),
            Value::Bool(self.must_exist),
        ])
    }

    pub(crate) fn decode(value: &Value) -> Option<Self> {
        fn optional_text(value: &Value) -> Result<Option<String>, ()> {
            match value {
                Value::Null => Ok(None),
                Value::Text(text) => Ok(Some(text.clone())),
                _ => Err(()),
            }
        }
        let [if_match, if_none_match, expected, Value::Bool(must_exist)] = value.as_array()? else {
            return None;
        };
        Some(Self {
            if_match: optional_text(if_match).ok()?,
            if_none_match: optional_text(if_none_match).ok()?,
            expected: match expected {
                Value::Null => None,
                value => Some(Hash::from_slice(value.as_bytes()?)?),
            },
            must_exist: *must_exist,
        })
    }
}

fn contains(list: &str, tag: &str, weak: bool) -> bool {
    list.split(',').map(str::trim).any(|candidate| {
        let candidate = if weak {
            candidate.trim_start_matches("W/")
        } else {
            candidate
        };
        candidate == "*" || candidate == tag
    })
}
