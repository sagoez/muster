//! The floor: a pure model of every project's processes, which turns each new
//! reading into the few numbered messages the page needs to catch up.

mod located_log;
mod message;
mod model;
mod occupant;
mod reading;
mod tenant;
mod view;

#[cfg(test)]
pub(crate) mod fixture;

pub(crate) use located_log::*;
pub(crate) use message::*;
pub(crate) use model::*;
pub(crate) use reading::*;
