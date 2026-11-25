// SPDX-License-Identifier: AGPL-3.0-only

//! Plugin host for homeserver.
//!
//! Not built yet. It will run WebAssembly plugins in a sandbox. There a plugin
//! can call only the host functions its manifest grants, inside limits on
//! memory and time.
//!
//! Plugins that use only the Plugin Interface are covered by the homeserver Plugin
//! Exception (see PLUGIN-EXCEPTION.md) and may use any license.

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");
