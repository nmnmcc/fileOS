// SPDX-License-Identifier: AGPL-3.0-only

//! Networking for homeserver.
//!
//! Person-to-person federation and mutual aid: relaying traffic, holding
//! mail for offline friends and keeping encrypted backups for them.
//!
//! The protocol defines how help is requested and provided. Whom to help,
//! and how much, is a local policy chosen by each person; this crate never
//! hard-codes one.

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");
