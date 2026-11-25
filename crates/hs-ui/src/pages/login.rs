//! Sign in with an access token (`hs grant` or `hs init` prints one).

use nsh_base::theme;
use nsh_core::prelude::*;

use crate::{api, store, ui};

pub(crate) fn login() -> impl IntoElement {
    let token = signal(String::new());
    let error = signal(String::new());
    let busy = signal(false);
    let submit = move || {
        let value = token.get_untracked().trim().to_owned();
        if value.is_empty() || busy.get_untracked() {
            return;
        }
        busy.set(true);
        error.set(String::new());
        spawn(async move {
            match api::sign_in(&value).await {
                Ok(_) => {
                    token.set(String::new());
                    store::signed_in();
                }
                Err(e) if matches!(e.status, 401 | 403) => error.set(t!("bad_token")),
                Err(e) => error.set(t!("error", msg = e)),
            }
            busy.set(false);
        });
    };
    view().flex_1().items_center().justify_center().p_4().child(
        view()
            .w_full()
            .max_w(px(380.0))
            .gap_4()
            .p_6()
            .border_1()
            .border_color(move || theme().border)
            .rounded(move || theme().radius)
            .bg(move || theme().card)
            .child(
                text("homeserver")
                    .text_2xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || theme().foreground),
            )
            .child(ui::muted(move || t!("token_hint")))
            .child(
                ui::field(token, "token")
                    .masked(true)
                    .auto_focus(true)
                    .on_submit(move |_: &str| submit()),
            )
            .child(
                ui::primary_button(move || t!("sign_in"), submit)
                    .disabled(move || busy.get() || token.with(|t| t.trim().is_empty())),
            )
            .child(when(
                move || error.with(|e| !e.is_empty()),
                move || ui::error_text(move || error.get()),
            )),
    )
}
