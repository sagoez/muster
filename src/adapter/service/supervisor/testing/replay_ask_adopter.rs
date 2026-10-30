use std::sync::{Arc, Mutex};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::replay_ask_handle::ReplayAskHandle;
use crate::{
    adapter::{
        holder::{HolderRecord, LineMatcher, LogScan, LoggedLine},
        service::supervisor::HolderAdopter,
    },
    domain::{
        port::{OutputSink, ProcessHandle},
        process::LogPattern,
        pty::PtyError,
        value::{OutputCursor, Pid, ProcessName, ProjectRoot},
    },
};

/// An adopter whose every holder answers with a [`ReplayAskHandle`] that
/// records in `asked` what each replay asked for, and whose log holds the
/// lines `logged`, oldest first, the first of them its oldest kept byte. Its
/// log is read on a worker, as the service's is, when `detach` is set.
#[derive(Clone, Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct ReplayAskAdopter {
    asked: Arc<Mutex<Vec<Vec<LogPattern>>>>,
    #[builder(default)]
    logged: Vec<LoggedLine>,
    #[builder(default)]
    detach: bool,
}

impl ReplayAskAdopter {
    /// The logged lines that begin from `from` up to `until`.
    fn lines_within(
        &self,
        from: OutputCursor,
        until: OutputCursor,
    ) -> impl Iterator<Item = &LoggedLine> {
        self.logged
            .iter()
            .filter(move |line| line.at() >= from && line.at() < until)
    }
}

impl HolderAdopter for ReplayAskAdopter {
    fn locate(&self, _child_pid: Pid) -> Option<HolderRecord> {
        None
    }

    fn serving(&self, _child_pid: Pid) -> Option<HolderRecord> {
        None
    }

    fn adopt(
        &self,
        _record: &HolderRecord,
        _since: Option<OutputCursor>,
        _sink: Box<dyn OutputSink>,
    ) -> Result<Box<dyn ProcessHandle>, PtyError> {
        Ok(Box::new(
            ReplayAskHandle::builder()
                .asked(Arc::clone(&self.asked))
                .build(),
        ))
    }

    fn log_scan(
        &self,
        _project: &ProjectRoot,
        _name: &ProcessName,
        from: OutputCursor,
        until: OutputCursor,
        matcher: &LineMatcher,
    ) -> Option<LogScan> {
        let began = self
            .logged
            .first()
            .map_or(from, |oldest| from.max(oldest.at()));
        let matched = self
            .lines_within(began, until)
            .find(|line| matcher.is_match(line.text()))
            .cloned();
        Some(LogScan::builder().from(began).matched(matched).build())
    }

    fn log_lines(
        &self,
        _project: &ProjectRoot,
        _name: &ProcessName,
        from: OutputCursor,
        until: OutputCursor,
        count: usize,
    ) -> Option<Vec<String>> {
        let lines: Vec<String> = self
            .lines_within(from, until)
            .map(|line| line.text().clone())
            .collect();
        Some(lines[lines.len().saturating_sub(count)..].to_vec())
    }

    fn detached(&self) -> Option<Arc<dyn HolderAdopter + Send + Sync>> {
        self.detach
            .then(|| Arc::new(self.clone()) as Arc<dyn HolderAdopter + Send + Sync>)
    }
}
