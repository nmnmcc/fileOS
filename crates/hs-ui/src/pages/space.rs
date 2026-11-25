//! A space: its resources, by type, and a new note.

use nsh_base::{nav_page, tab, tabs, theme};
use nsh_core::prelude::*;

use crate::{
    api,
    nav::{self, Route},
    ui,
};

/// The types offered as filters; others show under "All".
const TYPES: [&str; 4] = ["note", "file", "contact", "event"];

pub(crate) fn space(id: String) -> Screen {
    let info = {
        let id = id.clone();
        resource(
            move || Some(id.clone()),
            |id: String, _| async move { api::space(&id).await },
        )
    };
    let fallback = id.clone();
    let title = move || {
        info.with(|s| s.map(api::Space::label))
            .unwrap_or_else(|| ui::short(&fallback))
    };
    let kind = signal(None::<&'static str>);
    let list_id = id.clone();
    nav_page()
        .title(title.clone())
        .header(ui::header(title))
        .child(
            view()
                .flex_1()
                .child(
                    view()
                        .flex_row()
                        .flex_wrap()
                        .items_center()
                        .justify_between()
                        .gap_2()
                        .px_4()
                        .py_2()
                        .border_b_1()
                        .border_b_color(move || theme().border)
                        .child(
                            tabs()
                                .flex_row()
                                .gap_1()
                                .child(type_tab(None, kind))
                                .children(TYPES.map(|t| type_tab(Some(t), kind))),
                        )
                        .child(ui::primary_button(move || t!("new_note"), {
                            let id = id.clone();
                            move || nav::push(Route::NewNote(id.clone()))
                        })),
                )
                // Keyed by the filter: a new list, its paging from the start,
                // for each.
                .child(each(
                    move || vec![kind.get()],
                    |k| *k,
                    move |k, _| {
                        let id = list_id.clone();
                        let k = k.get_untracked();
                        ui::paged_rows(
                            move |offset| {
                                let id = id.clone();
                                async move { api::resources(&id, k, ui::PAGE, offset).await }
                            },
                            false,
                        )
                    },
                )),
        )
}

fn type_tab(kind: Option<&'static str>, current: Signal<Option<&'static str>>) -> impl IntoElement {
    let selected = move || current.get() == kind;
    tab()
        .selected(selected)
        .on_press(move || current.set(kind))
        .rounded(move || theme().radius)
        .px_3()
        .py_1()
        .selected_style(|s| s.bg(move || theme().muted))
        .child(
            text(move || kind.map_or_else(|| t!("all"), str::to_owned))
                .text_sm()
                .text_color(move || {
                    if selected() {
                        theme().foreground
                    } else {
                        theme().muted_foreground
                    }
                }),
        )
}
