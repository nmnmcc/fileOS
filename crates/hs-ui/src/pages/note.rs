//! A note's editor, for a new note and an existing one. A save sends the
//! `ETag` it started from, so it never overwrites someone else's change.

use nsh_base::{nav_page, theme};
use nsh_core::prelude::*;
use serde_json::{Map, Value};

use crate::{api, nav, pages::resource, store, ui};

/// A new note in `space`.
pub(crate) fn new_note(space: String) -> Screen {
    let target = format!("note/{}", api::new_id());
    nav_page()
        .title(t!("new_note"))
        .header(ui::header(move || t!("new_note")))
        .child(
            scroll()
                .flex_1()
                .child(view().p_4().pb_safe_area_plus(px(16.0)).child(editor(
                    space,
                    target,
                    &Value::Null,
                    None,
                ))),
        )
}

/// The fields of a note, saved over `etag` (none: a new note).
pub(crate) fn editor(
    space: String,
    target: String,
    note: &Value,
    etag: Option<String>,
) -> impl IntoElement + use<> {
    let field = |k: &str| {
        note.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let is_new = etag.is_none();
    let title = signal(field("title"));
    let body = signal(field("text"));
    // The note's other fields, kept as they are.
    let rest = note.as_object().cloned().unwrap_or_default();
    let etag = signal(etag);
    let status = signal(String::new());
    let failed = signal(false);
    let busy = signal(false);
    let save = move || {
        if busy.get_untracked() {
            return;
        }
        let mut json: Map<String, Value> = rest.clone();
        json.insert("title".into(), title.get_untracked().trim().into());
        json.insert("text".into(), body.get_untracked().into());
        let (space, target) = (space.clone(), target.clone());
        busy.set(true);
        status.set(t!("saving"));
        failed.set(false);
        spawn(async move {
            let tag = etag.get_untracked();
            match api::put_json(&space, &target, &Value::Object(json), tag.as_deref()).await {
                Ok(_) if is_new => {
                    // The note's own page takes over; this one, and its
                    // signals, go away.
                    store::changed();
                    nav::replace(resource::route(space, target));
                    return;
                }
                Ok(new) => {
                    etag.set(new);
                    status.set(t!("saved"));
                    store::changed();
                }
                Err(e) => {
                    failed.set(true);
                    status.set(if e.status == 412 {
                        t!("conflict")
                    } else {
                        t!("error", msg = e)
                    });
                }
            }
            busy.set(false);
        });
    };
    view()
        .gap_3()
        .child(ui::field(title, "title").auto_focus(is_new))
        .child(ui::field(body, "text").multiline(true).min_h(px(240.0)))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_3()
                .child(ui::primary_button(move || t!("save"), save).disabled(move || busy.get()))
                .child(text(move || status.get()).text_sm().text_color(move || {
                    if failed.get() {
                        theme().destructive
                    } else {
                        theme().muted_foreground
                    }
                })),
        )
}
