// SPDX-License-Identifier: AGPL-3.0-only

//! The homeserver binary: one program, one data directory.
//!
//! Keep this crate thin. It wires the components together and nothing else,
//! so that editing it rebuilds in seconds.

use std::process::ExitCode;

const COMPONENTS: &[&str] = &[
    hs_core::NAME,
    hs_identity::NAME,
    hs_store::NAME,
    hs_net::NAME,
    hs_node::NAME,
    hs_plugin_api::NAME,
    hs_plugin_host::NAME,
];

const USAGE: &str = "usage: hs [--version | components]";

const NOTICE: &str = "\
License AGPL-3.0-only: GNU AGPL version 3 <https://www.gnu.org/licenses/agpl-3.0.html>
homeserver is free software and comes with ABSOLUTELY NO WARRANTY.";

fn main() -> ExitCode {
    match std::env::args().nth(1).as_deref() {
        None | Some("--version" | "-V") => {
            println!("hs {}\n{NOTICE}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        Some("components") => {
            for name in COMPONENTS {
                println!("{name}");
            }
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("hs: unknown command `{other}`\n{USAGE}");
            ExitCode::from(2)
        }
    }
}
