//! Claude Code's memory files, and which of them hold what belongs in
//! muster's notes: facts about a project rather than about its user.

use std::{
    ffi::OsStr,
    path::{Component, Path, PathBuf},
    str::FromStr,
};

use serde_yaml_ng::Value as Yaml;
use strum::{EnumIter, EnumProperty, EnumString, IntoEnumIterator, IntoStaticStr};

/// The variable that moves Claude Code's config folder from `~/.claude`.
pub const CLAUDE_CONFIG_DIR_ENV: &str = "CLAUDE_CONFIG_DIR";
/// Claude Code's config folder, under the home folder.
const CLAUDE_DIR: &str = ".claude";
/// The folder under the config folder holding one folder per project.
const PROJECTS_DIR: &str = "projects";
/// The folder of a project's memory files.
const MEMORY_DIR: &str = "memory";
/// Opens and closes a memory file's frontmatter.
const FRONTMATTER_FENCE: &str = "---";
/// The frontmatter key naming a memory's type.
const TYPE_KEY: &str = "type";
/// The frontmatter key Claude Code nests a memory's type under.
const METADATA_KEY: &str = "metadata";
/// Opens a path read from the home folder.
const HOME_PREFIX: &str = "~";
/// Joins the frontmatter's lines again for the YAML parser.
const LINE_BREAK: &str = "\n";
/// What Claude Code writes, in a project's folder name, for each character
/// of its path that is not a letter or digit.
const FOLDER_NAME_SEPARATOR: char = '-';
/// The byte order mark an editor may put before the first fence.
const BOM: char = '\u{feff}';
/// The property marking a memory type every agent should read.
const SHARED: &str = "shared";
/// Joins the names of the two types memory keeps.
const AND: &str = " and ";

/// The type a Claude Code memory file declares. A fact about the work is
/// shared: every agent should read it, so it belongs in a muster note. The
/// rest are about how to work with the user, which that agent's memory
/// keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumString, EnumIter, EnumProperty, IntoStaticStr)]
#[strum(serialize_all = "kebab-case", ascii_case_insensitive)]
pub enum MemoryType {
    /// Who the user is.
    User,
    /// How the user wants the work done.
    Feedback,
    /// Ongoing work, decisions and constraints of a project.
    #[strum(props(shared = true))]
    Project,
    /// Where something lives outside the code: tickets, dashboards, URLs.
    #[strum(props(shared = true))]
    Reference,
}

impl MemoryType {
    /// Whether every agent should read a memory of this type.
    pub fn shared(self) -> bool {
        self.get_bool(SHARED).unwrap_or_default()
    }

    /// The names of the two types an agent's own memory keeps, joined as a
    /// sentence lists them: `user and feedback`.
    pub fn kept_names() -> String {
        Self::iter()
            .filter(|kind| !kind.shared())
            .map(<&'static str>::from)
            .collect::<Vec<_>>()
            .join(AND)
    }
}

/// Claude Code's config folder: the one `config_env` names, a leading `~/`
/// read as `home`, else `.claude` in `home`. A value that is empty or
/// otherwise relative is ignored, as the hook runs in another folder than
/// Claude Code started in.
pub fn claude_dir(config_env: Option<PathBuf>, home: Option<&Path>) -> Option<PathBuf> {
    config_env
        .map(|dir| match (dir.strip_prefix(HOME_PREFIX), home) {
            (Ok(rest), Some(home)) => home.join(rest),
            _ => dir,
        })
        .filter(|dir| dir.is_absolute())
        .or_else(|| home.map(|home| home.join(CLAUDE_DIR)))
}

/// Whether `path` passes through a folder named as a memory folder, which
/// any memory file's path does, as named or as it resolves.
pub fn names_a_memory_folder(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str() == OsStr::new(MEMORY_DIR))
}

/// The project folder `path` keeps memory for, when it lies anywhere below
/// `<claude_dir>/projects/<folder>/memory/`, read by name; none for a path
/// holding `..`, which only the resolved path can place.
pub fn memory_folder<'a>(path: &'a Path, claude_dir: &Path) -> Option<&'a OsStr> {
    if path.components().any(|part| part == Component::ParentDir) {
        return None;
    }
    let mut parts = path
        .strip_prefix(claude_dir.join(PROJECTS_DIR))
        .ok()?
        .components();
    let Some(Component::Normal(folder)) = parts.next() else {
        return None;
    };
    (parts.next() == Some(Component::Normal(OsStr::new(MEMORY_DIR))) && parts.next().is_some())
        .then_some(folder)
}

/// The folder name Claude Code keeps a project's memory under, for a
/// project at `root`: its path with every character but a letter or digit
/// as `-`.
pub fn claude_folder_name(root: &Path) -> String {
    root.to_string_lossy()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                FOLDER_NAME_SEPARATOR
            }
        })
        .collect()
}

/// Whether `text` opens with closed YAML frontmatter declaring a shared
/// type, such as `type: project`, at its top level or under `metadata`, as
/// Claude Code writes it, in any case. Frontmatter that is not valid YAML
/// declares nothing.
pub fn holds_shared_fact(text: &str) -> bool {
    let Some(yaml) =
        frontmatter(text).and_then(|block| serde_yaml_ng::from_str::<Yaml>(&block).ok())
    else {
        return false;
    };
    [
        yaml.get(TYPE_KEY),
        yaml.get(METADATA_KEY)
            .and_then(|metadata| metadata.get(TYPE_KEY)),
    ]
    .into_iter()
    .flatten()
    .filter_map(Yaml::as_str)
    .filter_map(|kind| MemoryType::from_str(kind.trim()).ok())
    .any(MemoryType::shared)
}

/// The lines between the fence opening `text`, after any byte order mark,
/// and the unindented fence closing them, so a `---` inside a block scalar
/// does not end them; none when either is missing.
fn frontmatter(text: &str) -> Option<String> {
    let mut lines = text.trim_start_matches(BOM).lines();
    if lines.next()?.trim() != FRONTMATTER_FENCE {
        return None;
    }
    let mut block: Vec<&str> = Vec::new();
    for line in lines {
        if line.trim_end() == FRONTMATTER_FENCE {
            return Some(block.join(LINE_BREAK));
        }
        block.push(line);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The config folder every test reads under.
    const CLAUDE: &str = "/home/me/.claude";

    /// The project folder a path keeps memory for, if any.
    fn folder_of(path: &str) -> Option<&OsStr> {
        memory_folder(Path::new(path), Path::new(CLAUDE))
    }

    /// A file anywhere below a project's memory folder keeps that project's
    /// memory; nothing else does, nor any path holding `..`.
    #[test]
    fn only_a_project_memory_file_keeps_memory() {
        assert_eq!(
            folder_of("/home/me/.claude/projects/-work-shop/memory/a.md"),
            Some(OsStr::new("-work-shop"))
        );
        assert_eq!(
            folder_of("/home/me/.claude/projects/-work-shop/memory/topics/a.md"),
            Some(OsStr::new("-work-shop"))
        );
        for elsewhere in [
            "/home/me/.claude/projects/-work-shop/a.md",
            "/home/me/.claude/projects/-work-shop/memory",
            "/home/me/.claude/skills/x/memory/a.md",
            "/work/shop/projects/x/memory/a.md",
            "/home/me/.claude/projects/-work-shop/link/../memory/a.md",
        ] {
            assert_eq!(folder_of(elsewhere), None, "{elsewhere}");
        }
        assert!(names_a_memory_folder(Path::new("/a/memory/b.md")));
        assert!(!names_a_memory_folder(Path::new("/a/memories/b.md")));
    }

    /// A project's folder name is its path with every character but a
    /// letter or digit as `-`, as Claude Code names it.
    #[test]
    fn a_project_folder_is_named_as_claude_code_names_it() {
        assert_eq!(
            claude_folder_name(Path::new("/home/me/.muster-worktrees/a_b")),
            "-home-me--muster-worktrees-a-b"
        );
    }

    /// The config folder is the one the variable names, `~/` read as home,
    /// else `.claude` at home, and none without either; an empty or other
    /// relative value falls back to home.
    #[test]
    fn the_config_folder_follows_its_variable() {
        let home = Some(Path::new("/home/me"));
        assert_eq!(
            claude_dir(Some(PathBuf::from("/etc/claude")), home),
            Some(PathBuf::from("/etc/claude"))
        );
        assert_eq!(claude_dir(None, home), Some(PathBuf::from(CLAUDE)));
        assert_eq!(
            claude_dir(Some(PathBuf::new()), home),
            Some(PathBuf::from(CLAUDE))
        );
        assert_eq!(
            claude_dir(Some(PathBuf::from("claude")), home),
            Some(PathBuf::from(CLAUDE))
        );
        assert_eq!(
            claude_dir(Some(PathBuf::from("~/.claude-work")), home),
            Some(PathBuf::from("/home/me/.claude-work"))
        );
        assert_eq!(claude_dir(None, None), None);
    }

    /// A project or reference type at the top level or under `metadata`,
    /// quoted, in any case, with comments or blank lines around it, after a
    /// byte order mark, or in flow style, holds a shared fact. A user or
    /// feedback type, a type deeper or under another key, inside a block
    /// scalar, past the frontmatter or in frontmatter never closed or not
    /// YAML, and text with no frontmatter do not.
    #[test]
    fn project_and_reference_types_hold_shared_facts() {
        for shared in [
            "---\nname: a\ntype: project\n---\nbody",
            "---\nname: a\nmetadata:\n  type: \"reference\"\n---\n",
            "---\ntype: Project\n---\n",
            "---\ntype: project\t# agreed today\n---\n",
            "---\nmetadata: # by claude\n\n  # why\n  type: project\n---\n",
            "---\nmetadata: {type: project}\n---\n",
            "---\ndescription: |\n  ---\ntype: project\n---\nfact",
            "\u{feff}---\ntype: project\n---\n",
        ] {
            assert!(holds_shared_fact(shared), "{shared:?}");
        }
        for kept in [
            "---\ntype: feedback\n---\n",
            "---\ntype: user\n---\n",
            "---\nnode_type: project\n---\n",
            "---\nname: a\n---\ntype: project\n",
            "---\ntype: project\n",
            "type: project\n",
            "---\ntype: feedback\nsource:\n  type: reference\n---\n",
            "---\ntype: user\ndescription: |\n  type: project\n---\n",
            "---\nmetadata:\n  type: feedback\n  source:\n    type: project\n---\n",
            "---\ntype: [project\n---\n",
        ] {
            assert!(!holds_shared_fact(kept), "{kept:?}");
        }
    }

    /// Memory keeps the types that are not shared, named as a sentence does.
    #[test]
    fn memory_keeps_user_and_feedback() {
        assert_eq!(MemoryType::kept_names(), "user and feedback");
    }
}
