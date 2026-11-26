// SPDX-License-Identifier: MIT OR Apache-2.0

//! The SDK for fileOS apps and engines.
//!
//! An app implements [`app::exports::fos::app::events::Guest`] and calls
//! [`app::export_app!`]. An engine implements
//! [`engine::exports::fos::app::authz::Guest`] and calls [`engine::export_engine!`].
//! The interfaces are in `wit/fos.wit`.

#![allow(unsafe_code, reason = "the code that wit-bindgen makes uses unsafe")]

/// Bindings for the `app` world.
#[allow(missing_docs, clippy::pedantic, reason = "made by wit-bindgen")]
pub mod app {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "app",
        pub_export_macro: true,
        export_macro_name: "export_app",
        default_bindings_module: "fos_app::app",
    });
}

/// Bindings for the `engine` world.
#[allow(missing_docs, clippy::pedantic, reason = "made by wit-bindgen")]
pub mod engine {
    wit_bindgen::generate!({
        path: "../../wit",
        world: "engine",
        pub_export_macro: true,
        export_macro_name: "export_engine",
        default_bindings_module: "fos_app::engine",
    });
}
