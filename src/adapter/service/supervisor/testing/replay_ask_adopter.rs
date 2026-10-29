use std::sync::{Arc, Mutex};

use getset::Getters;
use typed_builder::TypedBuilder;

use super::replay_ask_handle::ReplayAskHandle;
use crate::{
    adapter::{holder::HolderRecord, service::supervisor::HolderAdopter},
    domain::{
        port::{OutputSink, ProcessHandle},
        process::LogPattern,
        pty::PtyError,
        value::{OutputCursor, Pid},
    },
};

/// An adopter whose every holder answers with a [`ReplayAskHandle`] that
/// records in `asked` what each replay asked for.
#[derive(Getters, TypedBuilder)]
#[getset(get = "pub(crate)")]
pub(crate) struct ReplayAskAdopter {
    asked: Arc<Mutex<Vec<Vec<LogPattern>>>>,
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
}
