use std::{collections::HashMap, thread, time::Instant};

use super::{
    entity::Supervisor,
    held_answer::HeldAnswer,
    held_wait::HeldWait,
    log_answer::LogAnswer,
    log_ask::LogAsk,
    log_read_job::LogReadJob,
    log_read_result::LogReadResult,
    pending_read::PendingRead,
    waiting::{FAILURE_TAIL_LINES, LOG_TAIL_TIMEOUT},
};
use crate::{
    adapter::service::{ServiceEvent, WaitOutcome, WaitReply},
    domain::value::{OutputCursor, PaneId},
};

impl Supervisor {
    /// Starts a read of `pane`'s log for what is held on it and not yet
    /// asked of a read: its end for waits on a process this service never
    /// saw run, the slices held waits scan, and the run tails held answers
    /// fill. It runs on a worker when the adopter allows it and inline
    /// otherwise. At most one read is in flight per pane: waits on the log's
    /// end join it, and anything else held meanwhile waits for the next.
    pub(super) fn read_log(&mut self, pane: PaneId) {
        if self.log_reads.contains_key(&pane) {
            return;
        }
        let id = self.next_log_read;
        let mut end = false;
        let mut asks = Vec::new();
        for held in self.held_waits.get_mut(&pane).into_iter().flatten() {
            if held.adopting {
                continue;
            }
            let Some(scan) = &held.scan else {
                end = true;
                continue;
            };
            if held.read.is_some() {
                continue;
            }
            if let Some(ask) = held.prepared.scan_ask(scan.until) {
                asks.push((held.key, ask));
                held.read = Some(id);
            }
        }
        for held in self
            .held_answers
            .iter_mut()
            .filter(|held| held.pane == pane && held.read.is_none())
        {
            asks.push((held.key, LogAsk::Lines {
                from: held.from,
                until: held.until,
                count: FAILURE_TAIL_LINES,
            }));
            held.read = Some(id);
        }
        if !end && asks.is_empty() {
            return;
        }
        self.next_log_read += 1;
        self.log_reads.insert(pane, PendingRead { id, end });
        let job = LogReadJob { end, asks };
        let (Some(adopter), Some(name)) = (
            self.adopter.as_ref(),
            self.workspace
                .process(pane)
                .map(|process| process.name().clone()),
        ) else {
            self.handle_log_read(pane, id, LogReadResult::default());
            return;
        };
        let project = self.root().clone();
        let Some(detached) = adopter.detached() else {
            let result = job.run(adopter.as_ref(), &project, &name);
            self.handle_log_read(pane, id, result);
            return;
        };
        let events = self.events.clone();
        thread::spawn(move || {
            let result = job.run(detached.as_ref(), &project, &name);
            let _ = events.send(ServiceEvent::LogRead {
                project,
                pane,
                read: id,
                result: Box::new(result),
            });
        });
    }

    /// Settles the read `read` of `pane`'s log, which landed with `result`:
    /// the answers held on it go out with the lines it read, even for a pane
    /// retired since. Unless `pane` was retired or replaced since the read
    /// started, the waits held on its end are answered, those held on its
    /// scans are registered past what they found, and anything held
    /// meanwhile is read next.
    pub fn handle_log_read(&mut self, pane: PaneId, read: u64, mut result: LogReadResult) {
        let tail = result.take_tail();
        let mut answers = result.take_answers();
        self.send_held_answers(read, &mut answers);
        let Some(end) = self
            .log_reads
            .get(&pane)
            .filter(|pending| pending.id == read)
            .map(|pending| pending.end)
        else {
            return;
        };
        if end {
            self.answer_held_waits(pane, tail);
        }
        self.register_scanned_waits(pane, read, &mut answers);
        self.log_reads.remove(&pane);
        self.read_log(pane);
    }

    /// Sends each answer held on the read `read`, with the run's last lines
    /// it read in `answers` when they hold more than the tap did.
    fn send_held_answers(&mut self, read: u64, answers: &mut HashMap<u64, LogAnswer>) {
        let landed: Vec<HeldAnswer> = self
            .held_answers
            .extract_if(.., |held| held.read == Some(read))
            .collect();
        for held in landed {
            let outcome = match answers.remove(&held.key) {
                Some(LogAnswer::Lines(lines)) if lines.len() > held.outcome.tail().len() => {
                    held.outcome.with_tail(lines)
                },
                Some(LogAnswer::Lines(_) | LogAnswer::Scan(_)) | None => held.outcome,
            };
            self.deliver(outcome, held.reply);
        }
    }

    /// Registers each wait on `pane` held on the scan the read `read` made,
    /// with what it found in `answers`.
    fn register_scanned_waits(
        &mut self,
        pane: PaneId,
        read: u64,
        answers: &mut HashMap<u64, LogAnswer>,
    ) {
        let Some(waits) = self.held_waits.get_mut(&pane) else {
            return;
        };
        let landed: Vec<HeldWait> = waits
            .extract_if(.., |held| held.scan.is_some() && held.read == Some(read))
            .collect();
        if waits.is_empty() {
            self.held_waits.remove(&pane);
        }
        for held in landed {
            let found = match answers.remove(&held.key) {
                Some(LogAnswer::Scan(found)) => Some(found),
                Some(LogAnswer::Lines(_)) | None => None,
            };
            let scan = held.scan.map(|mut scan| {
                scan.found = found;
                scan
            });
            self.register_held(
                held.prepared,
                held.after,
                WaitReply::Direct(held.reply),
                scan,
            );
        }
    }

    /// Holds `outcome`, a wait's answer on `pane` that did not match, while
    /// the last lines of its run, from `from` up to `until`, are read from
    /// the log, then sends it to `reply`.
    pub(super) fn hold_answer(
        &mut self,
        pane: PaneId,
        outcome: WaitOutcome,
        reply: WaitReply,
        from: OutputCursor,
        until: OutputCursor,
    ) {
        let key = self.next_ask_key();
        self.held_answers.push(HeldAnswer {
            pane,
            key,
            outcome,
            reply,
            from,
            until,
            deadline: Instant::now() + LOG_TAIL_TIMEOUT,
            read: None,
        });
        self.read_log(pane);
    }

    /// Sends each held answer whose read is overdue with the lines the tap
    /// held, so a log that cannot be read never holds one past
    /// [`LOG_TAIL_TIMEOUT`].
    pub(super) fn send_overdue_answers(&mut self, now: Instant) {
        let overdue: Vec<HeldAnswer> = self
            .held_answers
            .extract_if(.., |held| held.deadline <= now)
            .collect();
        for held in overdue {
            self.deliver(held.outcome, held.reply);
        }
    }

    /// The key the next held wait or answer takes.
    pub(super) fn next_ask_key(&mut self) -> u64 {
        let key = self.next_log_ask;
        self.next_log_ask += 1;
        key
    }
}
