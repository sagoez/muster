use std::path::Path;

use super::{
    candidate::Candidate,
    detect_source::DetectSource,
    source_file::{candidate, read_first, shell_word, suggested},
};

/// The justfile names `just` looks for.
const JUSTFILES: &[&str] = &["justfile", "Justfile", ".justfile"];
/// Recipes suggested by name.
const SUGGESTED: &[&str] = &["dev", "serve", "watch", "start", "run"];
/// The program that runs a recipe.
const JUST: &str = "just";
/// Marks a recipe that does not echo its lines.
const QUIET_PREFIX: char = '@';
/// Ends a recipe's header.
const HEADER_END: char = ':';
/// What follows the colon of an assignment rather than a header.
const ASSIGNMENT: char = '=';

/// One candidate per recipe header at column 0 of the justfile.
pub fn detect(root: &Path) -> Vec<Candidate> {
    let Some(text) = read_first(root, JUSTFILES) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(recipe)
        .filter_map(|name| {
            candidate(
                name,
                &format!("{JUST} {}", shell_word(name)),
                DetectSource::Justfile,
                suggested(name, SUGGESTED, &[]),
            )
        })
        .collect()
}

/// The recipe a line declares: a name at column 0, optionally quiet, then
/// parameters and a colon that does not start an assignment.
fn recipe(line: &str) -> Option<&str> {
    let header = line.strip_prefix(QUIET_PREFIX).unwrap_or(line);
    let first = header.chars().next()?;
    if !(first.is_ascii_alphabetic() || first == '_') {
        return None;
    }
    let (before, after) = header.split_once(HEADER_END)?;
    if after.starts_with(ASSIGNMENT) {
        return None;
    }
    let name = before.split_whitespace().next()?;
    name.chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        .then_some(name)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// Recipes become `just <recipe>`; settings, assignments, aliases and
    /// indented lines do not.
    #[test]
    fn recipes_are_offered_and_the_rest_is_not() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("justfile"),
            "set shell := [\"bash\", \"-c\"]\nport := \"8080\"\nalias d := dev\n\n# serve it\ndev port=\"1\":\n    cargo run\n@build: dev\n    cargo build\n",
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
            ("dev".to_string(), "just dev".to_string(), true),
            ("build".to_string(), "just build".to_string(), false),
        ]);
    }
}
