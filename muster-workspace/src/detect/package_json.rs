use std::path::Path;

use serde_json::Value;

use super::{
    candidate::Candidate,
    detect_source::DetectSource,
    source_file::{any_exists, candidate, read_first, shell_word, suggested},
};

/// The file the scripts are read from.
const PACKAGE_JSON: &str = "package.json";
/// The key holding the scripts.
const SCRIPTS_KEY: &str = "scripts";
/// Scripts suggested by name.
const SUGGESTED: &[&str] = &["dev", "start", "serve", "watch"];
/// Scripts suggested by prefix.
const SUGGESTED_PREFIXES: &[&str] = &["dev:", "watch:"];
/// Each package manager, after the lockfiles that mark it; npm when none.
const LOCKFILES: &[(&[&str], &str)] = &[
    (&["pnpm-lock.yaml"], "pnpm"),
    (&["yarn.lock"], "yarn"),
    (&["bun.lock", "bun.lockb"], "bun"),
];
/// The package manager when no lockfile names another.
const DEFAULT_MANAGER: &str = "npm";
/// The package manager's subcommand that runs a script.
const RUN_WORD: &str = "run";

/// One candidate per `scripts` entry of `package.json`, run through the
/// package manager its lockfile names.
pub fn detect(root: &Path) -> Vec<Candidate> {
    let Some(text) = read_first(root, &[PACKAGE_JSON]) else {
        return Vec::new();
    };
    let Ok(Value::Object(package)) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    let Some(Value::Object(scripts)) = package.get(SCRIPTS_KEY) else {
        return Vec::new();
    };
    let manager = LOCKFILES
        .iter()
        .find(|(files, _)| any_exists(root, files))
        .map_or(DEFAULT_MANAGER, |(_, manager)| manager);
    scripts
        .keys()
        .filter_map(|script| {
            candidate(
                script,
                &format!("{manager} {RUN_WORD} {}", shell_word(script)),
                DetectSource::PackageJson,
                suggested(script, SUGGESTED, SUGGESTED_PREFIXES),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    /// The (name, command, suggested) of each candidate.
    fn summary(candidates: &[Candidate]) -> Vec<(String, String, bool)> {
        candidates
            .iter()
            .map(|candidate| {
                (
                    candidate.name().to_string(),
                    candidate.command().to_string(),
                    candidate.suggested(),
                )
            })
            .collect()
    }

    /// Each script runs through the manager the lockfile names; the dev
    /// ones are suggested.
    #[test]
    fn scripts_run_through_the_locked_manager() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(PACKAGE_JSON),
            r#"{"scripts": {"dev": "vite", "build": "vite build", "dev:api": "node api"}}"#,
        )
        .unwrap();
        fs::write(dir.path().join("pnpm-lock.yaml"), "").unwrap();

        let mut found = summary(&detect(dir.path()));
        found.sort();
        assert_eq!(found, vec![
            ("build".to_string(), "pnpm run build".to_string(), false),
            ("dev".to_string(), "pnpm run dev".to_string(), true),
            ("dev:api".to_string(), "pnpm run dev:api".to_string(), true),
        ]);
    }

    /// Without a lockfile npm runs them; a file that is not JSON offers
    /// nothing.
    #[test]
    fn npm_without_a_lockfile_and_nothing_from_garbage() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join(PACKAGE_JSON),
            r#"{"scripts": {"start": "node ."}}"#,
        )
        .unwrap();
        assert_eq!(summary(&detect(dir.path())), vec![(
            "start".to_string(),
            "npm run start".to_string(),
            true
        )]);
        fs::write(dir.path().join(PACKAGE_JSON), "not json").unwrap();
        assert!(detect(dir.path()).is_empty());
    }
}
