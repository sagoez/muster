use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};

use getset::Getters;

/// Lines of the service log shown when the service cannot be started.
const TAIL_LINES: usize = 20;
/// Most bytes read from the end of the service log for its tail, so a huge
/// log is never read whole.
const TAIL_BYTES: u64 = 16 * 1024;

/// The log the service writes its failures to, read from the client side.
#[derive(Debug, Clone, Getters)]
pub struct ServiceLog {
    /// Where it is.
    #[getset(get = "pub")]
    path: PathBuf,
}

impl ServiceLog {
    /// The log at `path`.
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// Where the log ends now, which marks a start attempt: everything
    /// written after it came from that attempt. An absent log ends at 0.
    pub fn end(&self) -> u64 {
        fs::metadata(&self.path).map_or(0, |metadata| metadata.len())
    }

    /// The last lines written after `from`, reading at most a bounded tail
    /// of the file. Empty when the log cannot be read.
    pub fn tail_since(&self, from: u64) -> Vec<String> {
        let Ok(mut file) = File::open(&self.path) else {
            return Vec::new();
        };
        let len = file.metadata().map_or(0, |metadata| metadata.len());
        let start = from.max(len.saturating_sub(TAIL_BYTES));
        let mut bytes = Vec::new();
        if file.seek(SeekFrom::Start(start)).is_err() || file.read_to_end(&mut bytes).is_err() {
            return Vec::new();
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut lines: Vec<&str> = text.lines().collect();
        if start > from && !lines.is_empty() {
            lines.remove(0);
        }
        lines[lines.len().saturating_sub(TAIL_LINES)..]
            .iter()
            .map(|line| (*line).to_string())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn the_tail_holds_only_this_attempt_and_its_last_lines() {
        let scratch = TempDir::new().unwrap();
        let log = ServiceLog::new(scratch.path().join("service.log"));
        assert_eq!(log.end(), 0);
        fs::write(log.path(), "an older failure\n").unwrap();
        let attempt = log.end();
        let lines: Vec<String> = (0..TAIL_LINES + 5).map(|n| format!("line {n}")).collect();
        fs::write(
            log.path(),
            format!("an older failure\n{}\n", lines.join("\n")),
        )
        .unwrap();

        let tail = log.tail_since(attempt);

        assert_eq!(tail.len(), TAIL_LINES);
        assert_eq!(tail.last().unwrap(), &format!("line {}", TAIL_LINES + 4));
        assert!(!tail.iter().any(|line| line.contains("older")));
        assert!(
            ServiceLog::new(scratch.path().join("absent.log"))
                .tail_since(0)
                .is_empty()
        );
    }

    #[test]
    fn a_huge_log_is_read_only_at_its_end() {
        let scratch = TempDir::new().unwrap();
        let log = ServiceLog::new(scratch.path().join("service.log"));
        let filler = "x".repeat(usize::try_from(TAIL_BYTES).unwrap() * 4);
        fs::write(log.path(), format!("{filler}\nthe reason\n")).unwrap();

        assert_eq!(log.tail_since(0), vec!["the reason".to_string()]);
    }
}
