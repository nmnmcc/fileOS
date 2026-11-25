// SPDX-License-Identifier: AGPL-3.0-only

//! Core object model of homeserver.
//!
//! Everything a person keeps on their homeserver (files, photos, messages, notes,
//! contacts, events, passwords) is an object. Objects are organized around
//! people, time, places and things, not around apps.
//!
//! This crate sits at the bottom of the dependency graph. It must stay small
//! and must not depend on any other homeserver crate or on heavy external crates.

/// The name of this component.
pub const NAME: &str = env!("CARGO_PKG_NAME");
