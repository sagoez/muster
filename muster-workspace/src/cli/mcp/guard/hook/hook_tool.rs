use std::path::{Path, PathBuf};

use serde_json::Value;
use strum::{EnumIter, EnumProperty, EnumString, IntoStaticStr};

use crate::cli::mcp::guard::shell::shell_scan::guarded_dirs;

/// The key of a shell command, and of Codex's patch, in a tool's input.
const COMMAND_KEY: &str = "command";
/// The keys naming the file an edit tool writes.
const PATH_KEYS: [&str; 2] = ["file_path", "notebook_path"];
/// The heads of the patch lines naming a file `apply_patch` writes.
const PATCH_FILE_HEADS: [&str; 4] = [
    "*** Add File: ",
    "*** Update File: ",
    "*** Delete File: ",
    "*** Move to: ",
];
/// The property marking a tool that edits files.
const EDITS: &str = "edits";
/// The property marking a tool that writes Claude Code's memory files.
const WRITES_MEMORY: &str = "writes_memory";

/// A tool the guard holds to the agent's announced work, as the plugin's
/// PreToolUse matcher names it. Each variant is spelled as the agents name
/// the tool, which is not kebab-case, so the spelling sits on the variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString, EnumIter, EnumProperty, IntoStaticStr)]
pub enum HookTool {
    /// A shell command, in both agents.
    #[strum(serialize = "Bash")]
    Bash,
    /// Claude Code's edit.
    #[strum(serialize = "Edit", props(edits = true, writes_memory = true))]
    Edit,
    /// Claude Code's write.
    #[strum(serialize = "Write", props(edits = true, writes_memory = true))]
    Write,
    /// Claude Code's multi-edit.
    #[strum(serialize = "MultiEdit", props(edits = true, writes_memory = true))]
    MultiEdit,
    /// Claude Code's notebook edit.
    #[strum(serialize = "NotebookEdit", props(edits = true))]
    NotebookEdit,
    /// Codex's patch, its only edit tool.
    #[strum(serialize = "apply_patch", props(edits = true))]
    ApplyPatch,
}

impl HookTool {
    /// Whether it edits files, rather than running a shell command.
    pub fn edits(self) -> bool {
        self.get_bool(EDITS).unwrap_or_default()
    }

    /// Whether it is Claude Code's write or edit, the tools that write its
    /// memory files.
    pub fn writes_memory(self) -> bool {
        self.get_bool(WRITES_MEMORY).unwrap_or_default()
    }

    /// What a call with `input` in `cwd` touches, each against `cwd`: for an
    /// edit, the files it writes, or `cwd` when it names none that reads;
    /// for a shell command, what its git and gh commands change or publish,
    /// as [`guarded_dirs`] finds it, `~` read as `home`.
    pub fn touched(self, input: &Value, cwd: &Path, home: Option<&Path>) -> Vec<Option<PathBuf>> {
        let command = input.get(COMMAND_KEY).and_then(Value::as_str);
        if !self.edits() {
            return command.map_or_else(Vec::new, |command| guarded_dirs(command, cwd, home));
        }
        let named: Vec<Option<PathBuf>> = match self {
            Self::ApplyPatch => command
                .iter()
                .flat_map(|patch| patch.lines())
                .filter_map(|line| {
                    PATCH_FILE_HEADS
                        .iter()
                        .find_map(|head| line.strip_prefix(head))
                })
                .map(|path| Some(cwd.join(path.trim())))
                .collect(),
            _ => PATH_KEYS
                .iter()
                .filter_map(|key| input.get(key).and_then(Value::as_str))
                .map(|path| Some(cwd.join(path)))
                .collect(),
        };
        if named.is_empty() {
            vec![Some(cwd.to_path_buf())]
        } else {
            named
        }
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use serde_json::json;
    use strum::IntoEnumIterator;

    use super::*;

    /// The folder every test call runs in.
    const CWD: &str = "/work/shop";

    /// What `tool` called with `input` touches.
    fn touched(tool: HookTool, input: &Value) -> Vec<Option<PathBuf>> {
        tool.touched(input, Path::new(CWD), None)
    }

    /// `paths` as touched paths.
    fn paths(paths: &[&str]) -> Vec<Option<PathBuf>> {
        paths.iter().map(|path| Some(PathBuf::from(path))).collect()
    }

    /// Every tool reads as the agents spell it, and an unknown tool is none;
    /// only Claude Code's write and edits write its memory.
    #[test]
    fn tools_read_as_the_agents_spell_them() {
        for tool in HookTool::iter() {
            let name: &'static str = tool.into();
            assert_eq!(HookTool::from_str(name), Ok(tool));
        }
        assert!(HookTool::from_str("Read").is_err());
        assert!(!HookTool::Bash.edits() && HookTool::ApplyPatch.edits());
        assert!(HookTool::Write.writes_memory() && !HookTool::ApplyPatch.writes_memory());
    }

    /// An edit touches the file it names, a patch every file it names, and
    /// either one naming none the folder it runs in.
    #[test]
    fn an_edit_touches_the_files_it_writes() {
        assert_eq!(
            touched(HookTool::Write, &json!({"file_path": "/tmp/notes.md"})),
            paths(&["/tmp/notes.md"])
        );
        assert_eq!(
            touched(HookTool::NotebookEdit, &json!({"notebook_path": "a.ipynb"})),
            paths(&["/work/shop/a.ipynb"])
        );
        let patch = "*** Begin Patch\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** Add File: \
                     /work/api/b.rs\n+z\n*** End Patch";
        assert_eq!(
            touched(HookTool::ApplyPatch, &json!({"command": patch})),
            paths(&["/work/shop/src/a.rs", "/work/api/b.rs"])
        );
        assert_eq!(touched(HookTool::Edit, &json!({})), paths(&[CWD]));
    }

    /// A shell command, which both agents send as one string, touches the
    /// folders of its tree-changing commands, and nothing otherwise.
    #[test]
    fn a_shell_command_touches_what_it_changes() {
        assert_eq!(
            touched(HookTool::Bash, &json!({"command": "git commit -m x"})),
            paths(&[CWD])
        );
        assert!(touched(HookTool::Bash, &json!({"command": "git status"})).is_empty());
        assert!(touched(HookTool::Bash, &json!({"command": 3})).is_empty());
    }
}
