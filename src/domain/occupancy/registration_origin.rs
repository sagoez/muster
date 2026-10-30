use getset::CopyGetters;
use typed_builder::TypedBuilder;

use super::ShellId;
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
    /// its folder at `arrived`: by that shell itself, or after it arrived.
    /// Either way the shell did not walk into it.
    pub fn around(&self, shell: ShellId, arrived: EpochNanos) -> bool {
        self.by == Some(shell) || self.at > arrived
    }
}

#[cfg(test)]
mod tests {
    use super::{super::testing::test_shell, *};

    /// The project at `at` nanoseconds, registered by `by`.
    fn origin(at: u64, by: Option<ShellId>) -> RegistrationOrigin {
        RegistrationOrigin::builder()
            .at(EpochNanos::new(at))
            .by(by)
            .build()
    }

    /// The registering shell never walked in, whenever it arrived; any
    /// other shell walked in only when it arrived after the registration.
    #[test]
    fn a_project_is_around_its_registrant_and_shells_that_were_there_first() {
        let (registrant, other) = (test_shell(100, 1), test_shell(200, 1));
        let by_registrant = origin(50, Some(registrant));

        assert!(by_registrant.around(registrant, EpochNanos::new(60)));
        assert!(!by_registrant.around(other, EpochNanos::new(60)));
        assert!(by_registrant.around(other, EpochNanos::new(40)));
        assert!(!origin(50, None).around(other, EpochNanos::new(50)));
    }
}
