// SPDX-License-Identifier: AGPL-3.0-only

//! Plugin host for homeserver.
//!
//! Not implemented yet. It will run WebAssembly plugins in a sandbox, where a
//! plugin can call only the host functions its manifest grants, within memory
//! and time limits.
//!
//! Plugins that use only the Plugin Interface are covered by the homeserver Plugin
//! Exception (see PLUGIN-EXCEPTION.md) and may use any license.

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");
