use muster_domain::{config::ProcessSpec, process::ProcessIcon, value::ProjectRoot};

use super::{command_icon::command_icon, manifest_icon::manifest_icon};
use crate::ManifestReader;

/// The icon `spec` wears in `muster view`, the first answer of: its stored
/// icon, the tool its command starts with, the manifests of its launch folder
/// (the root when it has none or the stored one is refused), a terminal.
#[must_use]
pub fn resolve(
    spec: &ProcessSpec,
    root: &ProjectRoot,
    manifests: &dyn ManifestReader,
) -> ProcessIcon {
    if let Some(icon) = spec.icon() {
        return *icon;
    }
    let command: &str = spec.command().as_ref();
    if let Some(icon) = command_icon(command) {
        return icon;
    }
    let dir = spec
        .launch_dir(root)
        .unwrap_or_else(|_| root.as_ref().clone());
    manifest_icon(&dir, manifests).unwrap_or(ProcessIcon::Terminal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon::{
        fake_manifests::FakeManifests,
        spec_fixture::{ROOT, root, spec},
    };

    /// The icon of a process running `command` in `working_dir`, over `files`.
    fn resolved(command: &str, working_dir: Option<&str>, files: &FakeManifests) -> ProcessIcon {
        resolve(&spec(command, working_dir, None), &root(), files)
    }

    /// A stored icon wins over the command and the manifests.
    #[test]
    fn a_stored_icon_wins() {
        let files = FakeManifests::default().with(&format!("{ROOT}/Cargo.toml"), "");
        let stored = spec("cargo run", None, Some(ProcessIcon::Redis));
        assert_eq!(resolve(&stored, &root(), &files), ProcessIcon::Redis);
    }

    /// The first word names the tool, ahead of any manifest.
    #[test]
    fn the_first_word_names_the_tool() {
        let files = FakeManifests::default().with(&format!("{ROOT}/go.mod"), "");
        let cases = [
            ("cargo run", ProcessIcon::Rust),
            ("PORT=1 npm run dev", ProcessIcon::Nodedotjs),
            ("docker compose up db", ProcessIcon::Docker),
            ("./node_modules/.bin/vite", ProcessIcon::Vite),
            ("uv run app.py", ProcessIcon::Python),
            ("next dev", ProcessIcon::Nextdotjs),
            ("podman run x", ProcessIcon::Docker),
        ];
        for (command, icon) in cases {
            assert_eq!(resolved(command, None, &files), icon, "{command}");
        }
    }

    /// A command that names no tool takes the manifests of its launch folder.
    #[test]
    fn the_launch_folders_manifests_come_next() {
        let files = FakeManifests::default().with(&format!("{ROOT}/api/go.mod"), "");
        assert_eq!(resolved("make run", Some("api"), &files), ProcessIcon::Go);
        assert_eq!(
            resolved("make run", None, &files),
            ProcessIcon::Terminal,
            "the root holds no manifest"
        );
    }

    /// A launch folder that leaves the root is refused, so the root's
    /// manifests are read instead.
    #[test]
    fn a_refused_launch_folder_reads_the_root() {
        let files = FakeManifests::default()
            .with(&format!("{ROOT}/Cargo.toml"), "")
            .with("/home/me/go.mod", "");
        assert_eq!(
            resolved("make run", Some("../.."), &files),
            ProcessIcon::Rust
        );
    }

    /// `package.json` is refined by its dependencies.
    #[test]
    fn package_json_is_refined() {
        let manifest =
            |body: &str| FakeManifests::default().with(&format!("{ROOT}/package.json"), body);
        assert_eq!(
            resolved(
                "make",
                None,
                &manifest(r#"{"devDependencies":{"vite":"5"}}"#)
            ),
            ProcessIcon::Vite
        );
        assert_eq!(
            resolved(
                "make",
                None,
                &manifest(r#"{"dependencies":{"next":"14"},"devDependencies":{"vite":"5"}}"#)
            ),
            ProcessIcon::Nextdotjs
        );
        assert_eq!(
            resolved("make", None, &manifest("{")),
            ProcessIcon::Nodedotjs
        );
    }

    /// Nothing known is a terminal.
    #[test]
    fn nothing_known_is_a_terminal() {
        assert_eq!(
            resolved("redis-server", None, &FakeManifests::default()),
            ProcessIcon::Terminal
        );
    }
}
