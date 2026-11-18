use getset::Getters;
use muster_domain::{
    coordination::{AgentId, Author, ClaimState, LOCK_TTL_MAX, Lock, Todo, TodoStatus},
    project::Project,
    value::EpochSeconds,
};
use typed_builder::TypedBuilder;

use crate::cli::mcp::{
    context::session_context::{bounded, credit},
    guard::{checkout::Checkout, verdict::Verdict},
};

/// The most bytes a project's name or a folder takes in a guard message.
const PATH_MAX_BYTES: usize = 600;
/// The most bytes another agent's todo title takes in a guard message.
const TITLE_MAX_BYTES: usize = 300;
/// Opens every refusal: why the guard holds the call.
const DENY_LEAD: &str = "muster holds this call until the work is announced, so every agent \
    sees who is doing what and no two agents change one checkout.";
/// Says no todo of this agent's is in progress, before the project's name.
const NO_TODO: &str = "No todo is in progress under you in the muster project";
/// How to get one, between which the project's root is named.
const CLAIM_ONE: [&str; 4] = [
    "Claim the one for this work: todo_claim(project=",
    ", id=<its id from todo_list>). Or add it and claim it: todo_add(project=",
    ", title=<the repo, the branch and what you are doing>), then todo_claim(project=",
    ", id=<the id todo_add returned>).",
];
/// Opens the line naming a checkout.
const THE_CHECKOUT: &str = "The checkout";
/// Says the checkout's lock is free, and how to take it.
const TAKE_LOCK: [&str; 5] = [
    "needs its lock before git changes or publishes it: lock_acquire(project=",
    ", name=",
    ", note=",
    ", ttl_s=",
    "). Each guarded git command renews it; lock_release frees it when you are done.",
];
/// Says another agent holds the checkout, before its holder's credit.
const HELD_BY: &str = "is held";
/// Follows what the holder's lease has left.
const LEFT: &str = "left,";
/// Opens the holder's in-progress todo.
const WORKING_ON: &str = "working on";
/// Said when the holder has no todo in progress.
const NO_HOLDER_TODO: &str = "with no todo in progress";
/// How to proceed while another agent holds the checkout, before its lock.
const WORK_ELSEWHERE: &str = ". Work in a worktree of your own (git worktree add) or wait. \
    If that agent is gone, a person can free it with `muster lock release";
/// Closes the command freeing a lock.
const COMMAND_CLOSE: &str = "`.";
/// Closes every refusal: how a person goes past it.
const DENY_TAIL: &str = "A person who wants this done anyway can run it themselves, which no \
    hook checks.";
/// The reminder after a push or a pull request, around the project's root
/// and the todo's id.
const REMINDER: [&str; 3] = [
    "That published the branch. Put the branch or pull request link on your in-progress todo \
     with todo_update(project=",
    ", id=",
    ", title=<its title with the link>), so todo_list shows where the work is, and write a note \
     with note_write if the next agent should know something.",
];
/// Stands for the id of a todo the agent has yet to claim.
const YOUR_TODO_ID: &str = "<your todo's id>";
/// Separates the sentences of a message.
const SENTENCE_BREAK: &str = " ";

/// What the guard knows of one project when an agent calls a tool: who is
/// calling, the project, its todos and live locks, and the moment.
#[derive(Clone, Copy, Debug, Getters, TypedBuilder)]
#[getset(get = "pub")]
pub struct GuardFacts<'a> {
    /// The calling agent's id.
    me: &'a AgentId,
    /// The project the call runs in.
    project: &'a Project,
    /// Its todos.
    todos: &'a [Todo],
    /// Its live locks.
    locks: &'a [Lock],
    /// When the call is made.
    now: EpochSeconds,
}

impl GuardFacts<'_> {
    /// What to answer a call before it runs: it needs a todo the agent holds
    /// in progress, and a call into `checkout` also needs the checkout's
    /// lock. Held by the agent, the lock is renewed; held by another, the
    /// call is refused naming who; otherwise a refusal says what to call.
    pub fn verdict(&self, checkout: Option<&Checkout>) -> Verdict {
        let mut fixes = Vec::new();
        if self.held_todo(|owner| self.me.is(owner)).is_none() {
            fixes.push(self.claim_fix());
        }
        let mut renew = None;
        if let Some(checkout) = checkout {
            match self
                .locks
                .iter()
                .find(|lock| lock.name() == checkout.lock())
            {
                Some(lock) if self.me.is(lock.owner()) => renew = Some(lock.clone()),
                Some(lock) => return self.deny(&[self.held_fix(checkout, lock)]),
                None => fixes.push(self.lock_fix(checkout)),
            }
        }
        if !fixes.is_empty() {
            return self.deny(&fixes);
        }
        renew.map_or(Verdict::Allow, Verdict::Renew)
    }

    /// The reminder after the agent published a branch: link it on its
    /// in-progress todo and leave a note.
    pub fn reminder(&self) -> String {
        let id = self.held_todo(|owner| self.me.is(owner)).map_or_else(
            || YOUR_TODO_ID.to_owned(),
            |todo| bounded(todo.id().as_ref(), PATH_MAX_BYTES),
        );
        let root = self.root();
        format!("{}{root}{}{id}{}", REMINDER[0], REMINDER[1], REMINDER[2])
    }

    /// The first todo in progress whose live claim's owner `is_owner`
    /// accepts.
    fn held_todo(&self, is_owner: impl Fn(&Author) -> bool) -> Option<&Todo> {
        self.todos.iter().find(|todo| {
            todo.effective_status(self.now) == TodoStatus::InProgress
                && matches!(todo.claim_state(self.now), ClaimState::Held { owner, .. } if is_owner(owner))
        })
    }

    /// The project's root, quoted.
    fn root(&self) -> String {
        bounded(
            &self.project.root().as_ref().display().to_string(),
            PATH_MAX_BYTES,
        )
    }

    /// How to announce the work: claim or add a todo.
    fn claim_fix(&self) -> String {
        let root = self.root();
        format!(
            "{NO_TODO} {}. {}{root}{}{root}{}{root}{}",
            bounded(self.project.name().as_ref(), PATH_MAX_BYTES),
            CLAIM_ONE[0],
            CLAIM_ONE[1],
            CLAIM_ONE[2],
            CLAIM_ONE[3],
        )
    }

    /// How to take the free lock of `checkout`.
    fn lock_fix(&self, checkout: &Checkout) -> String {
        let folder = bounded(&checkout.root().display().to_string(), PATH_MAX_BYTES);
        format!(
            "{THE_CHECKOUT} {folder} {}{}{}{}{}{folder}{}{}{}",
            TAKE_LOCK[0],
            self.root(),
            TAKE_LOCK[1],
            bounded(checkout.lock().as_ref(), PATH_MAX_BYTES),
            TAKE_LOCK[2],
            TAKE_LOCK[3],
            LOCK_TTL_MAX.as_secs(),
            TAKE_LOCK[4],
        )
    }

    /// Who holds `checkout` through `lock`, for how long, on what, and how
    /// to proceed.
    fn held_fix(&self, checkout: &Checkout, lock: &Lock) -> String {
        let doing = self.held_todo(|owner| owner == lock.owner()).map_or_else(
            || NO_HOLDER_TODO.to_owned(),
            |todo| {
                format!(
                    "{WORKING_ON} {}",
                    bounded(todo.title().as_ref(), TITLE_MAX_BYTES)
                )
            },
        );
        format!(
            "{THE_CHECKOUT} {} {HELD_BY} {}, {} {LEFT} {doing}{WORK_ELSEWHERE} {}{COMMAND_CLOSE}",
            bounded(&checkout.root().display().to_string(), PATH_MAX_BYTES),
            credit(lock.owner()),
            humantime::format_duration(lock.expires_in(self.now)),
            lock.name(),
        )
    }

    /// The refusal holding `fixes`, between why and how to go past it.
    fn deny(&self, fixes: &[String]) -> Verdict {
        let mut reason = vec![DENY_LEAD.to_owned()];
        reason.extend_from_slice(fixes);
        reason.push(DENY_TAIL.to_owned());
        Verdict::Deny(reason.join(SENTENCE_BREAK))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use muster_domain::{
        config::ImportMode,
        coordination::{LockHolder, LockTtl, NewTodoTitle, TodoDeps, TodoLease},
        port::{CoordinationStore, ProjectStore, StackStore},
        value::ProjectRoot,
    };
    use muster_os::clock::epoch_now;
    use muster_store::{
        SqliteStore,
        test_support::{fresh_store, spec},
    };
    use tempfile::TempDir;

    use super::*;

    /// The id of the agent calling in these tests.
    const ME: &str = "herdr-pane-1";
    /// The label another agent writes under.
    const OTHER: &str = "claude-code@herdr-pane-2";
    /// The checkout these tests guard.
    const REPO: &str = "/work/shop";

    /// A store holding one registered project, `shop`, and its scratch.
    fn shop() -> (SqliteStore, Project, TempDir) {
        let (store, scratch) = fresh_store();
        let dir = scratch.path().join("shop");
        fs::create_dir_all(&dir).unwrap();
        let root = ProjectRoot::try_new(dir.canonicalize().unwrap()).unwrap();
        StackStore::import(&store, &root, &[spec("web", "true")], ImportMode::Strict).unwrap();
        let project = store.project_containing(root.as_ref()).unwrap().unwrap();
        (store, project, scratch)
    }

    /// Adds the todo `title` and claims it as `label`.
    fn claimed(store: &SqliteStore, project: &Project, label: &str, title: &str) -> Todo {
        let author = Author::agent(label);
        let todo = store
            .add_todo(
                project.root(),
                &author,
                &NewTodoTitle::try_new(title).unwrap(),
                &TodoDeps::try_new(Vec::new()).unwrap(),
            )
            .unwrap();
        store
            .claim_todo(project.root(), &author, Some(todo.id()), TodoLease::DEFAULT)
            .unwrap();
        todo
    }

    /// Takes the lock of the checkout at [`REPO`] as `label`.
    fn lock_checkout(store: &SqliteStore, project: &Project, label: &str) {
        store
            .acquire_lock(
                project.root(),
                &LockHolder::fresh(label),
                checkout().lock(),
                LockTtl::DEFAULT,
                None,
            )
            .unwrap();
    }

    /// The checkout at [`REPO`].
    fn checkout() -> Checkout {
        Checkout::at(Path::new(REPO)).unwrap()
    }

    /// What the guard answers [`ME`] in `project` for a call into
    /// `checkout`, from the store's current todos and locks.
    fn verdict(store: &SqliteStore, project: &Project, checkout: Option<&Checkout>) -> Verdict {
        let me = AgentId::named(Some(ME), &|_| None).unwrap();
        let (todos, locks) = (
            store.todos(project.root()).unwrap(),
            store.locks(project.root()).unwrap(),
        );
        GuardFacts::builder()
            .me(&me)
            .project(project)
            .todos(&todos)
            .locks(&locks)
            .now(epoch_now())
            .build()
            .verdict(checkout)
    }

    /// The reason of a refusal, failing on any other verdict.
    fn denied(verdict: Verdict) -> String {
        match verdict {
            Verdict::Deny(reason) => reason,
            other => panic!("not denied: {other:?}"),
        }
    }

    /// Without a todo of its own in progress an edit is refused with the
    /// calls that announce it, naming the project; another agent's todo
    /// does not count.
    #[test]
    fn an_edit_without_an_own_todo_is_refused_with_the_fix() {
        let (store, project, _scratch) = shop();
        claimed(&store, &project, OTHER, "their work");

        let reason = denied(verdict(&store, &project, None));

        let root = project.root().as_ref().display().to_string();
        for call in [
            format!("todo_claim(project=\"{root}\", id=<its id"),
            format!("todo_add(project=\"{root}\", title="),
        ] {
            assert!(reason.contains(&call), "{reason}");
        }
        assert!(reason.contains("\"shop\""), "{reason}");
        assert!(reason.ends_with(DENY_TAIL), "{reason}");
    }

    /// An own in-progress todo lets an edit run, under any client name.
    #[test]
    fn an_edit_with_an_own_todo_runs() {
        let (store, project, _scratch) = shop();
        claimed(&store, &project, &format!("codex@{ME}"), "my work");

        assert_eq!(verdict(&store, &project, None), Verdict::Allow);
    }

    /// A free checkout is refused with the exact lock_acquire, and a missing
    /// todo is asked for in the same refusal.
    #[test]
    fn a_free_checkout_is_refused_with_the_lock_to_take() {
        let (store, project, _scratch) = shop();

        let reason = denied(verdict(&store, &project, Some(&checkout())));

        let root = project.root().as_ref().display().to_string();
        let acquire = format!(
            "lock_acquire(project=\"{root}\", name=\"{}\", note=\"{REPO}\", ttl_s={})",
            checkout().lock(),
            LOCK_TTL_MAX.as_secs()
        );
        assert!(reason.contains(&acquire), "{reason}");
        assert!(reason.contains("todo_add("), "{reason}");
    }

    /// A checkout another agent holds is refused naming the holder, the
    /// time left, its todo and how to free it.
    #[test]
    fn a_checkout_another_holds_is_refused_naming_them() {
        let (store, project, _scratch) = shop();
        claimed(&store, &project, ME, "my work");
        claimed(&store, &project, OTHER, "fix the login");
        lock_checkout(&store, &project, OTHER);

        let reason = denied(verdict(&store, &project, Some(&checkout())));

        assert!(
            reason.contains(&format!("is held by \"{OTHER}\", ")),
            "{reason}"
        );
        assert!(
            reason.contains(" left, working on \"fix the login\""),
            "{reason}"
        );
        assert!(
            reason.contains(&format!("`muster lock release {}`", checkout().lock())),
            "{reason}"
        );
        assert!(!reason.contains("lock_acquire("), "{reason}");
    }

    /// A checkout this agent holds, with its todo, runs and renews that lock
    /// through the session that took it.
    #[test]
    fn an_own_checkout_runs_and_renews() {
        let (store, project, _scratch) = shop();
        claimed(&store, &project, ME, "my work");
        lock_checkout(&store, &project, ME);

        let Verdict::Renew(lock) = verdict(&store, &project, Some(&checkout())) else {
            panic!("not renewed");
        };
        assert_eq!(lock.name(), checkout().lock());
        assert_eq!(lock.owner(), &Author::agent(ME));
    }

    /// The reminder names the agent's own todo and the calls to make.
    #[test]
    fn the_reminder_names_the_own_todo() {
        let (store, project, _scratch) = shop();
        let todo = claimed(&store, &project, ME, "my work");
        let me = AgentId::named(Some(ME), &|_| None).unwrap();
        let todos = store.todos(project.root()).unwrap();

        let reminder = GuardFacts::builder()
            .me(&me)
            .project(&project)
            .todos(&todos)
            .locks(&[])
            .now(epoch_now())
            .build()
            .reminder();

        assert!(
            reminder.contains(&format!(", id=\"{}\"", todo.id().as_ref())),
            "{reminder}"
        );
        assert!(reminder.contains("note_write"), "{reminder}");
    }
}
