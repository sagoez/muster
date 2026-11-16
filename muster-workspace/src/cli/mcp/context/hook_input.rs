use std::{
    io::{self, IsTerminal, Read},
    sync::mpsc,
    thread,
    time::Duration,
};

use getset::Getters;
use serde_json::Value;
use typed_builder::TypedBuilder;

/// The most bytes of hook input read: a session-start input is a few
/// hundred bytes, so anything past this is not one.
const HOOK_INPUT_MAX_BYTES: u64 = 64 * 1024;
/// How long the hook input may take to arrive: the agent writes it as the
/// hook starts, so a stdin that has said nothing by then never will.
pub const HOOK_INPUT_WAIT: Duration = Duration::from_millis(500);
/// The input key naming the session, in Claude Code's and Codex's input.
const SESSION_ID_KEY: &str = "session_id";
/// The input key naming how the session started, in both agents' input.
const SOURCE_KEY: &str = "source";
/// Between the session and the source in a mark's key.
const KEY_SEPARATOR: u8 = 0;

/// What an agent's session-start hook is told on its input, as far as
/// `muster mcp context` uses it: the session, and how it started (startup,
/// resume, clear or compact).
#[derive(Clone, Debug, PartialEq, Eq, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct HookInput {
    /// The agent's session id.
    session_id: String,
    /// How the session started, when the agent says.
    source: Option<String>,
}

impl HookInput {
    /// The hook input on stdin, waited for at most [`HOOK_INPUT_WAIT`]:
    /// none from a terminal, or when none came in time. The read runs on a
    /// thread of its own, so a stdin that never ends holds nothing up.
    pub fn from_stdin() -> Option<Self> {
        if io::stdin().is_terminal() {
            return None;
        }
        let (sent, received) = mpsc::channel();
        thread::spawn(move || {
            let _ = sent.send(Self::read(io::stdin().lock()));
        });
        received.recv_timeout(HOOK_INPUT_WAIT).ok().flatten()
    }

    /// The hook input `reader` holds, read up to [`HOOK_INPUT_MAX_BYTES`]:
    /// none when it is not a JSON object naming a session.
    pub fn read(reader: impl Read) -> Option<Self> {
        let mut text = Vec::new();
        reader
            .take(HOOK_INPUT_MAX_BYTES)
            .read_to_end(&mut text)
            .ok()?;
        let input: Value = serde_json::from_slice(&text).ok()?;
        let session_id = input
            .get(SESSION_ID_KEY)?
            .as_str()
            .filter(|id| !id.is_empty())?;
        let source = input
            .get(SOURCE_KEY)
            .and_then(Value::as_str)
            .map(str::to_owned);
        Some(
            Self::builder()
                .session_id(session_id.to_owned())
                .source(source)
                .build(),
        )
    }

    /// The bytes that name this session and source together.
    pub fn key(&self) -> Vec<u8> {
        let mut key = self.session_id.as_bytes().to_vec();
        key.push(KEY_SEPARATOR);
        key.extend(self.source.as_deref().unwrap_or_default().as_bytes());
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Claude Code's and Codex's session-start input both name the session
    /// and its source; input without a session names none.
    #[test]
    fn both_agents_inputs_name_the_session() {
        let claude = r#"{"session_id":"abc","transcript_path":"/t.jsonl","cwd":"/w","hook_event_name":"SessionStart","source":"compact"}"#;
        let codex = r#"{"session_id":"thr_1","transcript_path":null,"cwd":"/w","hook_event_name":"SessionStart","model":"m","permission_mode":"default","source":"startup"}"#;

        let read = HookInput::read(claude.as_bytes()).unwrap();
        assert_eq!(read.session_id(), "abc");
        assert_eq!(read.source().as_deref(), Some("compact"));
        assert_eq!(
            HookInput::read(codex.as_bytes())
                .unwrap()
                .source()
                .as_deref(),
            Some("startup")
        );
        for none in [
            "",
            "not json",
            "{}",
            r#"{"session_id": ""}"#,
            r#"{"session_id": 3}"#,
        ] {
            assert_eq!(HookInput::read(none.as_bytes()), None, "{none}");
        }
    }

    /// The same session with another source is another key.
    #[test]
    fn a_source_makes_its_own_key() {
        let input = |source: &str| {
            HookInput::builder()
                .session_id("abc".to_owned())
                .source(Some(source.to_owned()))
                .build()
        };

        assert_ne!(input("startup").key(), input("compact").key());
        assert_eq!(input("startup").key(), input("startup").key());
    }
}
