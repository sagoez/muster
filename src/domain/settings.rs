use getset::{Getters, WithSetters};
use serde::{Deserialize, Serialize};
use typed_builder::TypedBuilder;

/// User settings that apply across every workspace, stored per machine rather
/// than in any single project's config.
#[derive(Clone, Debug, Serialize, Deserialize, Getters, WithSetters, TypedBuilder)]
#[set_with]
pub struct Settings {
    /// Whether to raise OS desktop notifications. In-app status-bar notices are
    /// always shown regardless.
    #[getset(get = "pub", set_with = "pub")]
    desktop_notifications: bool,
    /// How to open a project in an external editor.
    #[getset(get = "pub", set_with = "pub")]
    editor: EditorSettings,
}

/// How the `e` action opens a project. The editor is launched as a detached
/// window, so a terminal editor needs a terminal launcher to open its own window;
/// a GUI editor opens directly.
#[derive(Clone, Debug, Serialize, Deserialize, Getters, WithSetters, TypedBuilder)]
#[set_with]
pub struct EditorSettings {
    /// Command that opens the editor. Empty falls back to `$VISUAL`, then
    /// `$EDITOR`. The project path is appended.
    #[getset(get = "pub", set_with = "pub")]
    command: String,
    /// Terminal launcher wrapping a terminal editor (`nvim`, `vim`, ...) so it
    /// opens its own window; muster runs `<terminal> <command> <path>`. Empty runs
    /// the editor directly, which is right for a GUI editor (`code`, `zed`).
    #[getset(get = "pub", set_with = "pub")]
    terminal: String,
}
