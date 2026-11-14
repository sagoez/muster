//! The floor: a pure model of every project's processes, which turns each new
//! reading into the few numbered messages the page needs to catch up.

mod home_path;
mod located_log;
mod message;
mod model;
mod occupant;
mod reading;
mod tenant;
mod view;

#[cfg(test)]
pub(crate) mod fixture;

pub use located_log::*;
pub use message::*;
pub use model::*;
pub use reading::*;
pub use view::*;
