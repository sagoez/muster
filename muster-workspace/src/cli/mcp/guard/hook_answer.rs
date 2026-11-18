use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    str::FromStr,
    time::Duration,
};

use muster_client::resolve_project;
use muster_domain::coordination::{AgentId, LockTtl};
use muster_os::clock::epoch_now;

use crate::cli::{
    command_line::cli_ports::CliPorts,
    mcp::guard::{
        checkout::Checkout, guard_facts::GuardFacts, guarded_call::GuardedCall,
        hook_event::HookEvent, hook_input::HookInput, hook_reply, verdict::Verdict,
    },
};

/// How long `muster mcp guard` may take before it ends printing nothing,
/// which lets the call run: well inside the plugin hook's own limit.
pub const GUARD_DEADLINE: Duration = Duration::from_secs(3);
/// The most bytes of hook input read: past any tool call an agent makes
/// but a huge write, which then reads as malformed and runs unguarded.
pub const GUARD_INPUT_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// The program that finds a checkout's root.
const GIT: &str = "git";
/// git's flag naming the folder it runs in.
const GIT_DIR_FLAG: &str = "-C";
/// The git command printing the working tree's root.
const SHOW_TOPLEVEL: [&str; 2] = ["rev-parse", "--show-toplevel"];
/// Ends git's output line.
const LINE_END: u8 = b'\n';

/// What `muster mcp guard` prints for the hook input `raw`, as `me`, with
/// `env_project` the folder `MUSTER_PROJECT` names: a denial or a reminder
/// as hook JSON, or none to let the call run unremarked. None on input that
/// does not read, outside a project, without a current database, or on any
/// other failure. The one write it makes renews a checkout lock `me` holds.
pub fn hook_answer(
    ports: &dyn CliPorts,
    raw: &str,
    me: &AgentId,
    env_project: Option<&Path>,
) -> Option<String> {
    let input: HookInput = serde_json::from_str(raw).ok()?;
    let event = HookEvent::from_str(input.hook_event_name()).ok()?;
    let call = GuardedCall::of(input.tool_name(), input.tool_input(), input.cwd())?;
    let store = ports.existing_state().ok()?.current()?;
    let project = resolve_project(None, env_project, input.cwd(), store.as_ref())
        .ok()?
        .within(store.as_ref())
        .ok()?;
    let todos = store.todos(project.root()).ok()?;
    let locks = store.locks(project.root()).ok()?;
    let facts = GuardFacts::builder()
        .me(me)
        .project(&project)
        .todos(&todos)
        .locks(&locks)
        .now(epoch_now())
        .build();
    match (event, call) {
        (HookEvent::PreToolUse, GuardedCall::Edit) => answered(ports, &facts, None),
        (HookEvent::PreToolUse, GuardedCall::Shell(shell)) => {
            let checkout = Checkout::at(&repo_root(shell.dir())?)?;
            answered(ports, &facts, Some(&checkout))
        },
        (HookEvent::PostToolUse, GuardedCall::Shell(shell)) if shell.publishes() => {
            Some(hook_reply::added_context(&facts.reminder()))
        },
        (HookEvent::PostToolUse, _) => None,
    }
}

/// The PreToolUse answer `facts` give a call into `checkout`, renewing the
/// lock when the verdict says so: a failed renewal still lets the call run.
fn answered(
    ports: &dyn CliPorts,
    facts: &GuardFacts<'_>,
    checkout: Option<&Checkout>,
) -> Option<String> {
    match facts.verdict(checkout) {
        Verdict::Allow => None,
        Verdict::Renew(lock) => {
            if let Ok(store) = ports.open_state() {
                let _ = store.acquire_lock(
                    facts.project().root(),
                    &lock.holder(),
                    lock.name(),
                    LockTtl::MAX,
                    lock.note().as_ref(),
                );
            }
            None
        },
        Verdict::Deny(reason) => Some(hook_reply::denial(&reason)),
    }
}

/// The root of the git working tree holding `dir`, or none outside one or
/// when git cannot run.
fn repo_root(dir: &Path) -> Option<PathBuf> {
    let output = Command::new(GIT)
        .arg(GIT_DIR_FLAG)
        .arg(dir)
        .args(SHOW_TOPLEVEL)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let mut root = output.stdout;
    if root.last() == Some(&LINE_END) {
        root.pop();
    }
    (!root.is_empty()).then(|| PathBuf::from(OsString::from_vec(root)))
}
