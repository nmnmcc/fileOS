// SPDX-License-Identifier: AGPL-3.0-only

//! Spaces, with no I/O: signed logs in a causal graph, files with fixed IDs,
//! and grants with strong removal.
//!
//! This is the data model of the new kernel (see the docs pages "The model"
//! and "Data"). A [`Space`] is the unit of sync, owners and keys. Every
//! writer has its own signed log, and the logs make one causal graph. There
//! is no ordering node: every node can write at any time, also when offline,
//! and every node that has the same entries gets the same state.
//!
//! - [`entry`]: the signed entries and what they do ([`Act`]).
//! - [`content`]: content in chunks, cut at points the content picks.
//! - [`space`]: one space, and the state that comes from its entries.
//! - [`sync`]: a node with many spaces, and the messages between nodes.
//!
//! This crate is portable: no threads, files, sockets or clocks.

pub mod content;
pub mod crypto;
pub mod entry;
pub mod space;
pub mod sync;

pub use entry::{
    Act, Content, Effect, Entry, Header, Id, KeyRef, Permission, Principal, Resource, Retention,
    Sealed,
};
pub use space::{File, Given, Grant, Keyring, Snapshot, Space, Summary, Version};
pub use sync::{Contents, Event, Msg, Node, Step};

#[cfg(test)]
mod tests;
