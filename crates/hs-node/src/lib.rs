// SPDX-License-Identifier: AGPL-3.0-only

//! A homeserver node.
//!
//! Assembles identity, spaces, blobs, replicas, the derived index and the HTTP
//! API into one node. Browsers and phones are user agents that talk to a node
//! over HTTP; they do not run one.

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The components this node is built from, bottom first.
pub const COMPONENTS: &[&str] = &[hs_core::NAME, hs_identity::NAME, hs_store::NAME, NAME];
