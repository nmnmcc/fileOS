// SPDX-License-Identifier: AGPL-3.0-only

//! Networking for homeserver: the HTTP server and client, signed
//! node-to-node requests, ACME certificates, Web Push and router port
//! mapping.
//!
//! Federation and mutual aid (relaying traffic, holding mail for offline
//! peers, keeping encrypted backups for them) are planned here. The protocol
//! defines how help is requested and given; whom to help, and how much, is
//! local policy set by each person, and this crate does not hard-code one.

pub mod acme;
pub mod client;
pub mod natpmp;
pub mod push;
pub mod server;
pub mod sig;

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");
