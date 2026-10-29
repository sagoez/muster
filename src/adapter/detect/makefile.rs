use std::path::Path;

use super::{
    candidate::Candidate,
    detect_source::DetectSource,
    source_file::{candidate, read_first, suggested},
};

/// The makefile names `make` looks for.
const MAKEFILES: &[&str] = &["GNUmakefile", "makefile", "Makefile"];
/// Targets suggested by name.
const SUGGESTED: &[&str] = &["dev", "serve", "watch", "start", "run"];
/// The program that makes a target.
const MAKE: &str = "make";
/// Ends a rule's targets.
const RULE_END: char = ':';
/// What follows the colon of an assignment (`:=`) or a double-colon rule.
const NOT_A_PLAIN_RULE: [char; 2] = ['=', ':'];
/// Marks a special target such as `.PHONY`.
const SPECIAL_PREFIX: char = '.';

/// One candidate per plain target at column 0 of the makefile: no special
/// `.`-target, no pattern, no assignment.
pub fn detect(root: &Path) -> Vec<Candidate> {
    let Some(text) = read_first(root, MAKEFILES) else {
        return Vec::new();
    };
    text.lines()
        .flat_map(targets)
        .filter_map(|target| {
            candidate(
                target,
                &format!("{MAKE} {target}"),
                DetectSource::Makefile,
                suggested(target, SUGGESTED, &[]),
            )
        })
        .collect()
}

/// The plain targets a rule line declares.
fn targets(line: &str) -> Vec<&str> {
    let starts_a_rule = line
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphanumeric() || first == '_');
    let Some((before, after)) = line.split_once(RULE_END).filter(|_| starts_a_rule) else {
        return Vec::new();
    };
    if after.starts_with(NOT_A_PLAIN_RULE) || before.contains(['=', '$', '%']) {
        return Vec::new();
    }
    before
        .split_whitespace()
        .filter(|target| {
            !target.starts_with(SPECIAL_PREFIX)
                && target.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
                })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// Plain targets become `make <target>`; special targets, patterns,
    /// assignments and recipe lines do not.
    #[test]
    fn plain_targets_are_offered() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("Makefile"),
            ".PHONY: dev test\nCC := gcc\nOUT = bin\n%.o: %.c\n\tcc $<\ndev: build\n\t./run\ntest lint:\n\tcargo test\n",
        )
        .unwrap();

        let found: Vec<(String, bool)> = detect(dir.path())
            .iter()
            .map(|candidate| (candidate.command().to_string(), candidate.suggested()))
            .collect();
        assert_eq!(found, vec![
            ("make dev".to_string(), true),
            ("make test".to_string(), false),
            ("make lint".to_string(), false),
        ]);
    }
}
