use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

use getset::{CopyGetters, Getters};
use typed_builder::TypedBuilder;

use crate::domain::{
    color_scheme::ColorScheme,
    process::StopPolicy,
    pty::{PtySize, SpawnHistory},
    value::{ByteSize, CommandLine, ProcessName},
};

/// A request to spawn one process under a PTY.
#[derive(Clone, Debug, Getters, CopyGetters, TypedBuilder)]
pub struct SpawnRequest {
    /// Name of the process being spawned, as its project knows it.
    #[getset(get = "pub")]
    process: ProcessName,
    /// Shell command to run.
    #[getset(get = "pub")]
    command: CommandLine,
    /// Directory to launch in; inherits the parent's cwd when absent.
    #[getset(get = "pub")]
    #[builder(default)]
    working_dir: Option<PathBuf>,
    /// Project root exported to the process, letting the `muster` CLI target
    /// the current project without a flag. Absent leaves it unset.
    #[getset(get = "pub")]
    #[builder(default)]
    project: Option<PathBuf>,
    /// The environment of the command that launched the process, under its
    /// own; kept so its restarts and an adopting service launch it the same.
    #[getset(get = "pub")]
    #[builder(default)]
    launch_environment: BTreeMap<OsString, OsString>,
    /// The process's own environment, over the launch environment.
    #[getset(get = "pub")]
    #[builder(default)]
    environment: BTreeMap<OsString, OsString>,
    /// How the process stops, kept by its holder so a service that adopts it
    /// after its removal still stops it as it was defined.
    #[getset(get_copy = "pub")]
    stop: StopPolicy,
    /// Initial PTY size.
    #[getset(get_copy = "pub")]
    size: PtySize,
    /// Lines of scrollback the process's terminal keeps.
    #[getset(get_copy = "pub")]
    scrollback: usize,
    /// Size at which the process's output log rotates.
    #[getset(get_copy = "pub")]
    log_max_size: ByteSize,
    /// The default colors the process's terminal reports when asked.
    #[getset(get_copy = "pub")]
    color_scheme: ColorScheme,
    /// What the process's earlier runs leave to this one.
    #[getset(get = "pub")]
    #[builder(default)]
    history: SpawnHistory,
}
