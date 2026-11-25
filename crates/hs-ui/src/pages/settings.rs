//! This node, and signing out.

use nsh_base::theme;
use nsh_core::prelude::*;

use crate::{api, store, ui};

pub(crate) fn settings() -> impl IntoElement {
    let node = resource(|| Some(()), |(), _| async move { api::node().await });
    let signing_out = signal(false);
    let sign_out_error = signal(String::new());
    let line = move |key: &'static str, value: fn(&api::Node) -> String| {
        view().gap_1().child(ui::muted(move || t!(key))).child(
            text(move || node.with(|n| n.map(value).unwrap_or_default()))
                .text_color(move || theme().foreground),
        )
    };
    scroll().flex_1().child(
        view()
            .gap_4()
            .p_4()
            .pb_safe_area_plus(px(16.0))
            .child(line("name", |n| n.name.clone()))
            .child(line("identity", |n| n.identity.clone()))
            .child(line("url", |n| n.url.clone().unwrap_or_default()))
            .child(line("version", |n| n.version.clone()))
            .child(when(
                move || node.error().is_some(),
                move || {
                    ui::error_text(move || {
                        let e = node.error().map(|e| e.to_string()).unwrap_or_default();
                        t!("error", msg = e)
                    })
                },
            ))
            .child(
                ui::outline_button(
                    move || t!("sign_out"),
                    move || {
                        if signing_out.get_untracked() {
                            return;
                        }
                        signing_out.set(true);
                        sign_out_error.set(String::new());
                        spawn(async move {
                            match api::sign_out().await {
                                Ok(()) => store::signed_out(),
                                Err(error) => {
                                    sign_out_error.set(t!("error", msg = error));
                                    signing_out.set(false);
                                }
                            }
                        });
                    },
                )
                .disabled(move || signing_out.get())
                .self_start(),
            )
            .child(when(
                move || sign_out_error.with(|error| !error.is_empty()),
                move || ui::error_text(move || sign_out_error.get()),
            )),
    )
}
