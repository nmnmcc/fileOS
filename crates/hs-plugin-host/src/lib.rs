// SPDX-License-Identifier: AGPL-3.0-only

//! Plugin host for homeserver.
//!
//! Runs WebAssembly plugins in a sandbox. A plugin can only call the host
//! functions it was granted in its manifest, within memory and time limits.
//!
//! Plugins that use only the Plugin Interface are covered by the homeserver Plugin
//! Exception (see PLUGIN-EXCEPTION.md) and may use any license.

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");
