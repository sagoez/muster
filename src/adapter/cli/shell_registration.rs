//! What a registration from a hooked shell leaves in that shell's record,
//! so `cd x && muster add --autostart` on one line is not an entering.

use std::path::Path;

use super::cli_ports::CliPorts;
use crate::domain::value::ProjectRoot;

/// Names `root`, just registered by a command run in `cwd`, in the record of
/// the hooked shell the command runs straight from, so that shell's next
/// report there finds it already in, not entering. Only when `cwd` and that
/// shell's own folder are inside `root`, the state directory and the record
/// are this user's, and the record is of the live shell the hook exported;
/// otherwise the record is left, and the shell's arrival time alone decides.
/// Best effort.
pub fn name_in_hooked_shell(ports: &dyn CliPorts, root: &ProjectRoot, cwd: &Path) {
    if !root.contains(cwd) || !ports.claim_state_dir() {
        return;
    }
    let Some(pid) = ports.hooked_shell_in(root) else {
        return;
    };
    let Ok(shells) = ports.shell_registry() else {
        return;
    };
    let Some(record) = shells.identify(pid).and_then(|shell| shells.record(&shell)) else {
        return;
    };
    let _ = shells.amend(&record.registered_around(root.clone()));
}

#[cfg(test)]
mod tests {
    use std::{fs, process};

    use rustix::process::geteuid;

    use super::{super::fake_ports::FakePorts, *};
    use crate::{
        adapter::{clock::nanos_now, store::test_support::fresh_store},
        domain::{
            occupancy::{ShellRecord, testing::stamp},
            value::Pid,
        },
    };

    /// This test process, standing for the hooked shell.
    fn this_shell() -> Pid {
        Pid::try_new(process::id()).unwrap()
    }

    /// The record of this test's shell in `ports`' shell files, if any.
    fn stored(ports: &FakePorts) -> Option<ShellRecord> {
        let shells = ports.shell_registry().unwrap();
        shells.record(&shells.identify(this_shell())?)
    }

    /// Only a registration run inside the new project, straight from the
    /// hooked shell working there, by the user owning the state, names the
    /// project in the shell's record; the stamp, folder and arrival stay.
    #[test]
    fn only_a_registration_from_inside_the_project_names_it_in_the_shell_record() {
        let (store, scratch) = fresh_store();
        let home = scratch.path().canonicalize().unwrap();
        let fresh = home.join("fresh");
        fs::create_dir_all(&fresh).unwrap();
        let root = ProjectRoot::try_new(fresh.clone()).unwrap();
        let ports = FakePorts::new(store);
        let shells = ports.shell_registry().unwrap();
        let before = ShellRecord::builder()
            .shell(shells.identify(this_shell()).unwrap())
            .stamp(stamp(1, 1))
            .folder(home.clone())
            .arrived(nanos_now())
            .build();
        shells.save_if_newer(&before).unwrap();

        name_in_hooked_shell(&ports, &root, &fresh);
        assert_eq!(stored(&ports), Some(before.clone()), "no hooked shell");

        let elsewhere = ports.hooked_from(this_shell(), &home);
        name_in_hooked_shell(&elsewhere, &root, &fresh);
        assert_eq!(stored(&elsewhere), Some(before.clone()), "shell outside it");

        let hooked = elsewhere.hooked_from(this_shell(), &fresh);
        name_in_hooked_shell(&hooked, &root, &home);
        assert_eq!(stored(&hooked), Some(before.clone()), "run outside it");

        let other_user = hooked.running_as(geteuid().as_raw() + 1);
        name_in_hooked_shell(&other_user, &root, &fresh.join("src"));
        assert_eq!(stored(&other_user), Some(before.clone()), "another user");

        let hooked = other_user.running_as(geteuid().as_raw());
        name_in_hooked_shell(&hooked, &root, &fresh.join("src"));
        assert_eq!(
            stored(&hooked),
            Some(before.registered_around(root)),
            "run inside it"
        );
    }
}
