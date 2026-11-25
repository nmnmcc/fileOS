//! The app's state: whether someone is signed in, the section the home page
//! shows, and a revision that every change bumps so lists load again. The
//! signals stay private; pages use these functions. A global is per surface,
//! so each test has its own.

use nsh_core::prelude::*;

/// Whether someone is signed in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Session {
    /// Not known yet: the first check runs.
    #[default]
    Checking,
    SignedOut,
    SignedIn,
}

fn session_signal() -> Signal<Session> {
    global::<Session>()
}

/// The session (tracked where it is read).
pub(crate) fn session() -> Session {
    session_signal().get()
}

pub(crate) fn signed_in() {
    session_signal().set(Session::SignedIn);
}

/// The session is gone: back to the sign-in page, with nothing open.
pub(crate) fn signed_out() {
    crate::nav::clear();
    session_signal().set(Session::SignedOut);
}

/// The home page's sections.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Section {
    #[default]
    Timeline,
    Spaces,
    Search,
    Settings,
}

pub(crate) const SECTIONS: [Section; 4] = [
    Section::Timeline,
    Section::Spaces,
    Section::Search,
    Section::Settings,
];

impl Section {
    /// Its translation key.
    pub(crate) fn key(self) -> &'static str {
        match self {
            Section::Timeline => "timeline",
            Section::Spaces => "spaces",
            Section::Search => "search",
            Section::Settings => "settings",
        }
    }
}

fn section_signal() -> Signal<Section> {
    global::<Section>()
}

pub(crate) fn section() -> Section {
    section_signal().get()
}

pub(crate) fn show_section(section: Section) {
    section_signal().set(section);
}

#[derive(Clone, Copy, Default)]
struct Revision(u64);

/// The data's revision (tracked where it is read): a list that reads it
/// loads again after a change.
pub(crate) fn revision() -> u64 {
    global::<Revision>().get().0
}

/// Something changed on the server.
pub(crate) fn changed() {
    global::<Revision>().update(|r| r.0 += 1);
}
