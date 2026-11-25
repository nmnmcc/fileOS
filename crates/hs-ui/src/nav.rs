//! The navigation stack's state: the pages open over the home page, bottom
//! to top. Putting a route in pushes its page, taking it out pops it.

use nsh_core::prelude::*;

/// A page over the home page.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Route {
    /// A space's resources.
    Space(String),
    /// One resource: its space and `type/id`.
    Resource { space: String, target: String },
    /// A new note in a space.
    NewNote(String),
}

#[derive(Clone, Default)]
struct Routes(Vec<Route>);

fn routes() -> Signal<Routes> {
    global::<Routes>()
}

/// The open routes (tracked where it is read).
pub(crate) fn open() -> Vec<Route> {
    routes().with(|r| r.0.clone())
}

/// Whether there is a page to go back to (tracked where it is read).
pub(crate) fn can_go_back() -> bool {
    routes().with(|r| !r.0.is_empty())
}

/// Opens `route`; one already on top opens nothing more.
pub(crate) fn push(route: Route) {
    routes().update(|r| {
        if r.0.last() != Some(&route) {
            r.0.push(route);
        }
    });
}

/// Pops the top page.
pub(crate) fn pop() {
    routes().update(|r| {
        r.0.pop();
    });
}

/// Replaces the top page with `route` (a new note, once saved).
pub(crate) fn replace(route: Route) {
    routes().update(|r| {
        r.0.pop();
        r.0.push(route);
    });
}

/// Leaves `depth` pages over the home page: the user went back natively.
pub(crate) fn truncate(depth: usize) {
    routes().update(|r| r.0.truncate(depth));
}

/// Closes every page.
pub(crate) fn clear() {
    routes().update(|r| r.0.clear());
}
