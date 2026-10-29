use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

use bytesize::ByteSize as RawByteSize;
use serde::{Deserialize, Serialize};

use crate::{
    adapter::wire::{WireForm, form},
    domain::{
        color_scheme::ColorScheme,
        process::StopPolicy,
        pty::{PtySize, SpawnHistory, SpawnRequest},
        value::{ByteSize, CommandLine, ProcessName},
    },
};

/// Reported for a log cap of zero bytes, which no log can keep.
const ZERO_LOG_CAP: &str = "the log cap must be at least one byte";

/// A spawn request as a holder's spec file carries it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct SpawnRequestWire {
    /// Name of the process being spawned.
    process: ProcessName,
    /// Shell command to run.
    command: CommandLine,
    /// Directory to launch in, when not the parent's.
    working_dir: Option<PathBuf>,
    /// Project root exported to the process.
    project: Option<PathBuf>,
    /// The launching command's environment, as name and value pairs, since a
    /// name is an OS string JSON cannot use as a map key.
    launch_environment: Vec<(OsString, OsString)>,
    /// The process's own environment, as name and value pairs.
    environment: Vec<(OsString, OsString)>,
    /// How the process stops.
    #[serde(with = "form")]
    stop: StopPolicy,
    /// Initial PTY size.
    #[serde(with = "form")]
    size: PtySize,
    /// Lines of scrollback the terminal keeps.
    scrollback: usize,
    /// Size at which the output log rotates, as an exact byte count, since
    /// its own human-readable form rounds.
    log_max_size: u64,
    /// The default colors the terminal reports.
    #[serde(with = "form")]
    color_scheme: ColorScheme,
    /// What earlier runs leave to this one.
    #[serde(with = "form")]
    history: SpawnHistory,
}

impl WireForm for SpawnRequest {
    type Wire = SpawnRequestWire;

    fn to_wire(&self) -> SpawnRequestWire {
        SpawnRequestWire {
            process: self.process().clone(),
            command: self.command().clone(),
            working_dir: self.working_dir().clone(),
            project: self.project().clone(),
            launch_environment: pairs(self.launch_environment()),
            environment: pairs(self.environment()),
            stop: self.stop(),
            size: self.size(),
            scrollback: self.scrollback(),
            log_max_size: self.log_max_size().bytes(),
            color_scheme: self.color_scheme(),
            history: self.history().clone(),
        }
    }

    fn from_wire(wire: SpawnRequestWire) -> Result<Self, String> {
        let log_max_size = ByteSize::try_new(RawByteSize::b(wire.log_max_size))
            .map_err(|_| ZERO_LOG_CAP.to_string())?;
        Ok(Self::builder()
            .process(wire.process)
            .command(wire.command)
            .working_dir(wire.working_dir)
            .project(wire.project)
            .launch_environment(wire.launch_environment.into_iter().collect())
            .environment(wire.environment.into_iter().collect())
            .stop(wire.stop)
            .size(wire.size)
            .scrollback(wire.scrollback)
            .log_max_size(log_max_size)
            .color_scheme(wire.color_scheme)
            .history(wire.history)
            .build())
    }
}

/// `environment` as name and value pairs.
fn pairs(environment: &BTreeMap<OsString, OsString>) -> Vec<(OsString, OsString)> {
    environment
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect()
}
