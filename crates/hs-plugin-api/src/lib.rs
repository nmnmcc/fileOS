// SPDX-License-Identifier: MIT OR Apache-2.0

//! Plugin Interface for homeserver.
//!
//! Types and bindings shared by the plugin host and by plugins. This crate is
//! part of the Plugin Interface defined in the homeserver Plugin Exception and is
//! licensed under MIT OR Apache-2.0, so plugins may use any license.
//!
//! It must stay `no_std`, dependency-free and fast to compile, because every
//! plugin author builds it.

#![no_std]

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");
