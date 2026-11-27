// SPDX-License-Identifier: AGPL-3.0-only

//! A deterministic simulation of many fileOS nodes, for tests.
//!
//! Nothing here uses threads, clocks or the network, so a test gives the
//! same result every time.

//! Enable the `test-util` feature to use the simulated nodes in another crate.

#[cfg(any(test, feature = "test-util"))]
pub mod world;
