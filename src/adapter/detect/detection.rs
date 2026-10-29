use std::collections::{HashMap, HashSet};

use super::{candidate::Candidate, compose, justfile, makefile, package_json, procfile};
use crate::domain::value::{ProcessName, ProjectRoot};

/// Every candidate the files under `root` offer, by source in a fixed order.
/// A name offered by two sources is offered as `<source>-<name>` from each,
/// and a name a source repeats is offered once.
pub fn detect(root: &ProjectRoot) -> Vec<Candidate> {
    let root = root.as_ref();
    let found: Vec<Candidate> = [
        package_json::detect(root),
        procfile::detect(root),
        justfile::detect(root),
        makefile::detect(root),
        compose::detect(root),
    ]
    .into_iter()
    .flat_map(first_of_each_name)
    .collect();
    let mut sources: HashMap<String, usize> = HashMap::new();
    for candidate in &found {
        *sources.entry(candidate.name().key()).or_default() += 1;
    }
    found
        .into_iter()
        .filter_map(|candidate| {
            if sources
                .get(&candidate.name().key())
                .copied()
                .unwrap_or_default()
                < 2
            {
                return Some(candidate);
            }
            let renamed = format!("{}-{}", candidate.source(), candidate.name());
            ProcessName::try_new(renamed)
                .ok()
                .map(|name| candidate.with_name(name))
        })
        .collect()
}

/// `candidates` of one source with a repeated name kept once, first wins.
fn first_of_each_name(candidates: Vec<Candidate>) -> Vec<Candidate> {
    let mut seen = HashSet::new();
    candidates
        .into_iter()
        .filter(|candidate| seen.insert(candidate.name().key()))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;
    use crate::adapter::detect::DetectSource;

    /// The root of `dir`.
    fn root(dir: &TempDir) -> ProjectRoot {
        ProjectRoot::try_new(dir.path().canonicalize().unwrap()).unwrap()
    }

    /// A name two sources offer is prefixed by each source; the others keep
    /// theirs, and a Compose file adds the stack.
    #[test]
    fn colliding_names_are_prefixed_by_source() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"scripts": {"dev": "vite", "lint": "eslint ."}}"#,
        )
        .unwrap();
        fs::write(dir.path().join("justfile"), "dev:\n    cargo run\n").unwrap();
        fs::write(dir.path().join("compose.yaml"), "services: {}\n").unwrap();

        let found: Vec<(String, DetectSource)> = detect(&root(&dir))
            .iter()
            .map(|candidate| (candidate.name().to_string(), candidate.source()))
            .collect();
        assert_eq!(found, vec![
            ("package-json-dev".to_string(), DetectSource::PackageJson),
            ("lint".to_string(), DetectSource::PackageJson),
            ("justfile-dev".to_string(), DetectSource::Justfile),
            ("compose".to_string(), DetectSource::Compose),
        ]);
    }

    /// A process both Procfiles define is offered once, from the first.
    #[test]
    fn a_name_repeated_in_one_source_is_offered_once() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("Procfile"), "web: puma\n").unwrap();
        fs::write(dir.path().join("Procfile.dev"), "web: rails s\n").unwrap();

        let found = detect(&root(&dir));
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].command().as_ref(), "puma");
    }

    /// An empty folder offers nothing.
    #[test]
    fn an_empty_folder_offers_nothing() {
        assert!(detect(&root(&TempDir::new().unwrap())).is_empty());
    }
}
