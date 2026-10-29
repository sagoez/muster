use std::{collections::BTreeMap, ffi::OsString, path::PathBuf};

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use crate::domain::{
    color_scheme::ColorScheme,
    process::StopPolicy,
    pty::{PtySize, SpawnHistory},
    value::{ByteSize, CommandLine, ProcessName},
};

/// A request to spawn one process under a PTY.
#[derive(Clone, Debug, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder)]
#[serde(rename_all = "kebab-case")]
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
    #[serde(with = "environment_pairs")]
    launch_environment: BTreeMap<OsString, OsString>,
    /// The process's own environment, over the launch environment.
    #[getset(get = "pub")]
    #[builder(default)]
    #[serde(with = "environment_pairs")]
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
    #[serde(with = "byte_count")]
    log_max_size: ByteSize,
    /// The default colors the process's terminal reports when asked.
    #[getset(get_copy = "pub")]
    color_scheme: ColorScheme,
    /// What the process's earlier runs leave to this one.
    #[getset(get = "pub")]
    #[builder(default)]
    history: SpawnHistory,
}

/// Carries a [`ByteSize`] as its exact byte count, since its own
/// human-readable form rounds.
mod byte_count {
    use bytesize::ByteSize as RawByteSize;
    use serde::{Deserialize, Deserializer, Serializer, de::Error};

    use crate::domain::value::ByteSize;

    /// Writes `size` as a plain byte count.
    ///
    /// # Errors
    /// Returns the serializer's error.
    pub(super) fn serialize<S: Serializer>(
        size: &ByteSize,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(size.bytes())
    }

    /// Reads a plain byte count, refusing zero.
    ///
    /// # Errors
    /// Returns the deserializer's error, or a custom one for zero.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<ByteSize, D::Error> {
        ByteSize::try_new(RawByteSize::b(u64::deserialize(deserializer)?)).map_err(D::Error::custom)
    }
}

/// Carries the environment as a list of name and value pairs, since a name is
/// an OS string that formats such as JSON cannot use as a map key.
mod environment_pairs {
    use std::{collections::BTreeMap, ffi::OsString};

    use serde::{Deserialize, Deserializer, Serializer};

    /// Writes `environment` as a sequence of pairs.
    ///
    /// # Errors
    /// Returns the serializer's error.
    pub(super) fn serialize<S: Serializer>(
        environment: &BTreeMap<OsString, OsString>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        serializer.collect_seq(environment)
    }

    /// Reads a sequence of pairs back into the environment.
    ///
    /// # Errors
    /// Returns the deserializer's error.
    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<BTreeMap<OsString, OsString>, D::Error> {
        Ok(Vec::<(OsString, OsString)>::deserialize(deserializer)?
            .into_iter()
            .collect())
    }
}
