use std::collections::HashMap;

use muster_domain::{config::Stack, value::ProjectRoot};

use crate::{ManifestReader, floor::ProjectReading};

/// Each project's reading as last resolved, kept with the stack it was
/// resolved from, so a stack that did not change reads again with no look
/// at its manifests. A cache the projects poller owns and fills as it
/// reads, so it has no builder.
#[derive(Default)]
pub struct ResolvedIcons {
    /// The stack each project's reading was resolved from, and that reading.
    by_root: HashMap<ProjectRoot, (Stack, ProjectReading)>,
}

impl ResolvedIcons {
    /// The reading of `stack`: the one kept for it when its stack is the
    /// same, else one resolved over `manifests` and kept.
    pub fn reading(&mut self, stack: Stack, manifests: &dyn ManifestReader) -> ProjectReading {
        let root = stack.project().root().clone();
        if let Some((kept, reading)) = self.by_root.get(&root)
            && *kept == stack
        {
            return reading.clone();
        }
        let reading = ProjectReading::of(&stack, manifests);
        self.by_root.insert(root, (stack, reading.clone()));
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
    use muster_domain::{project::Project, value::ProjectName};

    use super::*;
    use crate::icon::{
        fake_manifests::FakeManifests,
        spec_fixture::{root, spec},
    };

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

    /// A stack read again unchanged keeps its icons with no look at a
    /// manifest; a changed one, or one forgotten, is resolved afresh.
    #[test]
    fn an_unchanged_stack_looks_at_no_manifest() {
        let manifests = FakeManifests::default();
        let mut icons = ResolvedIcons::default();

        let first = icons.reading(stack(&["make dev"]), &manifests);
        let looked = manifests.looks();
        let again = icons.reading(stack(&["make dev"]), &manifests);
        let unchanged = manifests.looks();
        icons.reading(stack(&["make serve"]), &manifests);
        let changed = manifests.looks();
        icons.keep_only(&[]);
        icons.reading(stack(&["make serve"]), &manifests);

        assert!(looked > 0);
        assert_eq!(again, first);
        assert_eq!(unchanged, looked);
        assert!(changed > unchanged);
        assert!(manifests.looks() > changed);
    }
}
