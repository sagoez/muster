use super::{mark_kind::MarkKind, stream_mark::StreamMark};
use crate::domain::value::OutputCursor;

/// The marks a process's log records, oldest first: where each run began
/// and where the alternate screen came up or went away, so a reader that
/// starts anywhere in the output starts in the state the whole stream's
/// rendering is in there.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamMarks {
    /// The marks, ordered by offset, a run start after the screen marks at
    /// the same offset.
    marks: Vec<StreamMark>,
}

impl StreamMarks {
    /// The marks `marks` holds, in order, each once.
    pub fn new(mut marks: Vec<StreamMark>) -> Self {
        marks.sort_unstable();
        marks.dedup();
        Self { marks }
    }

    /// Every mark, oldest first.
    pub fn marks(&self) -> &[StreamMark] {
        &self.marks
    }

    /// Where each run began, oldest first.
    pub fn run_starts(&self) -> impl Iterator<Item = OutputCursor> + '_ {
        self.marks
            .iter()
            .filter(|mark| mark.kind() == MarkKind::RunStart)
            .map(|mark| mark.at())
    }

    /// Where the latest run beginning at or before `at` began.
    pub fn last_run_start(&self, at: OutputCursor) -> Option<OutputCursor> {
        self.marks[..self.end_of(at)]
            .iter()
            .rev()
            .find(|mark| mark.kind() == MarkKind::RunStart)
            .map(|mark| mark.at())
    }

    /// Whether the alternate screen is up at `at`: it came up since the
    /// latest run start there, and did not go away since.
    pub fn alternate_at(&self, at: OutputCursor) -> bool {
        self.marks[..self.end_of(at)]
            .last()
            .is_some_and(|mark| mark.kind() == MarkKind::EnterAlternate)
    }

    /// These marks for a log whose oldest byte kept is `oldest`: those from
    /// there on, and of the older ones only those the state at `oldest`
    /// comes from, the run start before it and a screen mark after that.
    /// Past `cap`, the oldest are dropped.
    pub fn retained(&self, oldest: OutputCursor, cap: usize) -> Self {
        let first_kept = self.marks.partition_point(|mark| mark.at() < oldest);
        let older = &self.marks[..first_kept];
        let run_start = older
            .iter()
            .rposition(|mark| mark.kind() == MarkKind::RunStart);
        let screen = older
            .last()
            .filter(|mark| mark.kind() != MarkKind::RunStart);
        let mut kept: Vec<StreamMark> = run_start
            .map(|index| older[index])
            .into_iter()
            .chain(screen.copied())
            .chain(self.marks[first_kept..].iter().copied())
            .collect();
        let excess = kept.len().saturating_sub(cap);
        kept.drain(..excess);
        Self { marks: kept }
    }

    /// How many marks sit at or before `at`.
    fn end_of(&self, at: OutputCursor) -> usize {
        self.marks.partition_point(|mark| mark.at() <= at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mark of `kind` at `offset`.
    fn mark(kind: MarkKind, offset: u64) -> StreamMark {
        StreamMark::builder()
            .at(OutputCursor::new(offset))
            .kind(kind)
            .build()
    }

    /// The alternate screen is up from just past the sequence that brought
    /// it up until one takes it away or a run starts, even at the offset the
    /// last run left it up at.
    #[test]
    fn the_alternate_screen_is_known_at_every_point() {
        let marks = StreamMarks::new(vec![
            mark(MarkKind::RunStart, 0),
            mark(MarkKind::EnterAlternate, 10),
            mark(MarkKind::LeaveAlternate, 20),
            mark(MarkKind::EnterAlternate, 30),
            mark(MarkKind::RunStart, 30),
            mark(MarkKind::EnterAlternate, 40),
        ]);

        let up: Vec<bool> = [0, 9, 10, 19, 20, 30, 39, 40, 1000]
            .map(|at| marks.alternate_at(OutputCursor::new(at)))
            .to_vec();

        assert_eq!(up, [
            false, false, true, true, false, false, false, true, true
        ]);
        assert_eq!(
            marks.last_run_start(OutputCursor::new(29)),
            Some(OutputCursor::new(0))
        );
        assert_eq!(marks.run_starts().count(), 2);
    }

    /// Marks older than the oldest byte kept go, but for those the state
    /// there comes from, and the cap drops the oldest of the rest.
    #[test]
    fn retained_marks_keep_the_state_at_the_oldest_byte() {
        let marks = StreamMarks::new(vec![
            mark(MarkKind::RunStart, 0),
            mark(MarkKind::RunStart, 5),
            mark(MarkKind::EnterAlternate, 8),
            mark(MarkKind::LeaveAlternate, 9),
            mark(MarkKind::EnterAlternate, 12),
            mark(MarkKind::RunStart, 50),
        ]);

        let retained = marks.retained(OutputCursor::new(20), usize::MAX);
        let capped = marks.retained(OutputCursor::new(20), 1);

        assert_eq!(retained.marks(), [
            mark(MarkKind::RunStart, 5),
            mark(MarkKind::EnterAlternate, 12),
            mark(MarkKind::RunStart, 50),
        ]);
        assert!(retained.alternate_at(OutputCursor::new(20)));
        assert_eq!(capped.marks(), [mark(MarkKind::RunStart, 50)]);
    }
}
