// SPDX-License-Identifier: AGPL-3.0-only

//! Identity for homeserver.
//!
//! A person is identified by their own key pair; a domain is only an alias.
//! Each device has its own sub-key that can be revoked on its own, and a
//! signed migration statement lets a person move to another server while
//! keeping their contacts and groups.

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");
