use std::collections::{HashMap, VecDeque};

use super::shell_note::ShellNote;
use crate::domain::occupancy::ShellId;

/// Most notes kept for one shell between its reports; the oldest go first.
pub const MAX_QUEUED_NOTES: usize = 16;

/// Notes found after a shell's report was answered, kept for its next one.
#[derive(Debug, Default)]
pub struct ShellNotes {
    /// Each shell's notes, oldest first.
    queued: HashMap<ShellId, VecDeque<ShellNote>>,
}

impl ShellNotes {
    /// Keeps `note` for `shell`, dropping its oldest past [`MAX_QUEUED_NOTES`].
    pub fn queue(&mut self, shell: ShellId, note: ShellNote) {
        let notes = self.queued.entry(shell).or_default();
        if notes.len() == MAX_QUEUED_NOTES {
            notes.pop_front();
        }
        notes.push_back(note);
    }

    /// Hands over every note kept for `shell`, oldest first.
    pub fn take(&mut self, shell: &ShellId) -> Vec<ShellNote> {
        self.queued.remove(shell).map(Vec::from).unwrap_or_default()
    }

    /// Drops the notes of `shell`, which is gone.
    pub fn forget(&mut self, shell: &ShellId) {
        self.queued.remove(shell);
    }
}

#[cfg(test)]
mod tests {
    use super::{super::runtime_testing::test_shell, *};
    use crate::domain::value::{ProcessName, ProjectName};

    /// A note that `index` started in the project `shop`.
    fn started(index: usize) -> ShellNote {
        ShellNote::Started {
            project: ProjectName::try_new("shop").unwrap(),
            names: vec![ProcessName::try_new(format!("p{index}")).unwrap()],
        }
    }

    #[test]
    fn queued_notes_are_capped_and_go_with_their_shell() {
        let mut notes = ShellNotes::default();
        for index in 0..MAX_QUEUED_NOTES + 2 {
            notes.queue(test_shell(101, 1), started(index));
        }
        notes.queue(test_shell(102, 1), started(0));

        let taken = notes.take(&test_shell(101, 1));
        assert_eq!(taken.len(), MAX_QUEUED_NOTES);
        assert_eq!(taken[0], started(2), "the two oldest were dropped");
        assert_eq!(taken[MAX_QUEUED_NOTES - 1], started(MAX_QUEUED_NOTES + 1));
        assert!(notes.take(&test_shell(101, 1)).is_empty(), "taken once");

        notes.forget(&test_shell(102, 1));
        assert!(notes.take(&test_shell(102, 1)).is_empty());
    }
}
