use muster_domain::coordination::Author;
use serde::Serialize;
use strum::{Display, EnumIter, EnumMessage, EnumProperty};

/// The property naming a mark's asset file.
const ASSET_PROP: &str = "asset";
/// The property naming a mark's `#rrggbb` color.
const COLOR_PROP: &str = "color";
/// What separates an agent's name from the label it works under.
const LABEL_MARK: char = '@';
/// The name prefix of Claude agents.
const CLAUDE_PREFIX: &str = "claude";
/// The name prefix of Codex agents.
const CODEX_PREFIX: &str = "codex";

/// The mark an author wears on a note in `muster view`. Simple Icons carries
/// no OpenAI mark, so Codex and any other agent draw Lucide's `bot`.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, Hash, Display, EnumIter, EnumMessage, EnumProperty, Serialize,
)]
#[strum(serialize_all = "kebab-case")]
#[serde(rename_all = "kebab-case")]
pub enum AgentMark {
    /// A Claude agent.
    #[strum(
        message = "Claude",
        props(asset = "icons/brands/claude.svg", color = "#d97757")
    )]
    Claude,
    /// A Codex agent.
    #[strum(
        message = "Codex",
        props(asset = "icons/lucide/bot.svg", color = "#10a37f")
    )]
    Codex,
    /// Any other agent.
    #[strum(
        message = "Agent",
        props(asset = "icons/lucide/bot.svg", color = "#a1a2a8")
    )]
    Agent,
    /// The human at the CLI.
    #[strum(
        message = "You",
        props(asset = "icons/lucide/user.svg", color = "#f1f1f3")
    )]
    Human,
}

impl AgentMark {
    /// The mark of `author`: an agent by its name before the `@` of its label,
    /// lowercased, the human as itself.
    #[must_use]
    pub fn of(author: &Author) -> Self {
        let Author::Agent(label) = author else {
            return Self::Human;
        };
        let name = label
            .split(LABEL_MARK)
            .next()
            .unwrap_or_default()
            .to_lowercase();
        if name.starts_with(CLAUDE_PREFIX) {
            Self::Claude
        } else if name.starts_with(CODEX_PREFIX) {
            Self::Codex
        } else {
            Self::Agent
        }
    }

    /// The mark's title.
    #[must_use]
    pub fn title(self) -> &'static str {
        self.get_message().unwrap_or_default()
    }

    /// The mark's file under the asset root.
    #[must_use]
    pub fn asset(self) -> &'static str {
        self.get_str(ASSET_PROP).unwrap_or_default()
    }

    /// The `#rrggbb` color the mark is drawn in on the dark face disc.
    #[must_use]
    pub fn color(self) -> &'static str {
        self.get_str(COLOR_PROP).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use strum::IntoEnumIterator;

    use super::*;

    /// An agent is Claude or Codex by the name before its `@`, in any case,
    /// else a generic agent; the human is You.
    #[test]
    fn an_author_wears_its_mark() {
        let cases = [
            (Author::agent("claude-code@builder"), AgentMark::Claude),
            (Author::agent("Claude Code"), AgentMark::Claude),
            (Author::agent("Codex@x"), AgentMark::Codex),
            (Author::agent("codex-cli"), AgentMark::Codex),
            (Author::agent("zed@y"), AgentMark::Agent),
            (Author::agent("agent"), AgentMark::Agent),
            (Author::agent("@claude"), AgentMark::Agent),
            (Author::human(), AgentMark::Human),
        ];
        for (author, mark) in cases {
            assert_eq!(AgentMark::of(&author), mark, "{author}");
        }
    }

    /// Every mark has a title, an asset file and a `#rrggbb` color, and the
    /// names are kebab-case.
    #[test]
    fn every_mark_has_a_title_a_file_and_a_color() {
        for mark in AgentMark::iter() {
            assert!(!mark.title().is_empty(), "{mark}");
            assert!(mark.asset().ends_with(".svg"), "{mark}");
            let color = mark.color();
            assert_eq!(color.len(), 7, "{mark} color {color}");
            assert!(color.starts_with('#'), "{mark} color {color}");
        }
        let names: Vec<String> = AgentMark::iter().map(|mark| mark.to_string()).collect();
        assert_eq!(names, ["claude", "codex", "agent", "human"]);
        assert_eq!(AgentMark::Human.title(), "You");
        assert_eq!(AgentMark::Claude.color(), "#d97757");
    }
}
