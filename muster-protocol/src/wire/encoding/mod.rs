//! How a domain value travels: its wire form, its text form, and the serde
//! with-modules a field names.

pub mod form;
pub mod form_list;
pub mod optional_form;
#[cfg(test)]
mod text_carrier;
mod text_form;
mod wire_form;

pub use text_form::*;
pub use wire_form::*;
