// SPDX-License-Identifier: AGPL-3.0-only

//! A deterministic simulation of many fileOS nodes, for tests.
//!
//! Nothing here uses threads, clocks or the network, so a test gives the
//! same result every time. See [`world`].

pub mod world;
