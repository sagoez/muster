use std::{path::PathBuf, time::Duration};

use crate::domain::{
    color_scheme::ColorScheme,
    config::{McpStackEdits, ProcessDraft, ProcessSpec, Registration},
    process::{RestartPolicy, StopSignal},
    project::Project,
    pty::PtySize,
    settings::Settings,
    value::{
        CommandLine, DetachKey, ProcessName, ProjectName, ProjectRoot, ScrollbackLines, StopGrace,
    },
};

/// The root of the project every editor test edits.
pub(super) fn root() -> ProjectRoot {
    ProjectRoot::try_new(PathBuf::from("/home/me/shop")).unwrap()
}

/// The registration of [`root`] as the newly registered `shop`.
pub(super) fn registration() -> Registration {
    Registration::builder()
        .project(
            Project::builder()
                .root(root())
                .name(ProjectName::try_new("shop").unwrap())
                .build(),
        )
        .newly_registered(true)
        .build()
}

/// A stop grace of `seconds`.
pub(super) fn grace(seconds: u64) -> StopGrace {
    StopGrace::try_new(Duration::from_secs(seconds)).unwrap()
}

/// Settings with every value explicit and distinct from a process's own.
pub(super) fn settings() -> Settings {
    Settings::builder()
        .desktop_notifications(true)
        .default_size(PtySize::try_new(24, 80).unwrap())
        .detach_key(DetachKey::try_new("ctrl-\\").unwrap())
        .log_max_size("10MiB".parse().unwrap())
        .scrollback(ScrollbackLines::try_new(1000).unwrap())
        .color_scheme(ColorScheme::Dark)
        .default_autostart(true)
        .default_restart(RestartPolicy::OnFailure)
        .default_stop_signal(StopSignal::Interrupt)
        .default_stop_grace(grace(7))
        .mcp_stack_edits(McpStackEdits::Allow)
        .idle_stop("30m".parse().unwrap())
        .build()
}

/// The name of the process every editor test edits.
pub(super) fn name() -> ProcessName {
    ProcessName::try_new("web").unwrap()
}

/// A draft of [`name`] running a dev server, leaving the rest to defaults.
pub(super) fn draft() -> ProcessDraft {
    ProcessDraft::builder()
        .name(name())
        .command(CommandLine::try_new("npm run dev").unwrap())
        .build()
}

/// [`draft`] completed from [`settings`].
pub(super) fn spec() -> ProcessSpec {
    draft().complete(&settings().process_defaults())
}
