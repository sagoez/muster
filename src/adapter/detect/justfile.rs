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
/// Opens a line of attributes for the item after it.
const ATTRIBUTES_OPEN: char = '[';
/// Closes a line of attributes.
const ATTRIBUTES_CLOSE: char = ']';
/// Separates the attributes of one line.
const ATTRIBUTE_SEPARATOR: char = ',';
/// Where an attribute's arguments start.
const ATTRIBUTE_ARGUMENTS: [char; 2] = ['(', ':'];
/// The attribute that hides a recipe from `just --list`.
const PRIVATE_ATTRIBUTE: &str = "private";
/// Starts the name of a recipe that is private by convention.
const PRIVATE_PREFIX: char = '_';
/// Opens a comment line.
const COMMENT: char = '#';

/// One candidate per public recipe header at column 0 of the justfile: none
/// marked `[private]` or named with a leading `_`.
pub fn detect(root: &Path) -> Vec<Candidate> {
    let Some(text) = read_first(root, JUSTFILES) else {
        return Vec::new();
    };
    let mut private = false;
    let mut recipes = Vec::new();
    for line in text.lines() {
        if let Some(attributes) = attributes(line) {
            private |= attributes.contains(&PRIVATE_ATTRIBUTE);
            continue;
        }
        if line.trim().is_empty() || line.starts_with(COMMENT) {
            continue;
        }
        if let Some(name) = recipe(line)
            && !private
            && !name.starts_with(PRIVATE_PREFIX)
        {
            recipes.push(name);
        }
        private = false;
    }
    recipes
        .into_iter()
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

/// The names of the attributes a `[name, name(argument)]` line gives the
/// item after it, or none for any other line.
fn attributes(line: &str) -> Option<Vec<&str>> {
    let inside = line
        .strip_prefix(ATTRIBUTES_OPEN)?
        .trim_end()
        .strip_suffix(ATTRIBUTES_CLOSE)?;
    Some(
        inside
            .split(ATTRIBUTE_SEPARATOR)
            .map(|attribute| {
                attribute
                    .split(ATTRIBUTE_ARGUMENTS)
                    .next()
                    .unwrap_or_default()
                    .trim()
            })
            .collect(),
    )
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

    /// A recipe marked `[private]`, alone or among other attributes and
    /// past a comment, or named with a leading `_`, is not offered; an
    /// attribute only marks the item right after it.
    #[test]
    fn private_recipes_are_not_offered() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("justfile"),
            "[private]\nsetup:\n    ./setup\n[no-cd, private]\n# helper\nclean:\n    rm -rf out\n_check:\n    true\n[group('dev')]\ndev: _check\n    cargo run\nserve:\n    ./serve\n",
        )
        .unwrap();

        let found: Vec<String> = detect(dir.path())
            .iter()
            .map(|candidate| candidate.name().to_string())
            .collect();
        assert_eq!(found, vec!["dev".to_string(), "serve".to_string()]);
    }
}
