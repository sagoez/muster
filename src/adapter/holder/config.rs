use std::{
    fs::{self, OpenOptions},
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    time::Duration,
};

use getset::{CopyGetters, Getters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

use super::{error::HolderError, id::HolderId};
use crate::domain::{color_scheme::ColorScheme, pty::SpawnRequest, value::ByteSize};

/// Mode of a handed-over file: owner read and write only, since the request
/// can carry the child's environment.
const PRIVATE_FILE_MODE: u32 = 0o600;
/// Extension of the spec file handed to a holder beside its socket.
const SPEC_EXTENSION: &str = "json";
/// Extension of the file a holder that fails reports its error in, beside its
/// spec.
const ERROR_EXTENSION: &str = "err";

/// Everything one holder needs to run its child and serve clients.
#[derive(Debug, Clone, Serialize, Deserialize, Getters, CopyGetters, TypedBuilder)]
#[serde(rename_all = "kebab-case")]
pub struct HolderConfig {
    /// The holder's identity, also its record's file stem.
    #[getset(get = "pub")]
    id: HolderId,
    /// The child to spawn.
    #[getset(get = "pub")]
    request: SpawnRequest,
    /// Socket the holder serves clients on.
    #[getset(get = "pub")]
    socket: PathBuf,
    /// Directory the holder writes its record into.
    #[getset(get = "pub")]
    record_dir: PathBuf,
    /// The child's output log.
    #[getset(get = "pub")]
    log: PathBuf,
    /// Size at which the log rotates.
    #[getset(get_copy = "pub")]
    #[serde(with = "byte_count")]
    log_cap: ByteSize,
    /// Scrollback lines the holder's emulator keeps.
    #[getset(get_copy = "pub")]
    scrollback: usize,
    /// The default colors the holder's emulator reports when the child asks.
    #[getset(get_copy = "pub")]
    color_scheme: ColorScheme,
    /// How long after starting the holder waits for its first supervisor.
    /// One that never arrives means the launcher gave up, so the holder kills
    /// its child and exits rather than leave it running unsupervised.
    #[getset(get_copy = "pub")]
    #[serde(with = "humantime_serde")]
    supervisor_deadline: Duration,
}

impl HolderConfig {
    /// Where this config is handed to its holder: beside the private socket.
    pub fn spec_path(&self) -> PathBuf {
        self.socket.with_extension(SPEC_EXTENSION)
    }

    /// Where this config's holder reports an error it fails with.
    pub fn error_path(&self) -> PathBuf {
        error_path_beside(&self.spec_path())
    }

    /// Writes this config as JSON to a new owner-only file at `path`, for a
    /// holder process to read at start.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when encoding fails or the file exists or
    /// cannot be written.
    pub fn write_private(&self, path: &Path) -> Result<(), HolderError> {
        write_private_file(path, &serde_json::to_vec(self)?)
    }

    /// Reads a config written by [`Self::write_private`] and deletes the file,
    /// so the handed-over request does not linger on disk.
    ///
    /// # Errors
    /// Returns a [`HolderError`] when the file cannot be read or parsed.
    pub fn take_from(path: &Path) -> Result<Self, HolderError> {
        let bytes = fs::read(path);
        let _ = fs::remove_file(path);
        Ok(serde_json::from_slice(&bytes?)?)
    }
}

/// The error file of the holder handed the spec at `spec`.
pub fn error_path_beside(spec: &Path) -> PathBuf {
    spec.with_extension(ERROR_EXTENSION)
}

/// Whether `path` is a spec or error file handed over beside a holder socket.
pub fn is_handover_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension == SPEC_EXTENSION || extension == ERROR_EXTENSION)
}

/// Writes `bytes` to a new owner-only file at `path`, removing it again when
/// the write fails part way.
///
/// # Errors
/// Returns a [`HolderError`] when the file exists or cannot be written.
pub(super) fn write_private_file(path: &Path, bytes: &[u8]) -> Result<(), HolderError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(PRIVATE_FILE_MODE)
        .open(path)?;
    let written = file.write_all(bytes);
    if written.is_err() {
        let _ = fs::remove_file(path);
    }
    Ok(written?)
}

/// Carries a [`ByteSize`] as its exact byte count, since its own human-readable
/// form rounds.
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

#[cfg(test)]
mod tests {
    use bytesize::ByteSize as RawByteSize;
    use tempfile::TempDir;

    use super::*;
    use crate::domain::{
        process::ProcessKind,
        pty::PtySize,
        value::{Cols, CommandLine, ProcessName, Rows},
    };

    /// A log cap with no exact human-readable form.
    const ODD_CAP_BYTES: u64 = 1_234_567;
    /// Mode bits a permission check compares.
    const PERMISSION_BITS: u32 = 0o777;
    /// A first-supervisor deadline with a fractional part.
    const SUPERVISOR_DEADLINE: Duration = Duration::from_millis(1500);

    #[test]
    fn a_handed_over_config_round_trips_exactly_and_is_removed_after_reading() {
        use std::os::unix::fs::PermissionsExt;

        let scratch = TempDir::new().unwrap();
        let request = SpawnRequest::builder()
            .process(ProcessName::try_new("dev server").unwrap())
            .kind(ProcessKind::Command)
            .command(Some(CommandLine::try_new("echo hi").unwrap()))
            .environment([("KEY".into(), "value".into())].into())
            .size(
                PtySize::builder()
                    .rows(Rows::new(24))
                    .cols(Cols::new(80))
                    .build(),
            )
            .build();
        let config = HolderConfig::builder()
            .id(HolderId::generate().unwrap())
            .request(request)
            .socket(scratch.path().join("h.sock"))
            .record_dir(scratch.path().join("records"))
            .log(scratch.path().join("h.log"))
            .log_cap(ByteSize::try_new(RawByteSize::b(ODD_CAP_BYTES)).unwrap())
            .scrollback(10)
            .color_scheme(ColorScheme::Light)
            .supervisor_deadline(SUPERVISOR_DEADLINE)
            .build();
        let path = config.spec_path();

        config.write_private(&path).unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & PERMISSION_BITS;
        assert_eq!(mode, PRIVATE_FILE_MODE);
        let read = HolderConfig::take_from(&path).unwrap();

        assert!(!path.exists());
        assert_eq!(read.supervisor_deadline(), SUPERVISOR_DEADLINE);
        assert_eq!(read.log_cap().bytes(), ODD_CAP_BYTES);
        assert_eq!(read.color_scheme(), ColorScheme::Light);
        assert_eq!(read.id(), config.id());
        assert_eq!(read.request().environment(), config.request().environment());
        assert_eq!(read.request().size(), config.request().size());
    }
}
