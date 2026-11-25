//! Everything readable, newest first.

use nsh_core::prelude::*;

use crate::{api, ui};

pub(crate) fn timeline() -> impl IntoElement {
    ui::paged_rows(|offset| api::timeline(ui::PAGE, offset), true)
}
