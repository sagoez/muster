use std::path::Path;

use super::{
    candidate::Candidate,
    detect_source::DetectSource,
    source_file::{any_exists, candidate},
};

/// The Compose file names `docker compose` looks for.
const COMPOSE_FILES: &[&str] = &[
    "compose.yaml",
    "compose.yml",
    "docker-compose.yml",
    "docker-compose.yaml",
];
/// The name the Compose stack is offered under.
const COMPOSE_NAME: &str = "compose";
/// The command that brings the Compose stack up in the foreground.
const COMPOSE_UP: &str = "docker compose up";

/// One suggested candidate running the Compose stack, when a Compose file
/// is present.
pub fn detect(root: &Path) -> Vec<Candidate> {
    if !any_exists(root, COMPOSE_FILES) {
        return Vec::new();
    }
    candidate(COMPOSE_NAME, COMPOSE_UP, DetectSource::Compose, true)
        .into_iter()
        .collect()
}
