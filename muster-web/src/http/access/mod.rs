//! Who may reach the view: the gate, the token it checks and the one-time
//! code the browser is opened with.

mod admission;
pub(crate) mod gate;
mod one_time_code;
mod open_code;
pub(crate) mod token;

pub use admission::*;
pub use gate::*;
pub use token::*;
