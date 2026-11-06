use serde::Serialize;
use strum::{Display, EnumIter};

/// Where `muster init` found a candidate process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Display, EnumIter, Serialize)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum DetectSource {
    /// A `scripts` entry of `package.json`.
    PackageJson,
    /// A line of `Procfile` or `Procfile.dev`.
    Procfile,
    /// A recipe of a `justfile`.
    Justfile,
    /// A target of a `Makefile`.
    Makefile,
    /// A Docker Compose file.
    Compose,
}
