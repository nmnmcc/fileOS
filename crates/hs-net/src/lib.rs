// SPDX-License-Identifier: AGPL-3.0-only

//! Networking for homeserver: the HTTP server and client, signed
//! node-to-node requests, Web Push and router port mapping.
//!
//! Federation and mutual aid will come here later: relaying traffic, holding
//! mail for peers that are offline, and keeping encrypted backups for them.
//! The protocol says how nodes ask for help and give it. Each person sets
//! their own local policy for whom to help, and how much. This crate does
//! not fix one policy in the code.

pub mod client;
pub mod natpmp;
pub mod push;
pub mod server;
pub mod sig;

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");
