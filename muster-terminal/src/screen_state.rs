use std::iter;

use getset::CopyGetters;

use super::keyboard_protocol::NO_FLAGS;

/// Maximum depth of the flag stack; further pushes drop the oldest entry so a
/// buggy or hostile child cannot grow muster's memory without bound. Kitty
/// itself caps the stack at this depth.
const STACK_LIMIT: usize = 16;

/// One screen's keyboard-enhancement state: the active flags and the push stack.
#[derive(Debug, Default, CopyGetters)]
pub(super) struct ScreenState {
    /// Flag values saved by pushes, restored by pops, most recent last.
    stack: Vec<u16>,
    /// The currently active flag value.
    #[getset(get_copy = "pub(super)")]
    flags: u16,
}

impl ScreenState {
    /// Saves the active flags and makes `flags` active, dropping the oldest
    /// saved level once the stack is full.
    pub(super) fn push(&mut self, flags: u16) {
        if self.stack.len() >= STACK_LIMIT {
            self.stack.remove(0);
        }
        self.stack.push(self.flags);
        self.flags = flags;
    }

    /// Makes `flags` active without saving the current ones.
    pub(super) fn set(&mut self, flags: u16) {
        self.flags = flags;
    }

    /// Restores the level saved `count` pushes ago, or no flags once the stack
    /// runs out.
    pub(super) fn pop(&mut self, count: u16) {
        for _ in 0..count {
            self.flags = self.stack.pop().unwrap_or(NO_FLAGS);
        }
    }

    /// Every level from the oldest saved one to the active one.
    pub(super) fn levels(&self) -> impl Iterator<Item = u16> + '_ {
        self.stack.iter().copied().chain(iter::once(self.flags))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pushes past the limit drop the oldest level rather than growing.
    #[test]
    fn unbalanced_pushes_stay_bounded() {
        let mut screen = ScreenState::default();

        for _ in 0..(STACK_LIMIT * 4) {
            screen.push(1);
        }

        assert_eq!(screen.levels().count(), STACK_LIMIT + 1);
    }

    /// A pop past the oldest saved level leaves no flags active.
    #[test]
    fn popping_an_empty_stack_clears_the_flags() {
        let mut screen = ScreenState::default();
        screen.push(3);

        screen.pop(2);

        assert_eq!(screen.flags(), NO_FLAGS);
    }
}
