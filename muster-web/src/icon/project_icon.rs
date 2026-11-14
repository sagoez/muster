use muster_domain::{process::ProcessIcon, value::ProjectRoot};

use super::manifest_icon::manifest_icon;
use crate::ManifestReader;

/// The icon that occurs most in `icons`; a tie goes to the one that comes first.
fn most_common(icons: &[ProcessIcon]) -> Option<ProcessIcon> {
    let mut best: Option<(ProcessIcon, usize)> = None;
    for icon in icons {
        let count = icons.iter().filter(|other| *other == icon).count();
        if best.is_none_or(|(_, top)| count > top) {
            best = Some((*icon, count));
        }
    }
    best.map(|(icon, _)| icon)
}

/// The icon of a project whose processes wear `icons` in stack order: the most
/// common one, else the one its root's manifests name, else a terminal.
#[must_use]
pub fn project_icon(
    icons: &[ProcessIcon],
    root: &ProjectRoot,
    manifests: &dyn ManifestReader,
) -> ProcessIcon {
    most_common(icons)
        .or_else(|| manifest_icon(root.as_ref(), manifests))
        .unwrap_or(ProcessIcon::Terminal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::icon::{
        fake_manifests::FakeManifests,
        spec_fixture::{ROOT, root},
    };

    /// The icon of a project wearing `icons` over `files`.
    fn of(icons: &[ProcessIcon], files: &FakeManifests) -> ProcessIcon {
        project_icon(icons, &root(), files)
    }

    /// The most common icon wins and a tie goes to stack order.
    #[test]
    fn a_project_takes_its_most_common_icon() {
        let none = FakeManifests::default();
        assert_eq!(
            of(
                &[
                    ProcessIcon::Docker,
                    ProcessIcon::Rust,
                    ProcessIcon::Rust,
                    ProcessIcon::Docker
                ],
                &none
            ),
            ProcessIcon::Docker
        );
        assert_eq!(
            of(
                &[ProcessIcon::Vite, ProcessIcon::Rust, ProcessIcon::Rust],
                &none
            ),
            ProcessIcon::Rust
        );
        assert_eq!(of(&[ProcessIcon::Go], &none), ProcessIcon::Go);
    }

    /// With no process the root's manifests answer, else a terminal.
    #[test]
    fn a_project_without_processes_reads_its_root() {
        let files = FakeManifests::default().with(&format!("{ROOT}/Cargo.toml"), "");
        assert_eq!(of(&[], &files), ProcessIcon::Rust);
        assert_eq!(of(&[], &FakeManifests::default()), ProcessIcon::Terminal);
    }

    /// The processes' icons win over the root's manifests.
    #[test]
    fn processes_win_over_the_root_manifests() {
        let files = FakeManifests::default().with(&format!("{ROOT}/Cargo.toml"), "");
        assert_eq!(of(&[ProcessIcon::Python], &files), ProcessIcon::Python);
    }
}
