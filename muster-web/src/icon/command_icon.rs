use std::path::Path;

use muster_domain::process::ProcessIcon;
use shlex::Shlex;

/// The tools a command's first word names, and the icon each wears.
const TOOLS: &[(&[&str], ProcessIcon)] = &[
    (&["cargo"], ProcessIcon::Rust),
    (&["docker", "podman"], ProcessIcon::Docker),
    (
        &["npm", "pnpm", "yarn", "bun", "node", "npx"],
        ProcessIcon::Nodedotjs,
    ),
    (&["python", "python3", "uv", "poetry"], ProcessIcon::Python),
    (&["go"], ProcessIcon::Go),
    (&["vite"], ProcessIcon::Vite),
    (&["next"], ProcessIcon::Nextdotjs),
];
/// What joins a variable's name to its value in a leading assignment.
const ASSIGNMENT_MARK: char = '=';

/// Whether `word` is a shell `NAME=value` assignment.
fn is_assignment(word: &str) -> bool {
    let Some((name, _)) = word.split_once(ASSIGNMENT_MARK) else {
        return false;
    };
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && chars.all(|rest| rest.is_ascii_alphanumeric() || rest == '_')
}

/// The icon of the tool `command` starts with: its first shell word that is
/// not an assignment, by file name. None when that tool is not a known one.
pub(super) fn command_icon(command: &str) -> Option<ProcessIcon> {
    let word = Shlex::new(command).find(|word| !is_assignment(word))?;
    let tool = Path::new(&word).file_name()?.to_str()?;
    TOOLS
        .iter()
        .find(|(names, _)| names.contains(&tool))
        .map(|(_, icon)| *icon)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The first word names the tool, through a path, past assignments and
    /// inside quotes; an unknown tool names none.
    #[test]
    fn the_first_word_names_the_tool() {
        let cases = [
            ("cargo run", Some(ProcessIcon::Rust)),
            ("PORT=1 npm run dev", Some(ProcessIcon::Nodedotjs)),
            ("A=1 B=\"x y\" cargo run", Some(ProcessIcon::Rust)),
            ("docker compose up db", Some(ProcessIcon::Docker)),
            ("podman run x", Some(ProcessIcon::Docker)),
            ("./node_modules/.bin/vite", Some(ProcessIcon::Vite)),
            ("uv run app.py", Some(ProcessIcon::Python)),
            ("next dev", Some(ProcessIcon::Nextdotjs)),
            ("go run .", Some(ProcessIcon::Go)),
            ("\"/usr/bin/node\" server.js", Some(ProcessIcon::Nodedotjs)),
            ("make run", None),
            ("FOO=1", None),
            ("=1 cargo", None),
            ("redis-server", None),
        ];
        for (command, icon) in cases {
            assert_eq!(command_icon(command), icon, "{command}");
        }
    }
}
