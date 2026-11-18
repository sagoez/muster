use std::{
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    str::FromStr,
    thread,
    time::{Duration, Instant},
};

use muster_client::resolve_project;
use muster_domain::{coordination::AgentId, port::LocalState, project::Project};
use muster_os::clock::epoch_now;

use crate::cli::{
    command_line::cli_ports::CliPorts,
    mcp::guard::{
        checkout_scope::{CheckoutScope, checkout_lock},
        hook::{hook_input::HookInput, hook_reply::denial, hook_tool::HookTool},
        refusal::refusal,
        shell::shell_scan::{GIT, GIT_DIR_FLAG},
        todo_scope::TodoScope,
    },
};

/// How long `muster mcp guard` may take before it ends printing nothing,
/// which lets the call run: well inside the plugin hook's own limit.
pub const GUARD_DEADLINE: Duration = Duration::from_secs(3);
/// The most bytes of hook input read: past any tool call an agent makes
/// but a huge write, which then reads as malformed and runs unguarded.
pub const GUARD_INPUT_MAX_BYTES: u64 = 16 * 1024 * 1024;
/// How long all the gits finding checkouts' roots may run together before
/// the one still running is killed, inside [`GUARD_DEADLINE`].
const GIT_DEADLINE: Duration = Duration::from_secs(2);
/// How often a git is checked for having ended.
const GIT_POLL: Duration = Duration::from_millis(10);
/// The git command printing the working tree's root.
const SHOW_TOPLEVEL: [&str; 2] = ["rev-parse", "--show-toplevel"];
/// Variables that would point git at another repository than the folder's.
const GIT_LOCATION_ENV: [&str; 2] = ["GIT_DIR", "GIT_WORK_TREE"];
/// Variables keeping that git from taking locks or asking for anything.
const GIT_QUIET_ENV: [(&str, &str); 2] =
    [("GIT_OPTIONAL_LOCKS", "0"), ("GIT_TERMINAL_PROMPT", "0")];
/// Ends git's output line.
const LINE_END: u8 = b'\n';

/// What `muster mcp guard` prints for the PreToolUse input `raw`, as `me`,
/// with `env_project` the folder `MUSTER_PROJECT` names and `home` the
/// user's home folder: a denial as hook JSON, or none to let the call run.
/// An edit needs a todo `me` holds in progress in each project holding a
/// file it writes. A git or gh command changing or publishing a checkout
/// needs the checkout's lock and such a todo, both in the checkout's
/// project: the one containing its root, else the first registered project
/// inside it by root, else the session's; and, for a folder git cannot
/// resolve, the todo in the session's project. None on input that does not
/// read, for a call touching no project, without a current database, or on
/// any other failure. It only reads.
pub fn hook_answer(
    ports: &dyn CliPorts,
    raw: &str,
    me: &AgentId,
    env_project: Option<&Path>,
    home: Option<&Path>,
) -> Option<String> {
    let input: HookInput = serde_json::from_str(raw).ok()?;
    let tool = HookTool::from_str(input.tool_name()).ok()?;
    let touched = tool.touched(input.tool_input(), input.cwd(), home);
    if touched.is_empty() {
        return None;
    }
    let state = ports.existing_state().ok()?.current()?;
    let store = state.as_ref();
    let (todo_projects, checkouts) = if tool.edits() {
        let projects: Vec<Project> = touched
            .iter()
            .filter_map(|path| containing(store, path))
            .collect();
        (projects, Vec::new())
    } else {
        let session = resolve_project(None, env_project, input.cwd(), store)
            .ok()
            .and_then(|target| target.within(store).ok());
        let deadline = Instant::now() + GIT_DEADLINE;
        let mut roots: Vec<PathBuf> = Vec::new();
        let mut unresolved = false;
        for dir in &touched {
            match repo_root(dir, deadline) {
                Some(root) if !roots.contains(&root) => roots.push(root),
                Some(_) => {},
                None => unresolved = true,
            }
        }
        let checkouts = roots
            .into_iter()
            .filter_map(|root| {
                let project = lock_project(store, &root).or_else(|| session.clone())?;
                checkout_scope(store, root, project)
            })
            .collect::<Vec<CheckoutScope>>();
        let mut projects: Vec<Project> = checkouts
            .iter()
            .map(|checkout| checkout.project().clone())
            .collect();
        if unresolved || checkouts.is_empty() {
            projects.extend(session);
        }
        (projects, checkouts)
    };
    let mut todo_scopes: Vec<TodoScope> = Vec::new();
    for project in todo_projects {
        if todo_scopes
            .iter()
            .all(|scope| scope.project().root() != project.root())
        {
            let todos = store.todos(project.root()).ok()?;
            todo_scopes.push(TodoScope::builder().project(project).todos(todos).build());
        }
    }
    refusal(me, &todo_scopes, &checkouts, epoch_now()).map(|reason| denial(&reason))
}

/// The registered project containing `path`, its deepest existing folder
/// resolved first.
fn containing(store: &dyn LocalState, path: &Path) -> Option<Project> {
    store.project_containing(&canonical(path)).ok().flatten()
}

/// The project the lock of the working tree at `root` lives in: the one
/// containing it, else, for a tree holding projects, such as a monorepo
/// whose services are the projects, the one of them first by root, so
/// every agent meets on one lock.
fn lock_project(store: &dyn LocalState, root: &Path) -> Option<Project> {
    containing(store, root).or_else(|| {
        store
            .projects()
            .ok()?
            .into_iter()
            .filter(|project| project.root().as_ref().starts_with(root))
            .min_by(|one, other| one.root().as_ref().cmp(other.root().as_ref()))
    })
}

/// The checkout at `root`, its lock living in `project`, with that
/// project's live locks and todos; none when they cannot be read.
fn checkout_scope(
    store: &dyn LocalState,
    root: PathBuf,
    project: Project,
) -> Option<CheckoutScope> {
    let lock = checkout_lock(&root)?;
    let locks = store.locks(project.root()).ok()?;
    let todos = store.todos(project.root()).ok()?;
    Some(
        CheckoutScope::builder()
            .root(root)
            .lock(lock)
            .project(project)
            .locks(locks)
            .todos(todos)
            .build(),
    )
}

/// `path` with its deepest existing folder resolved, so a file not written
/// yet, or one reached through a symlink, is matched to its project.
fn canonical(path: &Path) -> PathBuf {
    path.ancestors()
        .find_map(|ancestor| {
            let resolved = ancestor.canonicalize().ok()?;
            let rest = path.strip_prefix(ancestor).ok()?;
            Some(resolved.join(rest))
        })
        .unwrap_or_else(|| path.to_path_buf())
}

/// The root of the git working tree holding `dir`, or none outside one,
/// when git cannot run, or past `deadline`, when git is killed.
fn repo_root(dir: &Path, deadline: Instant) -> Option<PathBuf> {
    if Instant::now() >= deadline {
        return None;
    }
    let mut command = Command::new(GIT);
    command
        .arg(GIT_DIR_FLAG)
        .arg(dir)
        .args(SHOW_TOPLEVEL)
        .envs(GIT_QUIET_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    for name in GIT_LOCATION_ENV {
        command.env_remove(name);
    }
    let mut child = command.spawn().ok()?;
    loop {
        if child.try_wait().ok()?.is_some() {
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        thread::sleep(GIT_POLL);
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        return None;
    }
    let mut root = output.stdout;
    if root.last() == Some(&LINE_END) {
        root.pop();
    }
    (!root.is_empty()).then(|| PathBuf::from(OsString::from_vec(root)))
}
