//! Full-text search over every readable space.

use nsh_base::theme;
use nsh_core::prelude::*;

use crate::{api, ui};

pub(crate) fn search() -> impl IntoElement {
    let draft = signal(String::new());
    // The query searched for: set on submit, not on every key.
    let query = signal(String::new());
    let results = resource(
        move || Some(query.get()).filter(|q| !q.is_empty()),
        |q: String, _| async move { api::search(&q).await },
    );
    let submit = move || query.set(draft.get_untracked().trim().to_owned());
    view()
        .flex_1()
        .child(
            view()
                .flex_row()
                .gap_2()
                .p_4()
                .child(
                    ui::field(draft, "search_placeholder")
                        .flex_1()
                        .on_submit(move |_: &str| submit()),
                )
                .child(
                    ui::primary_button(move || t!("search"), submit)
                        .disabled(move || draft.with(|d| d.trim().is_empty())),
                ),
        )
        .child(when(
            move || results.error().is_some(),
            move || {
                view().px_4().child(ui::error_text(move || {
                    let e = results.error().map(|e| e.to_string()).unwrap_or_default();
                    t!("error", msg = e)
                }))
            },
        ))
        .child(when(
            move || {
                !results.loading()
                    && !query.with(String::is_empty)
                    && results.with(|r| r.is_some_and(Vec::is_empty))
            },
            move || {
                view()
                    .px_4()
                    .child(ui::muted(move || t!("no_results", q = query.get())))
            },
        ))
        .child(
            scroll().flex_1().child(
                view()
                    .border_t_1()
                    .border_t_color(move || theme().border)
                    .child(each(
                        move || results.get().unwrap_or_default(),
                        api::Row::key,
                        |row, _| ui::row_item(row, true),
                    )),
            ),
        )
}
