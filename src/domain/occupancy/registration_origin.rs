use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::{ShellId, ShellRecord};
use crate::domain::value::EpochNanos;

/// When a project was registered, and the hooked shell that registered it,
/// when one did.
#[derive(Debug, Clone, Copy, PartialEq, Eq, CopyGetters, TypedBuilder)]
#[getset(get_copy = "pub")]
pub struct RegistrationOrigin {
    /// When it was registered.
    at: EpochNanos,
    /// The shell `muster` was run from to register it, if any.
    by: Option<ShellId>,
}

impl RegistrationOrigin {
    /// Whether the project was registered around `shell`, which arrived in
    /// its folder at `arrived` after `previous`, its last stored report:
    /// after it arrived, or by that shell itself while it has not left since,
    /// that is while `previous`, if any, is from a folder it arrived in
    /// before the registration. Either way the shell did not walk into it.
    pub fn around(
        &self,
        shell: ShellId,
        arrived: EpochNanos,
        previous: Option<&ShellRecord>,
    ) -> bool {
        let registrant_stayed =
            self.by == Some(shell) && previous.is_none_or(|previous| *previous.arrived() < self.at);
        registrant_stayed || self.at > arrived
    }
}

#[cfg(test)]
mod tests {
    use super::{
        super::testing::{stamp, test_root, test_shell},
        *,
    };

    /// The project at `at` nanoseconds, registered by `by`.
    fn origin(at: u64, by: Option<ShellId>) -> RegistrationOrigin {
        RegistrationOrigin::builder()
            .at(EpochNanos::new(at))
            .by(by)
            .build()
    }

    /// `shell`'s stored report from a folder it arrived in at `arrived`.
    fn seen(shell: ShellId, arrived: u64) -> ShellRecord {
        ShellRecord::builder()
            .shell(shell)
            .stamp(stamp(1, 1))
            .folder(test_root("elsewhere").as_ref().to_path_buf())
            .arrived(EpochNanos::new(arrived))
            .build()
    }

    /// The registering shell does not walk in, whenever it arrived, until
    /// it has been seen in a folder it arrived in after the registration,
    /// having left; any other shell walks in only when it arrived after the
    /// registration.
    #[test]
    fn a_project_is_around_its_registrant_until_it_leaves_and_shells_there_first() {
        let (registrant, other) = (test_shell(100, 1), test_shell(200, 1));
        let by_registrant = origin(50, Some(registrant));
        let now = EpochNanos::new(60);

        assert!(by_registrant.around(registrant, now, None));
        assert!(by_registrant.around(registrant, now, Some(&seen(registrant, 40))));
        assert!(
            !by_registrant.around(registrant, now, Some(&seen(registrant, 55))),
            "it left after registering"
        );
        assert!(!by_registrant.around(other, now, Some(&seen(other, 40))));
        assert!(by_registrant.around(other, EpochNanos::new(40), None));
        assert!(!origin(50, None).around(other, EpochNanos::new(50), None));
    }
}
