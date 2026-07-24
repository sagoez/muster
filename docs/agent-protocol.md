# Muster agent protocol v1

Muster separates provider compatibility from session lifecycle through a small,
language-agnostic wire contract. An agent that speaks the contract does not need
provider-specific session discovery or a Rust dependency. Built-in launch and
resume behavior remains internal because durable sessions must serialize their
provider identity and commands.

## Session identity

Muster exports `MUSTER_AGENT_SESSION_ID` to every first-class agent process. This
is Muster's stable pane/session identity, not the provider conversation ID. On a
new or resumed native conversation, send this JSON to `muster hook capture` on
standard input. External integrations identify themselves as `custom`:

```sh
muster hook capture --provider custom --process-id "$PPID" \
  --parent-process-id "$(ps -o ppid= -p "$PPID" | tr -d '[:space:]')"
```

The command reads:

```json
{
  "version": 1,
  "event": "session_started",
  "session_id": "provider-owned-session-id"
}
```

The capture command is intentionally silent. When
`MUSTER_AGENT_SESSION_ID` is absent it exits successfully without recording
anything, so a user-level hook is harmless outside Muster.

`--process-id` identifies the provider process that invoked the hook. In a POSIX
shell hook, pass the parent provider process as `"$PPID"` and its parent with
`--parent-process-id`, as above. Node integrations should pass `process.pid`
and `process.ppid`, respectively. This permits a provider directly launched by
a managed shell while preventing nested agents that inherit Muster's environment
from replacing their parent's conversation.

Each event may replace the previous native ID for that Muster session. This is
how providers report legitimate in-pane conversation changes such as clearing
or switching the active thread. Events tagged with a different provider are
rejected so a nested agent cannot redirect its parent's history.

During the beta, compatibility adapters also accept provider-native hook
payloads containing `session_id` or `sessionId`. `muster hooks setup` installs
those adapters explicitly for the built-in providers.

Only `session_started` is defined in version 1. Unknown versions and unknown
versioned event names fail closed. Additive fields may be ignored by readers.

## Activity and attention

Agents should prefer existing terminal protocols instead of provider-specific
screen scraping:

- OSC 9;4 progress active marks the session working.
- OSC 9;4 progress complete marks the session waiting for input.
- OSC 9, OSC 777, Kitty desktop notifications, and the terminal bell request
  user attention.
- Terminal title changes and visible output remain compatibility fallbacks
  selected by each built-in provider integration.

## Resume

The provider owns conversation storage and semantics. Muster stores only the
native ID and the provider's durable resume behavior. A display name is metadata
and never participates in identity.

For providers that accept a caller-assigned ID, the generated ID remains
provisional until the lifecycle integration confirms it. If the process exits
before confirmation, Muster retries the original new-session command with the
same assigned ID instead of issuing a resume for a conversation that may not
exist.

Custom providers can supply a resume command in the advanced launcher. Use
an unquoted, whitespace-delimited `{session_id}` shell word where the provider ID
belongs; when omitted, Muster appends the shell-quoted ID. Quoted or embedded
placeholders are rejected in the launcher before the session is persisted
because their surrounding shell context is ambiguous. Placeholder-free
templates must also end outside quotes and without a dangling escape.
When a launch command is a shell composition such as a pipeline or sequence,
an explicit resume command is required so provider arguments are never attached
to a different command in the composition.

## Integrating another agent

1. Preserve `MUSTER_AGENT_SESSION_ID` in the agent process environment.
2. On new-session and resume lifecycle events, pipe the canonical JSON event to
   `muster hook capture --provider custom`.
3. Emit standard terminal progress or attention sequences where possible.
4. Use the Custom launcher and a resume template for providers not bundled with
   Muster.
