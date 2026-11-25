//! The app shell: the navigation stack and its pages, no page logic.

use nsh_base::nav_stack;
use nsh_core::prelude::*;

use crate::{
    nav::{self, Route},
    pages,
};

pub(crate) fn shell() -> impl IntoElement {
    nav_stack().flex_1().child(pages::home()).child(each(
        nav::open,
        Route::clone,
        |route, depth| {
            let page = match route.get_untracked() {
                Route::Space(id) => pages::space(id),
                Route::Resource { space, target } => pages::resource_page(space, target),
                Route::NewNote(space) => pages::new_note(space),
            };
            // Back on iOS and Android is the system's: it pops the page
            // natively, and the route (with what is above it) follows.
            page.on_dismiss(move || nav::truncate(depth.get_untracked()))
        },
    ))
}
