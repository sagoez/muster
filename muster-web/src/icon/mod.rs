//! The marks `muster view` draws: a process's icon, a project's, and an
//! author's.

mod agent_mark;
mod brand_asset;
mod command_icon;
mod manifest_icon;
mod project_icon;
mod resolution;

#[cfg(test)]
mod contrast;
#[cfg(test)]
mod fake_manifests;
#[cfg(test)]
mod spec_fixture;

pub use agent_mark::*;
pub use brand_asset::*;
pub use project_icon::*;
pub use resolution::*;
