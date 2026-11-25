//! The spaces one can read, and a form to make a new one (owners only; the
//! server says no to anyone else).

use nsh_base::{button, theme};
use nsh_core::prelude::*;

use crate::{
    api::{self, Space},
    nav::{self, Route},
    store, ui,
};

pub(crate) fn spaces() -> impl IntoElement {
    let list = resource(
        move || Some(store::revision()),
        |_, _| async move { api::spaces().await },
    );
    let name = signal(String::new());
    let error = signal(String::new());
    let create = move || {
        let value = name.get_untracked().trim().to_owned();
        if value.is_empty() {
            return;
        }
        error.set(String::new());
        spawn(async move {
            match api::create_space(&value).await {
                Ok(id) => {
                    name.set(String::new());
                    store::changed();
                    nav::push(Route::Space(id));
                }
                Err(e) => error.set(t!("error", msg = e)),
            }
        });
    };
    scroll().flex_1().child(
        view()
            .pb_safe_area_plus(px(16.0))
            .child(each(
                move || list.get().unwrap_or_default(),
                |s: &Space| s.id.clone(),
                |space, _| space_item(space),
            ))
            .child(when(
                move || list.with(|l| l.is_some_and(Vec::is_empty)),
                || view().p_4().child(ui::muted(move || t!("nothing_yet"))),
            ))
            .child(when(
                move || list.error().is_some(),
                move || {
                    view().p_4().child(ui::error_text(move || {
                        let e = list.error().map(|e| e.to_string()).unwrap_or_default();
                        t!("error", msg = e)
                    }))
                },
            ))
            .child(
                view()
                    .gap_2()
                    .p_4()
                    .child(
                        text(move || t!("new_space"))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(move || theme().foreground),
                    )
                    .child(
                        view()
                            .flex_row()
                            .gap_2()
                            .child(
                                ui::field(name, "space_name")
                                    .flex_1()
                                    .on_submit(move |_: &str| create()),
                            )
                            .child(
                                ui::primary_button(move || t!("create"), create)
                                    .disabled(move || name.with(|n| n.trim().is_empty())),
                            ),
                    )
                    .child(when(
                        move || error.with(|e| !e.is_empty()),
                        move || ui::error_text(move || error.get()),
                    )),
            ),
    )
}

fn space_item(space: ReadSignal<Space>) -> impl IntoElement {
    button()
        .gap_1()
        .px_4()
        .py_3()
        .border_b_1()
        .border_b_color(move || theme().border)
        .active(|s| s.bg(move || theme().muted))
        .on_press(move || nav::push(Route::Space(space.with(|s| s.id.clone()))))
        .child(
            text(move || space.with(Space::label))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
        .child(ui::muted(move || {
            space.with(|s| t!("members_count", n = s.members.len()))
        }))
}
