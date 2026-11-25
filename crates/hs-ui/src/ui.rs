//! What the pages share: the header, buttons, fields and list rows.

use nsh_base::{button, input, theme};
use nsh_core::prelude::*;

use crate::{
    api::{self, Row},
    nav::{self, Route},
    store,
};

/// A page's header on the desktops and the web; iOS and Android show the
/// system's, from the page's `title`.
pub(crate) fn header(title: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .gap_3()
        .px_4()
        .py_3()
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(when(nav::can_go_back, || {
            button()
                .active(|s| s.opacity(0.6))
                .on_press(nav::pop)
                .child(text(move || t!("back")).text_color(move || theme().primary))
        }))
        .child(
            text(title)
                .flex_1()
                .number_of_lines(1)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
}

/// The main action of a form.
pub(crate) fn primary_button(
    label: impl Fn() -> String + 'static,
    on_press: impl Fn() + 'static,
) -> nsh_base::ButtonElement {
    button()
        .on_press(on_press)
        .px_4()
        .py_2()
        .rounded(move || theme().radius)
        .bg(move || theme().primary)
        .active(|s| s.opacity(0.6))
        .disabled_style(|s| s.opacity(0.4))
        .child(text(label).text_color(move || theme().primary_foreground))
}

/// Another action.
pub(crate) fn outline_button(
    label: impl Fn() -> String + 'static,
    on_press: impl Fn() + 'static,
) -> nsh_base::ButtonElement {
    button()
        .on_press(on_press)
        .px_4()
        .py_2()
        .border_1()
        .border_color(move || theme().border)
        .rounded(move || theme().radius)
        .active(|s| s.opacity(0.6))
        .disabled_style(|s| s.opacity(0.4))
        .child(text(label).text_color(move || theme().foreground))
}

/// An action that removes something.
pub(crate) fn danger_button(
    label: impl Fn() -> String + 'static,
    on_press: impl Fn() + 'static,
) -> nsh_base::ButtonElement {
    button()
        .on_press(on_press)
        .px_4()
        .py_2()
        .border_1()
        .border_color(move || theme().destructive)
        .rounded(move || theme().radius)
        .active(|s| s.opacity(0.6))
        .disabled_style(|s| s.opacity(0.4))
        .child(text(label).text_color(move || theme().destructive))
}

/// A one-line text field bound to `value`, its placeholder and the name
/// screen readers say the translation of `key`.
pub(crate) fn field(value: Signal<String>, key: &'static str) -> Input {
    input()
        .value(move || value.get())
        .on_change(move |v: &str| value.set(v.to_owned()))
        .placeholder(move || t!(key))
        .label(move || t!(key))
        .px_3()
        .py_2()
        .border_1()
        .border_color(move || theme().input)
        .rounded(move || theme().radius)
        .text_color(move || theme().foreground)
}

/// Text in the muted color: hints, metadata, empty lists.
pub(crate) fn muted(s: impl Fn() -> String + 'static) -> impl IntoElement {
    text(s)
        .text_sm()
        .text_color(move || theme().muted_foreground)
}

/// An error, said to screen readers as it appears.
pub(crate) fn error_text(s: impl Fn() -> String + 'static) -> impl IntoElement {
    text(s).text_sm().text_color(move || theme().destructive)
}

/// A row of the index: its title, what it is and when, and the beginning
/// of its text. Pressing it opens the resource.
pub(crate) fn row_item(row: ReadSignal<Row>, space_name: bool) -> impl IntoElement {
    button()
        .gap_1()
        .px_4()
        .py_3()
        .border_b_1()
        .border_b_color(move || theme().border)
        .active(|s| s.bg(move || theme().muted))
        .on_press(move || {
            let row = row.get_untracked();
            nav::push(Route::Resource {
                space: row.space,
                target: row.target,
            });
        })
        .child(
            text(move || row.with(Row::label))
                .number_of_lines(1)
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().foreground),
        )
        .child(muted(move || {
            row.with(|r| {
                let when = format_time(if r.time == 0 { r.updated } else { r.time });
                if space_name {
                    format!("{} · {} · {when}", r.kind, short(&r.space))
                } else {
                    format!("{} · {when}", r.kind)
                }
            })
        }))
        .child(when(
            move || row.with(|r| r.snippet().is_some()),
            move || {
                text(move || row.with(Row::snippet).unwrap_or_default())
                    .number_of_lines(2)
                    .text_sm()
                    .text_color(move || theme().foreground)
            },
        ))
}

/// How many rows a list loads at first, and how many more each time.
pub(crate) const PAGE: usize = 50;

/// A list that appends pages and refreshes after local mutations.
pub(crate) fn paged_rows<F, Fut>(load: F, space_name: bool) -> impl IntoElement
where
    F: Fn(u32) -> Fut + Clone + 'static,
    Fut: Future<Output = api::Result<api::Page>> + 'static,
{
    let pages = signal(None::<(u64, api::Page)>);
    let pending = signal(None::<u64>);
    let more_error = signal(None::<(u64, api::Error)>);
    let first = resource(|| Some(store::revision()), {
        let load = load.clone();
        move |revision, _| {
            let request = load(0);
            async move {
                let page = request.await?;
                if store::revision() == revision {
                    pages.set(Some((revision, page)));
                }
                Ok::<(), api::Error>(())
            }
        }
    });
    let current = move || {
        let revision = store::revision();
        pages.with(|pages| {
            pages
                .as_ref()
                .filter(|(epoch, _)| *epoch == revision)
                .map(|(_, page)| page.clone())
        })
    };
    let more = move || {
        let revision = store::revision();
        pages.with(|page| {
            page.as_ref()
                .filter(|(epoch, _)| *epoch == revision)
                .and_then(|(_, page)| page.next_offset)
        })
    };
    let count = move || {
        let revision = store::revision();
        pages.with(|page| {
            page.as_ref()
                .filter(|(epoch, _)| *epoch == revision)
                .map_or(0, |(_, page)| page.items.len())
        })
    };
    let loading = move || first.loading() || pending.get() == Some(store::revision());
    let error = move || {
        let revision = store::revision();
        more_error
            .with(|error| {
                error
                    .as_ref()
                    .filter(|(epoch, _)| *epoch == revision)
                    .map(|(_, error)| error.to_string())
            })
            .or_else(|| first.error().map(|error| error.to_string()))
    };
    let load_more = move || {
        let revision = store::revision();
        let Some(offset) = more().filter(|_| !loading()) else {
            return;
        };
        pending.set(Some(revision));
        more_error.set(None);
        let request = load(offset);
        spawn(async move {
            let result = request.await;
            if pending.get_untracked() == Some(revision) {
                pending.set(None);
            }
            if store::revision() != revision {
                return;
            }
            match result {
                Ok(page) => pages.update(|pages| {
                    if let Some((_, loaded)) = pages {
                        let mut keys = loaded
                            .items
                            .iter()
                            .map(Row::key)
                            .collect::<std::collections::HashSet<_>>();
                        loaded.items.extend(
                            page.items
                                .into_iter()
                                .filter(|item| keys.insert(item.key())),
                        );
                        loaded.next_offset = page.next_offset;
                    }
                }),
                Err(error) => more_error.set(Some((revision, error))),
            }
        });
    };
    scroll().flex_1().child(
        view()
            .pb_safe_area_plus(px(16.0))
            .child(each(
                move || current().map_or_else(Vec::new, |page| page.items),
                Row::key,
                move |row, _| row_item(row, space_name),
            ))
            .child(when(
                move || !loading() && error().is_none() && count() == 0 && more().is_none(),
                || view().p_4().child(muted(move || t!("nothing_yet"))),
            ))
            .child(when(
                move || loading() && count() == 0,
                || view().p_4().child(muted(move || t!("loading"))),
            ))
            .child(when(
                move || error().is_some(),
                move || {
                    view().p_4().child(error_text(move || {
                        let e = error().unwrap_or_default();
                        t!("error", msg = e)
                    }))
                },
            ))
            .child(when(
                move || more().is_some(),
                move || {
                    view().p_4().child(
                        outline_button(move || t!("load_more"), load_more.clone())
                            .self_start()
                            .disabled(loading),
                    )
                },
            )),
    )
}

/// The first characters of a long id.
pub(crate) fn short(id: &str) -> String {
    if id.len() > 12 {
        format!("{}…", &id[..12])
    } else {
        id.to_owned()
    }
}

/// Milliseconds since the epoch as `YYYY-MM-DD HH:MM` (UTC).
pub(crate) fn format_time(ms: u64) -> String {
    if ms == 0 {
        return String::new();
    }
    let secs = ms / 1000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60
    )
}

/// The proleptic Gregorian date of a day count since 1970-01-01 (Howard
/// Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_times() {
        assert_eq!(format_time(0), "");
        assert_eq!(format_time(86_400_000), "1970-01-02 00:00");
        assert_eq!(format_time(1_767_258_000_000), "2026-01-01 09:00");
        assert_eq!(format_time(951_782_400_000), "2000-02-29 00:00");
    }
}
