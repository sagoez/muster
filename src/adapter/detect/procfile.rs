use std::path::Path;

use super::{
    candidate::Candidate,
    detect_source::DetectSource,
    source_file::{candidate, read_each},
};

/// The Procfiles read, in order: the development one first, so its
/// definition of a name wins.
const PROCFILES: &[&str] = &["Procfile.dev", "Procfile"];
/// Opens a comment line.
const COMMENT: char = '#';
/// Separates a process's name from its command.
const NAME_SEPARATOR: char = ':';

/// One suggested candidate per `name: command` line of `Procfile.dev` and
/// `Procfile`.
pub fn detect(root: &Path) -> Vec<Candidate> {
    read_each(root, PROCFILES)
        .iter()
        .flat_map(|text| text.lines())
        .filter_map(|line| {
            let line = line.trim();
            if line.starts_with(COMMENT) {
                return None;
            }
            let (name, command) = line.split_once(NAME_SEPARATOR)?;
            candidate(name.trim(), command.trim(), DetectSource::Procfile, true)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// Every process line is offered and suggested; comments and blank
    /// lines are not.
    #[test]
    fn each_process_line_is_offered() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Procfile"),
            "# web tier\nweb: bundle exec puma -p $PORT\n\nworker:  bin/worker\n",
        )
        .unwrap();

        let found: Vec<(String, String, bool)> = detect(dir.path())
            .iter()
            .map(|candidate| {
                (
                    candidate.name().to_string(),
                    candidate.command().to_string(),
                    candidate.suggested(),
                )
            })
            .collect();
        assert_eq!(found, vec![
            (
                "web".to_string(),
                "bundle exec puma -p $PORT".to_string(),
                true
            ),
            ("worker".to_string(), "bin/worker".to_string(), true),
        ]);
    }
}
