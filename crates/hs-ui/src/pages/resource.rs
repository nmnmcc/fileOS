//! One resource: a note opens in the editor, anything else shows as text.
//! Deleting asks first; the history is kept and the server can restore it.

use nsh_base::{alert_dialog, nav_page, theme};
use nsh_core::prelude::*;
use serde_json::Value;

use crate::{
    api,
    nav::{self, Route},
    pages::note,
    store, ui,
};

pub(crate) fn resource_page(space: String, target: String) -> Screen {
    let loaded = {
        let (space, target) = (space.clone(), target.clone());
        resource(
            move || Some((space.clone(), target.clone())),
            |(space, target): (String, String), _| async move { api::resource(&space, &target).await },
        )
    };
    let fallback = target.clone();
    let title = move || {
        loaded
            .with(|r| r.and_then(title_of))
            .unwrap_or_else(|| fallback.clone())
    };
    let is_note = target.starts_with("note/");
    let (s, tg) = (space.clone(), target.clone());
    nav_page()
        .title(title.clone())
        .header(ui::header(title))
        .child(
            scroll().flex_1().child(
                view()
                    .gap_4()
                    .p_4()
                    .pb_safe_area_plus(px(16.0))
                    .child(when(
                        move || loaded.loading() && loaded.with(|r| r.is_none()),
                        || ui::muted(move || t!("loading")),
                    ))
                    .child(when(
                        move || loaded.error().is_some(),
                        move || {
                            ui::error_text(move || {
                                let e = loaded.error().map(|e| e.to_string()).unwrap_or_default();
                                t!("error", msg = e)
                            })
                        },
                    ))
                    // Made once the resource is here: the editor starts from it.
                    .child(each(
                        move || {
                            loaded
                                .get()
                                .filter(|_| is_note)
                                .and_then(|r| Some((r.json()?, r.etag)))
                                .into_iter()
                                .collect::<Vec<_>>()
                        },
                        // A new editor only for another version than the one saved.
                        |_| (),
                        move |note, _| {
                            let (json, etag) = note.get_untracked();
                            note::editor(s.clone(), tg.clone(), &json, etag)
                        },
                    ))
                    .child(when(
                        move || !is_note && loaded.with(|r| r.is_some()),
                        move || {
                            text(move || loaded.with(|r| r.map(shown).unwrap_or_default()))
                                .text_sm()
                                .font_family("monospace")
                                .text_color(move || theme().foreground)
                        },
                    ))
                    .child(
                        ui::danger_button(
                            move || t!("delete"),
                            move || {
                                confirm_delete(space.clone(), target.clone());
                            },
                        )
                        .self_start(),
                    ),
            ),
        )
}

/// A title from the resource's JSON.
fn title_of(r: &api::Resource) -> Option<String> {
    let json = r.json()?;
    ["title", "name"]
        .iter()
        .find_map(|k| json.get(*k).and_then(Value::as_str))
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// What a resource that is not a note shows: its JSON laid out, its text,
/// or what kind of file it is.
fn shown(r: &api::Resource) -> String {
    if let Some(json) = r.json() {
        return serde_json::to_string_pretty(&json).unwrap_or_default();
    }
    if r.media_type.starts_with("text/") {
        return r.text.clone();
    }
    r.media_type.clone()
}

/// Asks first; the answer arrives in a task. The page pops before the
/// lists load again, so it leaves showing what it showed.
fn confirm_delete(space: String, target: String) {
    let answer = alert_dialog()
        .title(t!("confirm_delete"))
        .cancel("cancel", t!("cancel"))
        .destructive("delete", t!("delete"))
        .show();
    spawn(async move {
        let yes = answer
            .await
            .is_ok_and(|a| a.action.as_deref() == Some("delete"));
        if !yes {
            return;
        }
        match api::delete(&space, &target).await {
            Ok(()) => {
                nav::pop();
                store::changed();
            }
            Err(e) => {
                let _ = alert_dialog()
                    .title(t!("error", msg = e))
                    .cancel("ok", "OK")
                    .show()
                    .await;
            }
        }
    });
}

/// The route of a resource just made.
pub(crate) fn route(space: String, target: String) -> Route {
    Route::Resource { space, target }
}
