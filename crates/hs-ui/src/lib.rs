//! homeserver's web interface, a Native Shell app: `nsh.json` `"root"`
//! names [`App`]. `nsh package --target web` builds it for the browser, and
//! `hs` serves the result from the same origin as its API.

mod api;
mod nav;
mod pages;
mod shell;
mod store;
mod ui;

use nsh_base::theme;
use nsh_core::{element::desktop::window as app_window, prelude::*};

use crate::store::Session;

/// The app's UI set (`nsh.json` `"rust"`).
#[native_module("hs-ui/native")]
fn native(registry: &mut Registry) {
    registry.register::<App>();
}

/// The app's root: the sign-in page or the shell, on the theme's
/// background. It asks the server once whether a session exists.
#[component]
pub fn App() -> impl IntoElement {
    spawn(async {
        match api::session().await {
            Ok(_) => store::signed_in(),
            Err(_) => store::signed_out(),
        }
    });
    let content = || {
        view()
            .flex_1()
            .bg(move || theme().background)
            .child(when(
                || store::session() == Session::Checking,
                || {
                    view()
                        .flex_1()
                        .items_center()
                        .justify_center()
                        .child(ui::muted(move || t!("loading")))
                },
            ))
            .child(when(
                || store::session() == Session::SignedOut,
                pages::login,
            ))
            .child(when(|| store::session() == Session::SignedIn, shell::shell))
    };
    app_window().id("main").content(content)
}

/// The translations in `locales/`, as `nsh package` hands them to the root;
/// for hosts and tests that build the root themselves.
#[doc(hidden)]
pub const TRANSLATIONS: &str = concat!(
    r#"{"en":"#,
    include_str!("../locales/en.json"),
    r#","zh-CN":"#,
    include_str!("../locales/zh-CN.json"),
    "}"
);
