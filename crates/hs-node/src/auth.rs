// SPDX-License-Identifier: AGPL-3.0-only

//! Grants: the one permission model.
//!
//! A grant is a resource in the private space, written by an owner. It names
//! the spaces it covers and the role it gives. For bearer tokens, it also
//! has the BLAKE3 hash of the token. The token itself is shown once and
//! never stored. A `revoke` entry revokes a grant. Every request runs with
//! an [`Access`]: the owner, a grant, another node (by its key, as a member
//! of a space) or nobody.

use hs_core::event::Kind;
use hs_core::{Hash, PublicKey};
use hs_space::Role;
use serde_json::{Value as Json, json};

use crate::{Error, Node, Result, now_ms, random};

/// Who is asking.
#[derive(Clone, Debug)]
pub enum Access {
    /// Anyone, not authenticated.
    Public,
    /// Someone with a grant's token.
    Grant {
        /// The grant ID.
        id: String,
        /// The role it gives.
        role: Role,
        /// The spaces it covers (`None`: all).
        spaces: Option<Vec<Hash>>,
        /// The resource types it is limited to (`None`: all).
        types: Option<Vec<String>>,
    },
    /// Another node, which proved who it is with its key.
    Node {
        /// The node key.
        key: PublicKey,
        /// Its role in each space it is a member of.
        roles: Vec<(Hash, Role)>,
    },
}

impl Access {
    /// The grant ID to record in proposals.
    pub fn grant_id(&self) -> Option<String> {
        match self {
            Access::Grant { id, .. } => Some(id.clone()),
            _ => None,
        }
    }

    /// The caller's role in a space.
    pub fn role(&self, space: &Hash) -> Option<Role> {
        match self {
            Access::Public => None,
            Access::Grant { role, spaces, .. } => spaces
                .as_ref()
                .is_none_or(|s| s.contains(space))
                .then_some(*role),
            Access::Node { roles, .. } => roles.iter().find(|(s, _)| s == space).map(|(_, r)| *r),
        }
    }

    /// Whether the caller may read a space.
    pub fn can_read(&self, space: &Hash) -> bool {
        self.role(space).is_some()
    }

    /// Whether this is the owner (an owner grant for all spaces).
    pub fn is_owner(&self) -> bool {
        matches!(
            self,
            Access::Grant {
                role: Role::Owner,
                spaces: None,
                ..
            }
        )
    }

    /// Return an error if the caller does not have at least `needed` in
    /// `space` for `target`.
    ///
    /// # Errors
    ///
    /// Returns a permission error if the grant does not cover the role, space
    /// or resource type.
    pub fn require(&self, space: &Hash, needed: Role, target: &str) -> Result<()> {
        let role = self
            .role(space)
            .ok_or_else(|| Error::forbidden("no access to this space".into()))?;
        if role < needed {
            return Err(Error::forbidden(format!("needs {}", needed.as_str())));
        }
        if let Access::Grant {
            types: Some(types), ..
        } = self
        {
            let t = target.split('/').next().unwrap_or("");
            if !types.iter().any(|x| x == t) {
                return Err(Error::forbidden(format!("grant does not cover {t}")));
            }
        }
        Ok(())
    }
}

/// Hash a bearer token, to store it and look it up.
pub fn token_hash(token: &str) -> String {
    data_encoding::HEXLOWER.encode(&blake3::derive_key(
        "homeserver 2025 token v1",
        token.as_bytes(),
    ))
}

fn parse_grant(body: &Json) -> Option<Access> {
    let id = body.get("id")?.as_str()?.to_string();
    let role = Role::parse(body.get("role")?.as_str()?)?;
    let spaces = match body.get("spaces") {
        Some(Json::Array(a)) => Some(
            a.iter()
                .filter_map(|s| s.as_str().and_then(Hash::from_hex))
                .collect(),
        ),
        _ => None,
    };
    let types = match body.get("types") {
        Some(Json::Array(a)) => Some(
            a.iter()
                .filter_map(|s| s.as_str().map(String::from))
                .collect(),
        ),
        _ => None,
    };
    Some(Access::Grant {
        id,
        role,
        spaces,
        types,
    })
}

impl Node {
    /// The owner, acting locally (the CLI).
    pub fn owner(&self) -> Access {
        Access::Grant {
            id: "local".into(),
            role: Role::Owner,
            spaces: None,
            types: None,
        }
    }

    /// Create a grant with a bearer token. Returns the token (shown once)
    /// and the grant ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the new grant cannot be written to the private
    /// space.
    pub async fn create_token(
        &self,
        role: Role,
        spaces: Option<Vec<Hash>>,
        label: &str,
        extra: Option<Json>,
    ) -> Result<(String, String)> {
        let token = format!("hs_{}", hs_core::base32::encode(&random::<20>()));
        let id = hs_core::base32::encode(&random::<10>());
        let mut body = json!({
            "id": id,
            "role": role.as_str(),
            "label": label,
            "token": token_hash(&token),
            "created": now_ms(),
        });
        if let Some(s) = spaces {
            body["spaces"] = Json::Array(s.iter().map(|h| Json::String(h.to_hex())).collect());
        }
        if let Some(Json::Object(extra)) = extra {
            for (k, v) in extra {
                body[k] = v;
            }
        }
        self.add_grant(body).await?;
        Ok((token, id))
    }

    /// Write a grant (its body must have an `id`).
    ///
    /// # Errors
    ///
    /// Returns an error if the grant has no ID or cannot be written to the
    /// private space.
    pub async fn add_grant(&self, body: Json) -> Result<()> {
        let private = self.config().private;
        let bytes = serde_json::to_vec(&body).unwrap_or_default();
        let id = body.get("id").and_then(Json::as_str).unwrap_or("");
        self.write(
            &self.owner(),
            &private,
            Kind::Grant,
            &format!("grant/{id}"),
            Some(&bytes),
            "application/json",
            None,
        )
        .await?;
        Ok(())
    }

    /// All grants (not revoked), newest last.
    pub async fn grants(&self) -> Vec<Json> {
        let private = self.config().private;
        self.with_space(&private, |l| {
            l.space
                .grants
                .values()
                .filter(|g| !g.revoked)
                .map(|g| g.body.clone())
                .collect()
        })
        .await
        .unwrap_or_default()
    }

    /// Revoke a grant.
    ///
    /// # Errors
    ///
    /// Returns an error if the revocation cannot be written to the private
    /// space.
    pub async fn revoke(&self, id: &str) -> Result<()> {
        let private = self.config().private;
        self.write(
            &self.owner(),
            &private,
            Kind::Revoke,
            &format!("grant/{id}"),
            None,
            "",
            None,
        )
        .await?;
        Ok(())
    }

    /// Look up a bearer token.
    pub async fn authenticate(&self, token: &str) -> Option<Access> {
        let h = token_hash(token);
        let now = now_ms();
        self.grants().await.into_iter().find_map(|g| {
            let hash = g.get("token").and_then(Json::as_str)?;
            if !hs_core::ct_eq(hash.as_bytes(), h.as_bytes()) {
                return None;
            }
            if g.get("expires")
                .and_then(Json::as_u64)
                .is_some_and(|e| e < now)
            {
                return None;
            }
            parse_grant(&g)
        })
    }

    /// The access of another node, by its key: its roles in our spaces.
    pub async fn node_access(&self, key: PublicKey) -> Access {
        let mut roles = Vec::new();
        for id in self.space_ids() {
            if let Ok(Some(r)) = self.with_space(&id, |l| l.space.role(&key)).await {
                roles.push((id, r));
            }
        }
        if roles.is_empty() {
            Access::Public
        } else {
            Access::Node { key, roles }
        }
    }
}
