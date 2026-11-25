//! The home page: the sections as tabs, and the one selected.

use nsh_base::{nav_page, tab, tabs, theme};
use nsh_core::prelude::*;

use crate::{
    pages::{search, settings, spaces, timeline},
    store::{self, SECTIONS, Section},
    ui,
};

pub(crate) fn home() -> Screen {
    nav_page()
        .title("homeserver")
        .header(ui::header(|| "homeserver".to_owned()))
        .child(
            view()
                .flex_1()
                .child(
                    tabs()
                        .flex_row()
                        .gap_1()
                        .px_3()
                        .py_2()
                        .border_b_1()
                        .border_b_color(move || theme().border)
                        .children(SECTIONS.map(section_tab)),
                )
                .child(when(|| store::section() == Section::Timeline, timeline))
                .child(when(|| store::section() == Section::Spaces, spaces))
                .child(when(|| store::section() == Section::Search, search))
                .child(when(|| store::section() == Section::Settings, settings)),
        )
}

fn section_tab(section: Section) -> impl IntoElement {
    let selected = move || store::section() == section;
    tab()
        .selected(selected)
        .on_press(move || store::show_section(section))
        .rounded(move || theme().radius)
        .px_3()
        .py_2()
        .selected_style(|s| s.bg(move || theme().muted))
        .child(text(move || t!(section.key())).text_color(move || {
            if selected() {
                theme().foreground
            } else {
                theme().muted_foreground
            }
        }))
}
