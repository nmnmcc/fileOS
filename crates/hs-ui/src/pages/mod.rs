//! One file per page of the stack, and per section of the home page.

mod home;
mod login;
mod note;
mod resource;
mod search;
mod settings;
mod space;
mod spaces;
mod timeline;

pub(crate) use home::home;
pub(crate) use login::login;
pub(crate) use note::new_note;
pub(crate) use resource::resource_page;
pub(crate) use search::search;
pub(crate) use settings::settings;
pub(crate) use space::space;
pub(crate) use spaces::spaces;
pub(crate) use timeline::timeline;
