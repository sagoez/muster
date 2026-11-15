use std::collections::HashMap;

use getset::Getters;
use muster_domain::{config::Stack, value::ProjectRoot};
use typed_builder::TypedBuilder;

use super::stamping_manifests::{ManifestStamps, StampingManifests, still_stamped};
use crate::{ManifestReader, floor::ProjectReading};

/// Each project's reading as last resolved, kept with the stack it was
/// resolved from and the stamp of every manifest it looked at, so a stack
/// whose manifests did not change reads again with a stat of each and no
/// read. Its builder takes nothing, so every cache starts empty. It has no
/// getters: what it keeps reaches the poller only through
/// [`ResolvedIcons::reading`].
#[derive(Getters, TypedBuilder)]
pub struct ResolvedIcons {
    /// The stack each project's reading was resolved from, the manifests
    /// that resolution looked at, and that reading.
    #[builder(default, setter(skip))]
    by_root: HashMap<ProjectRoot, (Stack, ManifestStamps, ProjectReading)>,
}

impl ResolvedIcons {
    /// The reading of `stack`: the one kept for it when its stack is the
    /// same and every manifest it looked at stats as it did, else one
    /// resolved over `manifests` and kept.
    pub fn reading(&mut self, stack: Stack, manifests: &dyn ManifestReader) -> ProjectReading {
        let root = stack.project().root().clone();
        if let Some((kept, stamps, reading)) = self.by_root.get(&root)
            && *kept == stack
            && still_stamped(stamps, manifests)
        {
            return reading.clone();
        }
        let stamping = StampingManifests::builder().inner(manifests).build();
        let reading = ProjectReading::of(&stack, &stamping);
        self.by_root
            .insert(root, (stack, stamping.into_stamps(), reading.clone()));
        reading
    }

    /// Forgets every project but those `listed`, as after a reading that
    /// no longer lists the others.
    pub fn keep_only(&mut self, listed: &[ProjectRoot]) {
        self.by_root.retain(|root, _| listed.contains(root));
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use muster_domain::{process::ProcessIcon, project::Project, value::ProjectName};

    use super::*;
    use crate::icon::{
        fake_manifests::FakeManifests,
        spec_fixture::{ROOT, root, spec},
    };

    /// A `package.json` that makes a Vite project.
    const VITE_PACKAGE: &str = r#"{"devDependencies":{"vite":"5"}}"#;
    /// A `package.json` as long as [`VITE_PACKAGE`] that makes a Next.js one.
    const NEXT_PACKAGE: &str = r#"{"devDependencies":{"next":"5"}}"#;

    /// The fixture project's stack running each of `commands`, none of which
    /// names its tool, so its icons come from the manifests.
    fn stack(commands: &[&str]) -> Stack {
        Stack::builder()
            .project(
                Project::builder()
                    .root(root())
                    .name(ProjectName::try_new("shop").unwrap())
                    .build(),
            )
            .processes(
                commands
                    .iter()
                    .map(|command| spec(command, None, None))
                    .collect(),
            )
            .build()
    }

    /// The fixture root's `package.json`.
    fn package_json() -> String {
        format!("{ROOT}/package.json")
    }

    /// The icon of the one process in `reading`.
    fn icon(reading: &ProjectReading) -> ProcessIcon {
        *reading.processes()[0].icon()
    }

    /// A stack read again with its manifests unchanged keeps its icons with
    /// no read of a manifest; a changed stack, or one forgotten, is
    /// resolved afresh.
    #[test]
    fn an_unchanged_stack_reads_no_manifest() {
        let manifests = FakeManifests::default().with(&package_json(), VITE_PACKAGE);
        let mut icons = ResolvedIcons::builder().build();

        let first = icons.reading(stack(&["make dev"]), &manifests);
        let read = manifests.reads();
        let again = icons.reading(stack(&["make dev"]), &manifests);
        let unchanged = manifests.reads();
        icons.reading(stack(&["make serve"]), &manifests);
        let changed = manifests.reads();
        icons.keep_only(&[]);
        icons.reading(stack(&["make serve"]), &manifests);

        assert_eq!(icon(&first), ProcessIcon::Vite);
        assert!(read > 0);
        assert_eq!(again, first);
        assert_eq!(unchanged, read);
        assert!(manifests.stats() > 0);
        assert!(changed > unchanged);
        assert!(manifests.reads() > changed);
    }

    /// A manifest that appears after the stack was resolved without it
    /// gives the next reading its icon.
    #[test]
    fn a_manifest_that_appears_changes_the_icon() {
        let mut icons = ResolvedIcons::builder().build();

        let before = icons.reading(stack(&["make dev"]), &FakeManifests::default());
        let after = icons.reading(
            stack(&["make dev"]),
            &FakeManifests::default().with(&package_json(), VITE_PACKAGE),
        );

        assert_eq!(icon(&before), ProcessIcon::Terminal);
        assert_eq!(icon(&after), ProcessIcon::Vite);
    }

    /// A manifest written again, even to the same length, gives the next
    /// reading the icon it names now.
    #[test]
    fn a_manifest_that_changes_changes_the_icon() {
        let mut icons = ResolvedIcons::builder().build();
        let rewritten = UNIX_EPOCH + Duration::from_secs(1);

        let before = icons.reading(
            stack(&["make dev"]),
            &FakeManifests::default().with(&package_json(), VITE_PACKAGE),
        );
        let after = icons.reading(
            stack(&["make dev"]),
            &FakeManifests::default().written_at(&package_json(), NEXT_PACKAGE, rewritten),
        );

        assert_eq!(icon(&before), ProcessIcon::Vite);
        assert_eq!(icon(&after), ProcessIcon::Nextdotjs);
    }
}
