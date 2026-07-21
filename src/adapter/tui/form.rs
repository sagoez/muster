use crossterm::event::{KeyCode, KeyEvent};
use tui_input::{Input, InputRequest};

/// One editable field in a form.
pub enum Field {
    /// A free-text field (backed by `tui-input`: display-column cursor, scroll).
    Text { label: String, input: Input },
    /// A text field with a directory-autocomplete dropdown.
    Path {
        label: String,
        input: Input,
        candidates: Vec<String>,
        highlighted: usize,
    },
    /// A cyclic choice among fixed options.
    Choice {
        label: String,
        options: Vec<String>,
        selected: usize,
    },
}

impl Field {
    /// An empty text field.
    pub fn text(label: &str) -> Self {
        Self::Text {
            label: label.to_string(),
            input: Input::default(),
        }
    }

    /// A path field pre-filled with `value`, with directory autocomplete.
    pub fn path(label: &str, value: &str) -> Self {
        Self::Path {
            label: label.to_string(),
            input: Input::new(value.to_string()),
            candidates: Vec::new(),
            highlighted: 0,
        }
    }

    /// A choice field over `options`, with the first selected.
    pub fn choice(label: &str, options: &[&str]) -> Self {
        Self::Choice {
            label: label.to_string(),
            options: options.iter().map(|option| option.to_string()).collect(),
            selected: 0,
        }
    }

    /// The field's label.
    pub fn label(&self) -> &str {
        match self {
            Self::Text { label, .. } | Self::Path { label, .. } | Self::Choice { label, .. } => {
                label
            },
        }
    }

    /// The current value: the typed text, or the selected option.
    pub fn value(&self) -> String {
        match self {
            Self::Text { input, .. } | Self::Path { input, .. } => input.value().to_string(),
            Self::Choice {
                options, selected, ..
            } => options.get(*selected).cloned().unwrap_or_default(),
        }
    }

    /// For a text or path field, the slice visible within `width` columns and the
    /// cursor column within it, scrolling to the cursor when `active` and to the
    /// start otherwise. `None` for a choice field.
    pub fn visible(&self, width: usize, active: bool) -> Option<(String, usize)> {
        let (Self::Text { input, .. } | Self::Path { input, .. }) = self else {
            return None;
        };
        let width = width.max(1);
        let scroll = if active {
            input.visual_scroll(width)
        } else {
            0
        };
        let visible = input.value().chars().skip(scroll).take(width).collect();
        let cursor = input.visual_cursor().saturating_sub(scroll);
        Some((visible, cursor))
    }

    /// The autocomplete candidates of a path field, else empty.
    pub fn candidates(&self) -> &[String] {
        match self {
            Self::Path { candidates, .. } => candidates,
            _ => &[],
        }
    }

    /// The highlighted candidate index of a path field, else `0`.
    pub fn highlighted(&self) -> usize {
        match self {
            Self::Path { highlighted, .. } => *highlighted,
            _ => 0,
        }
    }

    /// Applies an editing key: typing and cursor motion for text/path (delegated
    /// to `tui-input`), left/right cycling for a choice.
    fn edit(&mut self, key: KeyEvent) {
        match self {
            Self::Text { input, .. } | Self::Path { input, .. } => {
                let request = match key.code {
                    KeyCode::Char(c) => Some(InputRequest::InsertChar(c)),
                    KeyCode::Backspace => Some(InputRequest::DeletePrevChar),
                    KeyCode::Delete => Some(InputRequest::DeleteNextChar),
                    KeyCode::Left => Some(InputRequest::GoToPrevChar),
                    KeyCode::Right => Some(InputRequest::GoToNextChar),
                    KeyCode::Home => Some(InputRequest::GoToStart),
                    KeyCode::End => Some(InputRequest::GoToEnd),
                    _ => None,
                };
                if let Some(request) = request {
                    input.handle(request);
                }
            },
            Self::Choice {
                options, selected, ..
            } => match key.code {
                KeyCode::Left => {
                    *selected = selected
                        .checked_sub(1)
                        .unwrap_or(options.len().saturating_sub(1));
                },
                KeyCode::Right if !options.is_empty() => {
                    *selected = (*selected + 1) % options.len();
                },
                _ => {},
            },
        }
    }
}

/// A multi-field form driven by keyboard input.
pub struct Form {
    title: String,
    fields: Vec<Field>,
    active: usize,
}

/// What the caller should do after a key is handled.
#[derive(Debug, PartialEq, Eq)]
pub enum FormOutcome {
    /// Keep editing.
    Continue,
    /// A completion was accepted: the value changed, but the dropdown must stay
    /// closed until the next edit rather than reopening on the new value.
    Accepted,
    /// The user accepted the form (Enter).
    Submit,
    /// The user cancelled the form (Esc).
    Cancel,
}

impl Form {
    /// A form titled `title` over `fields`, with the first field active.
    pub fn new(title: &str, fields: Vec<Field>) -> Self {
        Self {
            title: title.to_string(),
            fields,
            active: 0,
        }
    }

    /// The form title.
    pub fn title(&self) -> &str {
        &self.title
    }

    /// The form's fields, in order.
    pub fn fields(&self) -> &[Field] {
        &self.fields
    }

    /// The index of the active field.
    pub fn active(&self) -> usize {
        self.active
    }

    /// All field values, in order.
    pub fn values(&self) -> Vec<String> {
        self.fields.iter().map(Field::value).collect()
    }

    /// The active field's value when it is a path field, for recomputing its
    /// autocomplete candidates; `None` otherwise.
    pub fn active_path_value(&self) -> Option<String> {
        match self.fields.get(self.active) {
            Some(Field::Path { input, .. }) => Some(input.value().to_string()),
            _ => None,
        }
    }

    /// Replaces the active path field's autocomplete candidates.
    pub fn set_active_candidates(&mut self, candidates: Vec<String>) {
        if let Some(Field::Path {
            candidates: field,
            highlighted,
            ..
        }) = self.fields.get_mut(self.active)
        {
            *field = candidates;
            *highlighted = 0;
        }
    }

    /// Handles a key. When the active path field's dropdown is open, arrows move
    /// the highlight and Enter/Tab accept it; otherwise Esc cancels, Enter
    /// submits, Tab/arrows move between fields, and the rest edits the field.
    pub fn handle(&mut self, key: KeyEvent) -> FormOutcome {
        let dropdown = self.dropdown_open();
        match key.code {
            KeyCode::Esc => return FormOutcome::Cancel,
            KeyCode::Up if dropdown => self.move_highlight(-1),
            KeyCode::Down if dropdown => self.move_highlight(1),
            KeyCode::Enter | KeyCode::Tab if dropdown => {
                self.accept_completion();
                return FormOutcome::Accepted;
            },
            KeyCode::Enter => return FormOutcome::Submit,
            KeyCode::Tab | KeyCode::Down => self.focus_next(),
            KeyCode::BackTab | KeyCode::Up => self.focus_prev(),
            _ => {
                if let Some(field) = self.fields.get_mut(self.active) {
                    field.edit(key);
                }
            },
        }
        FormOutcome::Continue
    }

    /// Whether the active field is a path field showing candidates.
    fn dropdown_open(&self) -> bool {
        matches!(
            self.fields.get(self.active),
            Some(Field::Path { candidates, .. }) if !candidates.is_empty()
        )
    }

    /// Fills the active path field with its highlighted candidate.
    fn accept_completion(&mut self) {
        if let Some(Field::Path {
            input,
            candidates,
            highlighted,
            ..
        }) = self.fields.get_mut(self.active)
            && let Some(candidate) = candidates.get(*highlighted)
        {
            *input = Input::new(candidate.clone());
            candidates.clear();
            *highlighted = 0;
        }
    }

    /// Moves the highlighted candidate by `delta`, wrapping.
    fn move_highlight(&mut self, delta: i32) {
        if let Some(Field::Path {
            candidates,
            highlighted,
            ..
        }) = self.fields.get_mut(self.active)
            && !candidates.is_empty()
        {
            let len = candidates.len() as i32;
            *highlighted = (*highlighted as i32 + delta).rem_euclid(len) as usize;
        }
    }

    fn focus_next(&mut self) {
        if !self.fields.is_empty() {
            self.active = (self.active + 1) % self.fields.len();
        }
    }

    fn focus_prev(&mut self) {
        if !self.fields.is_empty() {
            self.active = self.active.checked_sub(1).unwrap_or(self.fields.len() - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::KeyModifiers;

    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn text_field_types_moves_and_deletes() {
        let mut field = Field::text("name");
        for c in "abc".chars() {
            field.edit(key(KeyCode::Char(c)));
        }
        assert_eq!(field.value(), "abc");
        assert_eq!(field.visible(10, true), Some(("abc".to_string(), 3)));

        field.edit(key(KeyCode::Left));
        field.edit(key(KeyCode::Char('X')));
        assert_eq!(field.value(), "abXc");

        field.edit(key(KeyCode::Backspace));
        assert_eq!(field.value(), "abc");
    }

    #[test]
    fn choice_field_cycles_and_wraps() {
        let mut field = Field::choice("kind", &["agent", "terminal", "command"]);
        assert_eq!(field.value(), "agent");
        field.edit(key(KeyCode::Right));
        assert_eq!(field.value(), "terminal");
        field.edit(key(KeyCode::Left));
        field.edit(key(KeyCode::Left));
        assert_eq!(
            field.value(),
            "command",
            "left from the first wraps to the last"
        );
        assert!(field.visible(10, true).is_none(), "a choice has no cursor");
    }

    #[test]
    fn form_navigates_submits_and_cancels() {
        let mut form = Form::new("New project", vec![
            Field::text("name"),
            Field::text("folder"),
        ]);
        for c in "muster".chars() {
            assert_eq!(form.handle(key(KeyCode::Char(c))), FormOutcome::Continue);
        }
        assert_eq!(form.handle(key(KeyCode::Tab)), FormOutcome::Continue);
        assert_eq!(form.active(), 1);
        for c in "~/p".chars() {
            form.handle(key(KeyCode::Char(c)));
        }
        assert_eq!(form.values(), vec!["muster".to_string(), "~/p".to_string()]);
        assert_eq!(form.handle(key(KeyCode::Enter)), FormOutcome::Submit);
        assert_eq!(form.handle(key(KeyCode::Esc)), FormOutcome::Cancel);
    }

    #[test]
    fn path_dropdown_navigates_and_accepts() {
        let mut form = Form::new("New project", vec![Field::path("folder", "~/w/")]);
        form.set_active_candidates(vec!["~/w/prism".to_string(), "~/w/proto".to_string()]);
        assert_eq!(form.active_path_value().as_deref(), Some("~/w/"));

        // Enter accepts the highlighted candidate instead of submitting, and
        // reports acceptance so the caller does not reopen the dropdown.
        assert_eq!(form.handle(key(KeyCode::Down)), FormOutcome::Continue);
        assert_eq!(form.handle(key(KeyCode::Enter)), FormOutcome::Accepted);
        assert_eq!(form.values(), vec!["~/w/proto".to_string()]);
        assert!(!form.dropdown_open(), "accepting clears the candidates");

        // With the dropdown closed, Enter submits.
        assert_eq!(form.handle(key(KeyCode::Enter)), FormOutcome::Submit);
    }
}
