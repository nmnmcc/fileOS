// SPDX-License-Identifier: AGPL-3.0-only

//! Request and response types shared by the REST server and [`crate::Client`].
//!
//! The `openapi` feature also turns on schema derives for the server's API
//! document.
//!
//! IDs are text. Spaces and payloads use their hash in hex. Nodes and
//! identities use their key ID. Times are milliseconds since 1970.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
#[cfg(feature = "openapi")]
use utoipa::{IntoParams, ToSchema};

/// What a role allows in a space.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Read.
    Reader,
    /// Read and write.
    Writer,
    /// Everything, including members and grants.
    Owner,
}

impl Role {
    /// `reader`, `writer` or `owner`.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Reader => "reader",
            Role::Writer => "writer",
            Role::Owner => "owner",
        }
    }
}

/// The body of every error: a message for people.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Error {
    /// What went wrong.
    pub error: String,
}

/// What a node says about itself to everyone (also at
/// `/.well-known/homeserver`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct NodeInfo {
    /// The owner's name for it.
    pub name: String,
    /// The owner's identity ID.
    pub identity: String,
    /// The owner's identity as a DID.
    pub did: String,
    /// The node's own key ID.
    pub node: String,
    /// The URL where others can reach it, if set.
    pub url: Option<String>,
    /// The software version.
    pub version: String,
}

/// Who is asking: a grant's token, another node, or nobody (`role` null).
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Session {
    /// The grant's ID, for a token.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant: Option<String>,
    /// The grant's role, for a token; null for nobody.
    #[serde(default)]
    pub role: Option<Role>,
    /// Whether the token is the owner's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<bool>,
    /// The spaces a token is limited to (hex); none means all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spaces: Option<Vec<String>>,
    /// The types a token is limited to; none means all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
    /// The key ID, for another node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// Another node's roles in spaces it shares with this one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<SpaceRole>>,
}

/// A role in one space.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SpaceRole {
    /// The space (hex).
    pub space: String,
    /// The role.
    pub role: Role,
}

/// Sign in with a token. The answer also sets the session cookie.
#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct SignIn {
    /// A bearer token.
    pub token: String,
}

impl std::fmt::Debug for SignIn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SignIn").finish_non_exhaustive()
    }
}

/// A space and its members.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Space {
    /// The space's ID (hex).
    pub id: String,
    /// Its name.
    pub name: String,
    /// What else its creator said about it.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub info: Json,
    /// How many changes it has had.
    pub seq: u64,
    /// The node that puts its changes in order (key ID).
    pub primary: String,
    /// Whether that is this node.
    pub is_primary: bool,
    /// The URL of the primary, if this node is not the primary.
    pub home: Option<String>,
    /// Its members.
    pub members: Vec<Member>,
}

/// A member of a space: a node, the identity it acts for, and a role.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Member {
    /// The node's key ID.
    pub node: String,
    /// The identity's ID.
    pub identity: String,
    /// The role.
    pub role: Role,
    /// The URL where the node can be reached.
    pub home: Option<String>,
}

/// Create a space.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct NewSpace {
    /// Its name.
    pub name: String,
    /// Anything else to say about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub info: Option<Json>,
}

/// Join a space kept by another node, which must already have added this
/// node as a member.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Join {
    /// The primary's URL.
    pub home: String,
    /// The space (hex).
    pub space: String,
}

/// Add a member: a node given by key and identity, or by `url`. With `url`,
/// this node asks that URL for the key and identity.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct NewMember {
    /// The node's key ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// The identity's ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// The role (writer if not given).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    /// The URL where the node can be reached, if not `url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// The node's URL, to look up its key and identity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// The answer when a member is added.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct MemberAdded {
    /// The change's sequence number.
    pub seq: u64,
    /// The member.
    pub member: Member,
}

/// Give the primary role to another member.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Handover {
    /// The new primary's key ID.
    pub node: String,
}

/// The answer when a change is recorded.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Seq {
    /// Its sequence number.
    pub seq: u64,
}

/// Changes got from a space's primary.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Pulled {
    /// How many.
    pub pulled: u64,
}

/// The answer when a resource is stored.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Written {
    /// The change's sequence number.
    pub seq: u64,
    /// The new `ETag`.
    pub etag: String,
}

/// A resource as lists, search and the timeline show it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Row {
    /// The space (hex).
    pub space: String,
    /// `type/id`.
    pub target: String,
    /// The type.
    #[serde(rename = "type")]
    pub kind: String,
    /// The payload hash (hex); null if erased.
    pub payload: Option<String>,
    /// The payload's media type.
    pub media_type: String,
    /// The sequence number of its last change.
    pub seq: u64,
    /// When it last changed.
    pub updated: u64,
    /// The time it is about: its own `time` field, or `updated`.
    pub time: u64,
    /// A title for lists.
    pub title: String,
    /// The payload, if it is JSON.
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub body: Option<Json>,
}

/// One page of rows (with `page=1`).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Page {
    /// The rows.
    pub items: Vec<Row>,
    /// The `offset` of the next page, if there is one.
    pub next_offset: Option<u32>,
}

/// Rows as lists send them: a bare array, or a `Page` with `page=1`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(untagged)]
pub enum Listing {
    /// Without `page=1`.
    Rows(Vec<Row>),
    /// With `page=1`.
    Page(Page),
}

/// A row, and how it is related to the row asked about.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Related {
    /// The relation, as named in the payload that links them.
    pub rel: String,
    /// The related resource.
    pub row: Row,
}

/// The answer when a blob is stored.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Blob {
    /// Its hash (hex).
    pub hash: String,
    /// Its size in bytes.
    pub size: u64,
}

/// A change, as `GET /api/events` sends it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Event {
    /// The space (hex).
    pub space: String,
    /// The change's sequence number.
    pub seq: u64,
    /// What happened: `put`, `delete`, `erase`, `member`, …
    pub kind: String,
    /// What it happened to.
    pub target: String,
}

/// A grant: what a token may do. The token itself is never shown again.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Grant {
    /// Its ID.
    pub id: String,
    /// The role.
    pub role: Role,
    /// A label for people.
    #[serde(default)]
    pub label: String,
    /// When it was made.
    #[serde(default)]
    pub created: u64,
    /// The spaces it is limited to (hex); none means all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spaces: Option<Vec<String>>,
    /// The types it is limited to; none means all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
    /// When it stops working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<u64>,
    /// Whether it is a sign-in session (from a passkey or an app).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<bool>,
}

/// Make a token.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct NewGrant {
    /// The role.
    pub role: Role,
    /// A label for people.
    #[serde(default)]
    pub label: String,
    /// Limit it to these spaces (names or IDs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spaces: Option<Vec<String>>,
    /// Limit it to these types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
    /// When it stops working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<u64>,
    /// Mark it as a sign-in session. Signing out revokes it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<bool>,
}

impl NewGrant {
    /// A grant with a role and label, and no limits.
    pub fn new(role: Role, label: &str) -> NewGrant {
        NewGrant {
            role,
            label: label.to_string(),
            spaces: None,
            types: None,
            expires: None,
            session: None,
        }
    }
}

/// A token made.
#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct NewToken {
    /// The bearer token, shown only this once.
    pub token: String,
    /// The grant's ID.
    pub id: String,
}

impl std::fmt::Debug for NewToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewToken")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn debug_redacts_tokens() {
        let token = "synthetic-token-for-debug-test";
        let sign_in = super::SignIn {
            token: token.into(),
        };
        let new_token = super::NewToken {
            token: token.into(),
            id: "grant-id".into(),
        };
        assert!(!format!("{sign_in:?}").contains(token));
        let debug = format!("{new_token:?}");
        assert!(!debug.contains(token));
        assert!(debug.contains("grant-id"));
    }
}

/// The node's health.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Health {
    /// Whether every check passed.
    pub ok: bool,
    /// Whether the index works.
    pub index: bool,
    /// Resources by type.
    pub counts: BTreeMap<String, u64>,
    /// How many spaces it keeps.
    pub spaces: u64,
    /// The node's current time.
    pub time: u64,
}

/// What one replica sync did.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ReplicaSync {
    /// The replica, without its secrets.
    pub replica: String,
    /// Whether it worked.
    pub ok: bool,
    /// Files sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sent: Option<u64>,
    /// Bytes sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    /// Files already there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unchanged: Option<u64>,
    /// Files removed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<u64>,
    /// What went wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Which resources to list.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct ListQuery {
    /// Only this type.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", param(rename = "type"))]
    pub kind: Option<String>,
    /// At most this many (up to 1000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Skip this many.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    /// `1` for a `Page` instead of a bare array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u8>,
}

/// What to search for.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct SearchQuery {
    /// Words to find.
    pub q: String,
    /// At most this many (default 50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// Which part of the timeline.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct TimelineQuery {
    /// Only what is about a time before this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<u64>,
    /// At most this many (up to 1000).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// Skip this many.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    /// `1` for a `Page` instead of a bare array.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u8>,
}

/// Which place to look near.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct NearQuery {
    /// Latitude in degrees.
    pub lat: f64,
    /// Longitude in degrees.
    pub lon: f64,
    /// How far, in kilometres (default 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub km: Option<f64>,
    /// At most this many (default 50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// How many.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct Limit {
    /// At most this many.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// How to send a blob back.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct BlobQuery {
    /// The media type to send it as (never one that runs scripts).
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "openapi", param(rename = "type"))]
    pub media_type: Option<String>,
    /// Send a thumbnail instead, this many pixels on its longer side (16 to
    /// 1024).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<u32>,
}

/// How to send a resource back.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(IntoParams))]
#[cfg_attr(feature = "openapi", into_params(parameter_in = Query))]
pub struct ResourceQuery {
    /// `vcard` or `ical`, for contacts and events stored as JSON (also by
    /// `Accept`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
}
