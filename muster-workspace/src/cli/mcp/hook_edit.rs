/// What an edit of a hooks file comes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HookEdit {
    /// The file's new text.
    Write(String),
    /// The file's new text holds an empty object and nothing else, as when
    /// muster made the file: it is deleted, unless it is a link, which keeps
    /// this text.
    Emptied(String),
}
