// SPDX-License-Identifier: AGPL-3.0-only

//! The derived index for homeserver.
//!
//! The signed space logs and content-addressed blobs are the truth. This crate
//! builds an SQLite index from them: the current version of each resource, the
//! links between resources, full-text search and places. The index never leaves
//! the node and can be deleted and rebuilt from the logs at any time. See the
//! architecture page in the documentation.

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");
