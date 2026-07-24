use std::time::{Duration, Instant};

use crate::domain::process::{
    ActivityState, AgentActivitySource, AgentProtocol, Process, ProcessKind,
};

/// Quiet period after activity evidence before a process returns to idle.
pub(super) const OUTPUT_IDLE_TIMEOUT: Duration = Duration::from_secs(1);

/// Which terminal evidence indicates that an agent is actively working.
/// Evidence currently governing inferred process activity.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum ActivityEvidence {
    /// No inferred timeout is scheduled.
    #[default]
    Unscheduled,
    /// Recent provider evidence keeps the process working until it expires.
    Recent(Instant),
    /// Explicit protocol progress keeps the process working until completion.
    ExplicitProgress,
}

/// Tracks provider-specific activity evidence for one terminal lifetime.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct ActivityTracker {
    strategy: AgentActivitySource,
    evidence: ActivityEvidence,
    last_title: Option<String>,
}

impl ActivityTracker {
    /// Creates a tracker using the selected process's agent preset.
    pub(super) fn for_process(process: &Process) -> Self {
        Self {
            strategy: if *process.kind() == ProcessKind::Agent {
                process
                    .agent_tool()
                    .map_or(AgentActivitySource::Output, |tool| tool.activity_source())
            } else {
                AgentActivitySource::Output
            },
            evidence: ActivityEvidence::Unscheduled,
            last_title: None,
        }
    }

    /// Records ordinary terminal output when the provider uses output activity.
    pub(super) fn observe_output(&mut self, now: Instant) -> Option<ActivityState> {
        if self.strategy != AgentActivitySource::Output {
            return None;
        }
        if self.evidence != ActivityEvidence::ExplicitProgress {
            self.evidence = ActivityEvidence::Recent(now);
        }
        Some(ActivityState::Working)
    }

    /// Records a terminal-title change when the provider uses title activity.
    pub(super) fn observe_title(&mut self, now: Instant, title: String) -> Option<ActivityState> {
        if self.strategy != AgentActivitySource::Title
            || self.last_title.as_deref() == Some(title.as_str())
        {
            return None;
        }
        self.last_title = Some(title);
        if self.evidence != ActivityEvidence::ExplicitProgress {
            self.evidence = ActivityEvidence::Recent(now);
        }
        Some(ActivityState::Working)
    }

    /// Records a protocol progress transition and returns the resulting activity.
    pub(super) fn observe_progress(&mut self, active: bool) -> ActivityState {
        if active {
            self.evidence = ActivityEvidence::ExplicitProgress;
            ActivityState::Working
        } else {
            self.evidence = ActivityEvidence::Unscheduled;
            ActivityState::AwaitingInput
        }
    }

    /// Records a notification or bell and returns the resulting activity.
    pub(super) fn observe_attention(&mut self) -> ActivityState {
        self.evidence = ActivityEvidence::Unscheduled;
        ActivityState::AwaitingInput
    }

    /// Clears all inferred activity for a new or exited child.
    pub(super) fn reset(&mut self) {
        self.evidence = ActivityEvidence::Unscheduled;
        self.last_title = None;
    }

    /// Returns when recent provider evidence should become idle, if scheduled.
    pub(super) fn deadline(&self) -> Option<Instant> {
        match self.evidence {
            ActivityEvidence::Recent(observed_at) => Some(observed_at + OUTPUT_IDLE_TIMEOUT),
            ActivityEvidence::Unscheduled | ActivityEvidence::ExplicitProgress => None,
        }
    }

    /// Expires recent provider evidence at `now`, returning the new activity only
    /// when a transition to idle occurred.
    pub(super) fn expire(&mut self, now: Instant) -> Option<ActivityState> {
        if self.deadline().is_some_and(|deadline| deadline <= now) {
            self.evidence = ActivityEvidence::Unscheduled;
            Some(ActivityState::Idle)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        process::{AgentTool, ProcessOrigin},
        value::{PaneId, ProcessName},
    };

    /// Builds an agent session for an activity strategy test.
    fn agent(tool: AgentTool) -> Process {
        Process::builder()
            .id(PaneId::new(1))
            .name(ProcessName::try_new(tool.to_string()).unwrap())
            .kind(ProcessKind::Agent)
            .agent_tool(Some(tool))
            .origin(ProcessOrigin::Session)
            .build()
    }

    /// Output-based work becomes idle after the quiet timeout.
    #[test]
    fn ordinary_output_expires_to_idle() {
        let observed_at = Instant::now();
        let mut tracker = ActivityTracker::default();

        assert_eq!(
            tracker.observe_output(observed_at),
            Some(ActivityState::Working)
        );
        assert_eq!(
            tracker.expire(observed_at + OUTPUT_IDLE_TIMEOUT),
            Some(ActivityState::Idle)
        );
        assert_eq!(tracker.evidence, ActivityEvidence::Unscheduled);
    }

    /// Title-based providers ignore visible output and deduplicate titles.
    #[test]
    fn title_strategy_ignores_output_and_observes_title_changes() {
        let observed_at = Instant::now();
        let mut tracker = ActivityTracker::for_process(&agent(AgentTool::Codex));

        assert_eq!(tracker.observe_output(observed_at), None);
        assert_eq!(
            tracker.observe_title(observed_at, "working".to_string()),
            Some(ActivityState::Working)
        );
        assert_eq!(
            tracker.observe_title(observed_at, "working".to_string()),
            None
        );
    }

    /// Explicit progress cannot be expired by ordinary provider evidence.
    #[test]
    fn explicit_progress_has_no_provider_deadline() {
        let mut tracker = ActivityTracker::default();
        tracker.observe_progress(true);
        tracker.observe_output(Instant::now());

        assert_eq!(tracker.evidence, ActivityEvidence::ExplicitProgress);
        assert!(tracker.deadline().is_none());
        assert_eq!(tracker.expire(Instant::now() + OUTPUT_IDLE_TIMEOUT), None);
    }

    /// An attention signal supersedes a pending output timeout.
    #[test]
    fn attention_clears_an_output_deadline() {
        let mut tracker = ActivityTracker::default();
        tracker.observe_output(Instant::now());

        assert_eq!(tracker.observe_attention(), ActivityState::AwaitingInput);
        assert_eq!(tracker.evidence, ActivityEvidence::Unscheduled);
    }
}
