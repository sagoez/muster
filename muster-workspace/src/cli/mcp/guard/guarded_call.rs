use std::{
    ffi::{OsStr, OsString},
    os::unix::ffi::OsStringExt,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use getset::{CopyGetters, Getters};
use muster_client::resolve_project;
use muster_domain::{coordination::Todo, port::LocalState, project::Project};
use typed_builder::TypedBuilder;

use crate::cli::mcp::guard::{
    call_scopes::CallScopes,
    checkout_scope::{CheckoutScope, checkout_lock},
    hook::{hook_input::HookInput, hook_tool::HookTool},
    memory_file::{claude_folder_name, holds_shared_fact, memory_folder, names_a_memory_folder},
    shell::shell_scan::{GIT, GIT_DIR_FLAG},
    todo_scope::TodoScope,
};

/// How long all the gits finding checkouts' roots may run together before
/// the one still running is killed, inside the guard's own deadline.
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

/// One tool call the guard judges against muster's database: what it
/// touches, and the projects, todos and locks that bear on it. It only
/// reads.
#[derive(Getters, CopyGetters, TypedBuilder)]
pub struct GuardedCall<'a> {
    /// The database, read only.
    #[getset(get_copy = "pub")]
    store: &'a dyn LocalState,
    /// The tool called.
    #[getset(get_copy = "pub")]
    tool: HookTool,
    /// The call as the agent sent it.
    #[getset(get_copy = "pub")]
    input: &'a HookInput,
    /// What it touches, as [`HookTool::touched`] finds it.
    #[getset(get = "pub")]
    touched: Vec<Option<PathBuf>>,
    /// The folder `MUSTER_PROJECT` names.
    #[getset(get_copy = "pub")]
    env_project: Option<&'a Path>,
}

impl GuardedCall<'_> {
    /// The project the calling session runs in, if any.
    pub fn session(&self) -> Option<Project> {
        resolve_project(None, self.env_project(), self.input().cwd(), self.store())
            .ok()
            .and_then(|target| target.within(self.store()).ok())
    }

    /// The project whose notes should hold the fact about the work this
    /// call, Claude Code's write or edit, leaves in a memory file under
    /// `claude_dir`: the project that memory folder is named for, else the
    /// session's. None when it leaves no such fact, or when neither is a
    /// registered project.
    pub fn shared_fact_project(&self, claude_dir: &Path) -> Option<Project> {
        if !self.tool().writes_memory() {
            return None;
        }
        self.touched().iter().flatten().find_map(|path| {
            let folder = memory_folder_of(path, claude_dir)?;
            if !self.writes_shared_fact(path) {
                return None;
            }
            self.project_named(&folder).or_else(|| self.session())
        })
    }

    /// What it needs before it runs: a todo in progress in each project it
    /// touches, with their open todos, and the checkouts' locks; none when
    /// the database cannot be read.
    pub fn scopes(&self) -> Option<CallScopes> {
        let (projects, checkouts) = if self.tool().edits() {
            (self.edited_projects(), Vec::new())
        } else {
            self.shell_scopes()
        };
        Some(
            CallScopes::builder()
                .todo_scopes(self.todo_scopes(projects)?)
                .checkouts(checkouts)
                .build(),
        )
    }

    /// Whether the file at `path` declares a shared memory type once this
    /// call lands.
    fn writes_shared_fact(&self, path: &Path) -> bool {
        self.tool()
            .written(self.input().tool_input(), path)
            .is_some_and(|text| holds_shared_fact(&text))
    }

    /// The registered project Claude Code keeps memory for under `folder`.
    fn project_named(&self, folder: &OsStr) -> Option<Project> {
        self.store()
            .projects()
            .ok()?
            .into_iter()
            .find(|project| OsStr::new(&claude_folder_name(project.root().as_ref())) == folder)
    }

    /// The projects holding a file an edit writes.
    fn edited_projects(&self) -> Vec<Project> {
        self.touched()
            .iter()
            .flatten()
            .filter_map(|path| self.containing(path))
            .collect()
    }

    /// What a git or gh command changing or publishing a checkout needs: the
    /// checkout's lock and a todo, both in its project, the one containing
    /// its root, else the first registered project inside it by root; a
    /// checkout with neither is not held. A folder git cannot resolve, and a
    /// `gh pr` naming another repository, need the todo in the session's
    /// project.
    fn shell_scopes(&self) -> (Vec<Project>, Vec<CheckoutScope>) {
        let session = self.session();
        let deadline = Instant::now() + GIT_DEADLINE;
        let mut roots: Vec<PathBuf> = Vec::new();
        let mut unresolved = false;
        for dir in self.touched() {
            match dir.as_deref().and_then(|dir| repo_root(dir, deadline)) {
                Some(root) if !roots.contains(&root) => roots.push(root),
                Some(_) => {},
                None => unresolved = true,
            }
        }
        let checkouts: Vec<CheckoutScope> = roots
            .into_iter()
            .filter_map(|root| self.checkout_scope(root))
            .collect();
        let mut projects: Vec<Project> = checkouts
            .iter()
            .map(|checkout| checkout.project().clone())
            .collect();
        if unresolved {
            projects.extend(session);
        }
        (projects, checkouts)
    }

    /// Each of `projects` once, with its open todos.
    fn todo_scopes(&self, projects: Vec<Project>) -> Option<Vec<TodoScope>> {
        let mut scopes: Vec<TodoScope> = Vec::new();
        for project in projects {
            if scopes
                .iter()
                .all(|scope| scope.project().root() != project.root())
            {
                let todos = self.open_todos(&project)?;
                scopes.push(TodoScope::builder().project(project).todos(todos).build());
            }
        }
        Some(scopes)
    }

    /// The registered project containing `path`, its deepest existing folder
    /// resolved first.
    fn containing(&self, path: &Path) -> Option<Project> {
        self.store()
            .project_containing(&canonical(path))
            .ok()
            .flatten()
    }

    /// The project the lock of the working tree at `root` lives in: the one
    /// containing it, else, for a tree holding projects, such as a monorepo
    /// whose services are the projects, the one of them first by root, so
    /// every agent meets on one lock.
    fn lock_project(&self, root: &Path) -> Option<Project> {
        self.containing(root).or_else(|| {
            self.store()
                .projects()
                .ok()?
                .into_iter()
                .filter(|project| project.root().as_ref().starts_with(root))
                .min_by(|one, other| one.root().as_ref().cmp(other.root().as_ref()))
        })
    }

    /// The checkout at `root`, its lock living in its lock project, with
    /// that project's live locks and todos; none outside every project or
    /// when they cannot be read.
    fn checkout_scope(&self, root: PathBuf) -> Option<CheckoutScope> {
        let project = self.lock_project(&root)?;
        let lock = checkout_lock(&root)?;
        let locks = self.store().locks(project.root()).ok()?;
        let todos = self.open_todos(&project)?;
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

    /// Every todo of `project` not done, a row that does not read left out,
    /// so neither done work nor a broken row turns the guard off; none when
    /// the store cannot be read.
    fn open_todos(&self, project: &Project) -> Option<Vec<Todo>> {
        Some(
            self.store()
                .open_todos(project.root(), usize::MAX)
                .ok()?
                .into_iter()
                .filter_map(Result::ok)
                .collect(),
        )
    }
}

/// The project folder `path` keeps Claude Code memory for under
/// `claude_dir`: matched as named, else as both resolve, so a symlinked
/// config or memory folder is matched either way. The disk is read only for
/// a path through a folder named as a memory folder whose name alone does
/// not match.
fn memory_folder_of(path: &Path, claude_dir: &Path) -> Option<OsString> {
    if !names_a_memory_folder(path) {
        return None;
    }
    if let Some(folder) = memory_folder(path, claude_dir) {
        return Some(folder.to_owned());
    }
    let resolved = lexical(&canonical(path));
    memory_folder(&resolved, &canonical(claude_dir)).map(OsStr::to_owned)
}

/// `path` with each `..` taking back the folder before it, as a path that
/// does not exist yet resolves: [`canonical`] leaves one past its deepest
/// existing folder.
fn lexical(path: &Path) -> PathBuf {
    path.components().fold(PathBuf::new(), |mut read, part| {
        if part == Component::ParentDir {
            read.pop();
        } else {
            read.push(part);
        }
        read
    })
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
