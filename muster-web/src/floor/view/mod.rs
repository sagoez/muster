//! What the page draws: each project's floor, its processes' faces and rooms,
//! the command each card suggests, and the notes.

mod ended_view;
mod face_view;
mod floor_view;
mod mark;
mod next_step;
mod next_verb;
mod note_view;
mod placement;
mod room;

pub(crate) use ended_view::*;
pub(crate) use face_view::*;
pub(crate) use floor_view::*;
pub(crate) use next_step::*;
pub(crate) use note_view::*;
pub(crate) use placement::*;
pub(crate) use room::*;
