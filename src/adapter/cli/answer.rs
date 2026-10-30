use std::io;

use getset::Getters;
use serde::Serialize;
use serde_json::Value;
use typed_builder::TypedBuilder;

use super::{error::CliError, row::Row};

/// What a coordination command answers: the rows a person reads, and the
/// value `--json` prints in their place.
#[derive(Clone, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct Answer {
    /// The lines shown without `--json`.
    rows: Vec<Row>,
    /// The JSON shown with it.
    json: Value,
}

impl Answer {
    /// `rows` for a person, `view` for `--json`.
    ///
    /// # Errors
    /// Returns [`CliError::Output`] when `view` cannot be rendered as JSON.
    pub fn of(rows: Vec<Row>, view: &impl Serialize) -> Result<Self, CliError> {
        let json = serde_json::to_value(view)
            .map_err(|error| CliError::Output(io::Error::other(error)))?;
        Ok(Self { rows, json })
    }
}
