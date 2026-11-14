//! Adopters: holders found, gated, failing, reconnecting and replaying.

mod failing_worker_adopter;
mod fake_adopter;
mod gated_adopt_adopter;
mod gated_locate_adopter;
mod logged_adopter;
mod reconnecting_adopter;
mod recovering_adopter;
mod replay_ask_adopter;
pub(crate) mod replay_ask_handle;
mod stuck_log_adopter;

pub(crate) use failing_worker_adopter::*;
pub(crate) use fake_adopter::*;
pub(crate) use gated_adopt_adopter::*;
pub(crate) use gated_locate_adopter::*;
pub(crate) use logged_adopter::*;
pub(crate) use reconnecting_adopter::*;
pub(crate) use recovering_adopter::*;
pub(crate) use replay_ask_adopter::*;
pub(crate) use stuck_log_adopter::*;
