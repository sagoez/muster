//! Whether the guard refuses a call, and the reason it gives: pure, from
//! the todos and locks the call's projects hold.

use muster_domain::{
    coordination::{
        AgentId, Author, ClaimState, LOCK_NOTE_MAX_CHARS, LOCK_TTL_MAX, Lock, TODO_LEASE_MAX, Todo,
        TodoStatus,
    },
    project::Project,
    value::EpochSeconds,
};

use crate::cli::mcp::{
    guard::{checkout_scope::CheckoutScope, todo_scope::TodoScope},
    quoted::{PROJECT_NAME_MAX_BYTES, ROOT_MAX_BYTES, bounded, credit},
};

/// The most bytes a todo id takes in a refusal.
const ID_MAX_BYTES: usize = 80;
/// Opens every refusal: why the guard holds the call.
const DENY_LEAD: &str = "muster's plugin holds this call until the work is announced, so every \
    agent sees who is doing what and no two agents change one checkout.";
/// Closes every refusal: how a person goes past it.
const DENY_TAIL: &str = "A person who wants this done anyway can run it themselves, which no \
    hook checks.";
/// Opens the way out of a refusal that outlives a claim, before the id the
/// hook sees.
const OTHER_ID_LEAD: &str = "If you already hold the todo asked for and are still refused, this \
    hook sees you as";
/// Says the server writes under another id, before its flag naming one.
const OTHER_ID_CAUSE: &str = "while muster's MCP server writes under another id, as one started \
    with";
/// Says what the user changes, before the flag again.
const OTHER_ID_FIX: &str = "does. Stop and ask the user to start that server without";
/// Between dropping the flag and setting it to the hook's id.
const OTHER_ID_OR: &str = "or with";
/// Closes the way out.
const OTHER_ID_TAIL: &str = "and to restart the session.";
/// The MCP server's flag naming the agent.
const AS_FLAG: &str = "--as";
/// Separates the sentences of a refusal.
const SENTENCE_BREAK: &str = " ";
/// Says no todo of this agent's is in progress, before the project's name.
const NO_TODO: &str = "No todo is in progress under you in the muster project";
/// Asks for the todo of the work, before the call claiming it.
const CLAIM_THE_ONE: &str = "Claim the one for this work:";
/// Offers adding the todo, before the call adding it.
const OR_ADD: &str = "Or add it:";
/// Joins adding a todo to claiming it.
const THEN: &str = ", then";
/// Opens a lapsed claim, before the todo's id.
const LAPSED_LEAD: &str = "Your claim on the todo";
/// Between the todo's id and its project's name.
const IN_PROJECT: &str = "in the muster project";
/// Says the claim lapsed, before the call taking it again.
const LAPSED_TAIL: &str = "lapsed. Claim it again:";
/// Opens a sentence about a checkout, before its root.
const THE_CHECKOUT: &str = "The checkout";
/// Says the checkout's lock is free, before the call taking it.
const NEEDS_LOCK: &str = "needs its lock before git changes or publishes it:";
/// Follows the call taking a lock.
const RELEASE_LATER: &str = "lock_release frees it when you are done.";
/// Says another agent holds the checkout, before its holder's credit.
const HELD: &str = "is held";
/// Follows what the holder's lease has left.
const LEFT: &str = "left,";
/// Before the id of the holder's in-progress todo.
const ON_TODO: &str = "on the todo";
/// Where the holder's todo is read in full.
const SEE_TODO_LIST: &str = "(todo_list shows it).";
/// Said when the holder has no todo in progress.
const NO_HOLDER_TODO: &str = "with no todo in progress.";
/// How to proceed while another agent holds the checkout.
const WORK_ELSEWHERE: &str = "Work in a worktree of your own (git worktree add) or wait; if \
    that agent is gone, ask the user to free its lock.";
/// Ends a sentence that ends with a call.
const FULL_STOP: &str = ".";
/// The tool claiming a todo.
const TODO_CLAIM: &str = "todo_claim";
/// The tool adding a todo.
const TODO_ADD: &str = "todo_add";
/// The tool taking a lock.
const LOCK_ACQUIRE: &str = "lock_acquire";
/// The argument naming a project.
const PROJECT_ARG: &str = "project";
/// The argument naming a todo.
const ID_ARG: &str = "id";
/// The argument giving a todo's lease.
const LEASE_ARG: &str = "lease_s";
/// The argument giving a todo's title.
const TITLE_ARG: &str = "title";
/// The argument naming a lock.
const NAME_ARG: &str = "name";
/// The argument giving a lock's note.
const NOTE_ARG: &str = "note";
/// The argument giving a lock's time-to-live.
const TTL_ARG: &str = "ttl_s";
/// Stands for the id of a todo `todo_list` shows.
const LISTED_ID: &str = "<its id from todo_list>";
/// Stands for the id `todo_add` returns.
const ADDED_ID: &str = "<the id todo_add returned>";
/// Stands for the title of a new todo.
const NEW_TITLE: &str = "<the repo, the branch and what you are doing>";
/// Opens a call's arguments.
const CALL_OPEN: &str = "(";
/// Closes a call's arguments.
const CALL_CLOSE: &str = ")";
/// Joins an argument's name to its value.
const ARG_IS: &str = "=";
/// Separates a call's arguments.
const ARG_SEPARATOR: &str = ", ";

/// The reason to refuse a call `me` makes at `now`, or none to let it run.
/// It needs a todo `me` holds in progress in each of `todo_scopes`, and the
/// lock of each of `checkouts`. Every missing piece is named with the call
/// that supplies it; a lock another agent holds names that agent and the id
/// of its in-progress todo, never anything it wrote.
pub fn refusal(
    me: &AgentId,
    todo_scopes: &[TodoScope],
    checkouts: &[CheckoutScope],
    now: EpochSeconds,
) -> Option<String> {
    let mut fixes: Vec<String> = todo_scopes
        .iter()
        .filter_map(|scope| todo_fix(me, scope, now))
        .collect();
    if !fixes.is_empty() {
        let id = bounded(me.as_ref(), ID_MAX_BYTES);
        fixes.push(format!(
            "{OTHER_ID_LEAD} {id} {OTHER_ID_CAUSE} {AS_FLAG} {OTHER_ID_FIX} {AS_FLAG}, \
             {OTHER_ID_OR} {AS_FLAG} {id}, {OTHER_ID_TAIL}"
        ));
    }
    fixes.extend(
        checkouts
            .iter()
            .filter_map(|checkout| checkout_fix(me, checkout, now)),
    );
    if fixes.is_empty() {
        return None;
    }
    Some(
        [DENY_LEAD.to_owned()]
            .into_iter()
            .chain(fixes)
            .chain([DENY_TAIL.to_owned()])
            .collect::<Vec<_>>()
            .join(SENTENCE_BREAK),
    )
}

/// How `me` gets a todo in progress in `scope`'s project at `now`: claim
/// again one whose claim lapsed, or claim or add one; none when it holds
/// one.
fn todo_fix(me: &AgentId, scope: &TodoScope, now: EpochSeconds) -> Option<String> {
    let project = scope.project();
    if held_todo(scope.todos(), now, |owner| me.is(owner)).is_some() {
        return None;
    }
    let root = root_of(project);
    let name = bounded(project.name().as_ref(), PROJECT_NAME_MAX_BYTES);
    let lease = TODO_LEASE_MAX.as_secs().to_string();
    let lapsed = scope
        .todos()
        .iter()
        .find(|todo| matches!(todo.claim_state(now), ClaimState::Stale { owner } if me.is(owner)));
    if let Some(todo) = lapsed {
        let id = bounded(todo.id().as_ref(), ID_MAX_BYTES);
        let claim = call(TODO_CLAIM, &[
            (PROJECT_ARG, &root),
            (ID_ARG, &id),
            (LEASE_ARG, &lease),
        ]);
        return Some(format!(
            "{LAPSED_LEAD} {id} {IN_PROJECT} {name} {LAPSED_TAIL} {claim}{FULL_STOP}"
        ));
    }
    let claim = call(TODO_CLAIM, &[
        (PROJECT_ARG, &root),
        (ID_ARG, LISTED_ID),
        (LEASE_ARG, &lease),
    ]);
    let add = call(TODO_ADD, &[(PROJECT_ARG, &root), (TITLE_ARG, NEW_TITLE)]);
    let claim_added = call(TODO_CLAIM, &[
        (PROJECT_ARG, &root),
        (ID_ARG, ADDED_ID),
        (LEASE_ARG, &lease),
    ]);
    Some(format!(
        "{NO_TODO} {name}. {CLAIM_THE_ONE} {claim}. {OR_ADD} {add}{THEN} {claim_added}{FULL_STOP}"
    ))
}

/// What keeps `me` out of `checkout` at `now`: its free lock, with the call
/// taking it, or the agent holding it; none when `me` holds it.
fn checkout_fix(me: &AgentId, checkout: &CheckoutScope, now: EpochSeconds) -> Option<String> {
    let folder = bounded(&checkout.root().display().to_string(), ROOT_MAX_BYTES);
    match checkout
        .locks()
        .iter()
        .find(|lock| lock.name() == checkout.lock())
    {
        Some(lock) if me.is(lock.owner()) => None,
        Some(lock) => Some(held_by(checkout, lock, &folder, now)),
        None => {
            let note = bounded(&checkout.root().display().to_string(), LOCK_NOTE_MAX_CHARS);
            let acquire = call(LOCK_ACQUIRE, &[
                (PROJECT_ARG, &root_of(checkout.project())),
                (NAME_ARG, &bounded(checkout.lock().as_ref(), ROOT_MAX_BYTES)),
                (NOTE_ARG, &note),
                (TTL_ARG, &LOCK_TTL_MAX.as_secs().to_string()),
            ]);
            Some(format!(
                "{THE_CHECKOUT} {folder} {NEEDS_LOCK} {acquire}{FULL_STOP} {RELEASE_LATER}"
            ))
        },
    }
}

/// Who holds `checkout` through `lock` at `now`, for how long and on which
/// todo, and how to proceed.
fn held_by(checkout: &CheckoutScope, lock: &Lock, folder: &str, now: EpochSeconds) -> String {
    let holder = AgentId::of(lock.owner());
    let doing = held_todo(checkout.todos(), now, |owner| {
        holder.as_ref().is_some_and(|holder| holder.is(owner))
    })
    .map_or_else(
        || NO_HOLDER_TODO.to_owned(),
        |todo| {
            format!(
                "{ON_TODO} {} {SEE_TODO_LIST}",
                bounded(todo.id().as_ref(), ID_MAX_BYTES)
            )
        },
    );
    format!(
        "{THE_CHECKOUT} {folder} {HELD} {}, {} {LEFT} {doing} {WORK_ELSEWHERE}",
        credit(lock.owner()),
        humantime::format_duration(lock.expires_in(now)),
    )
}

/// The first of `todos` in progress at `now` whose live claim's owner
/// `is_owner` accepts.
fn held_todo(
    todos: &[Todo],
    now: EpochSeconds,
    is_owner: impl Fn(&Author) -> bool,
) -> Option<&Todo> {
    todos.iter().find(|todo| {
        todo.effective_status(now) == TodoStatus::InProgress
            && matches!(todo.claim_state(now), ClaimState::Held { owner, .. } if is_owner(owner))
    })
}

/// `project`'s root, quoted.
fn root_of(project: &Project) -> String {
    bounded(
        &project.root().as_ref().display().to_string(),
        ROOT_MAX_BYTES,
    )
}

/// The call of `tool` with `args`, as an agent writes it.
fn call(tool: &str, args: &[(&str, &str)]) -> String {
    let args: Vec<String> = args
        .iter()
        .map(|(name, value)| format!("{name}{ARG_IS}{value}"))
        .collect();
    format!("{tool}{CALL_OPEN}{}{CALL_CLOSE}", args.join(ARG_SEPARATOR))
}

#[cfg(test)]
mod tests {
    use std::{
        path::{Path, PathBuf},
        slice,
    };

    use muster_domain::{
        coordination::{TodoClaim, TodoId, TodoLease, TodoTitle},
        value::{ProjectName, ProjectRoot},
    };

    use super::*;
    use crate::cli::mcp::guard::checkout_scope::checkout_lock;

    /// The id of the agent calling in these tests.
    const ME: &str = "herdr-pane-1";
    /// The label another agent writes under.
    const OTHER: &str = "claude-code@herdr-pane-2";
    /// The session's project folder.
    const SHOP: &str = "/work/shop";
    /// Another project's folder, holding its own checkout.
    const API: &str = "/work/api";
    /// The moment every test runs at.
    const NOW: u64 = 1_000_000;
    /// Seconds a lapsed claim ran out before [`NOW`].
    const LAPSED_FOR: u64 = 60;

    /// The calling agent.
    fn me() -> AgentId {
        AgentId::named(Some(ME), &|_| None).unwrap()
    }

    /// The moment every test runs at.
    fn now() -> EpochSeconds {
        EpochSeconds::new(NOW)
    }

    /// The project `name` rooted at `root`.
    fn project(name: &str, root: &str) -> Project {
        Project::builder()
            .root(ProjectRoot::try_new(PathBuf::from(root)).unwrap())
            .name(ProjectName::try_new(name).unwrap())
            .build()
    }

    /// The todo `id` claimed by `label` until `until` seconds past the
    /// epoch.
    fn claimed(id: &str, label: &str, until: u64) -> Todo {
        let owner = Author::agent(label);
        let taken_at = EpochSeconds::new(until - TodoLease::DEFAULT.as_ref().as_secs());
        Todo::builder()
            .id(TodoId::try_new(id).unwrap())
            .title(TodoTitle::try_new(format!("work {id}")).unwrap())
            .status(TodoStatus::InProgress)
            .deps(Vec::new())
            .claim(Some(TodoClaim::fresh(&owner, TodoLease::DEFAULT, taken_at)))
            .author(owner)
            .updated_at(taken_at)
            .build()
    }

    /// The todo `id` claimed by `label`, live at [`now`].
    fn live(id: &str, label: &str) -> Todo {
        claimed(id, label, NOW + LAPSED_FOR)
    }

    /// `project`'s todos to need one of.
    fn scope(project: Project, todos: Vec<Todo>) -> TodoScope {
        TodoScope::builder().project(project).todos(todos).build()
    }

    /// The checkout at `root`, its lock living in `project`, held by
    /// `holder` when one is named, with `todos` in that project.
    fn checkout(
        root: &str,
        project: Project,
        holder: Option<&str>,
        todos: Vec<Todo>,
    ) -> CheckoutScope {
        let lock = checkout_lock(Path::new(root)).unwrap();
        let locks = holder
            .map(|label| {
                Lock::builder()
                    .name(lock.clone())
                    .owner(Author::agent(label))
                    .note(None)
                    .expires_at(EpochSeconds::new(NOW + LAPSED_FOR))
                    .build()
            })
            .into_iter()
            .collect();
        CheckoutScope::builder()
            .root(PathBuf::from(root))
            .lock(lock)
            .project(project)
            .locks(locks)
            .todos(todos)
            .build()
    }

    /// The refusal of a call with `todo_scopes` and `checkouts`, failing
    /// when there is none.
    fn refused(todo_scopes: &[TodoScope], checkouts: &[CheckoutScope]) -> String {
        refusal(&me(), todo_scopes, checkouts, now()).unwrap_or_else(|| panic!("not refused"))
    }

    /// Without a todo of its own in progress a call is refused with the
    /// calls that announce it, at the longest lease; another agent's todo
    /// does not count, and an own todo under another client name does.
    #[test]
    fn a_call_needs_an_own_todo_in_progress() {
        let reason = refused(&[scope(project("shop", SHOP), vec![live("t1", OTHER)])], &[
        ]);
        let lease = TODO_LEASE_MAX.as_secs();
        for expected in [
            format!("todo_claim(project=\"{SHOP}\", id=<its id from todo_list>, lease_s={lease})"),
            format!("todo_add(project=\"{SHOP}\", title="),
            "the muster project \"shop\"".to_owned(),
            format!("this hook sees you as \"{ME}\""),
            format!("without --as, or with --as \"{ME}\", and to restart the session"),
        ] {
            assert!(reason.contains(&expected), "{expected}: {reason}");
        }
        assert!(reason.ends_with(DENY_TAIL), "{reason}");
        let two = refused(
            &[
                scope(project("shop", SHOP), Vec::new()),
                scope(project("api", API), Vec::new()),
            ],
            &[],
        );
        assert_eq!(two.matches(OTHER_ID_LEAD).count(), 1, "said once: {two}");

        let mine = scope(project("shop", SHOP), vec![live(
            "t1",
            &format!("codex@{ME}"),
        )]);
        assert_eq!(refusal(&me(), &[mine], &[], now()), None);
    }

    /// A claim that lapsed is named, with the call taking it again.
    #[test]
    fn a_lapsed_claim_is_named_with_the_call_renewing_it() {
        let lapsed = claimed("t9", ME, NOW - LAPSED_FOR);
        let reason = refused(&[scope(project("shop", SHOP), vec![lapsed])], &[]);
        assert!(reason.contains("Your claim on the todo \"t9\""), "{reason}");
        assert!(
            reason.contains(&format!(
                "todo_claim(project=\"{SHOP}\", id=\"t9\", lease_s={})",
                TODO_LEASE_MAX.as_secs()
            )),
            "{reason}"
        );
        assert!(!reason.contains("todo_add("), "{reason}");
    }

    /// A free checkout is refused with the exact lock_acquire in the
    /// project its lock lives in; one this agent holds needs nothing more.
    #[test]
    fn a_free_checkout_needs_its_lock_in_its_own_project() {
        let todos = scope(project("shop", SHOP), vec![live("t1", ME)]);
        let api = checkout(API, project("api", API), None, Vec::new());
        let reason = refused(slice::from_ref(&todos), slice::from_ref(&api));
        let acquire = format!(
            "lock_acquire(project=\"{API}\", name=\"{}\", note=\"{API}\", ttl_s={})",
            api.lock(),
            LOCK_TTL_MAX.as_secs()
        );
        assert!(reason.contains(&acquire), "{reason}");
        assert!(!reason.contains("todo_claim("), "{reason}");

        let held = checkout(
            API,
            project("api", API),
            Some(&format!("codex@{ME}")),
            Vec::new(),
        );
        assert_eq!(refusal(&me(), &[todos], &[held], now()), None);
    }

    /// A checkout another agent holds is refused naming that agent, the
    /// time left and only the id of its todo, never its title; a missing
    /// own todo is named too.
    #[test]
    fn a_checkout_another_holds_names_them_and_their_todo_id() {
        let theirs = live("t2", "codex@herdr-pane-2");
        let api = checkout(API, project("api", API), Some(OTHER), vec![theirs]);
        let reason = refused(&[scope(project("shop", SHOP), Vec::new())], &[api]);
        assert!(
            reason.contains(&format!("is held by \"{OTHER}\", ")),
            "{reason}"
        );
        assert!(
            reason.contains(" left, on the todo \"t2\" (todo_list shows it)."),
            "{reason}"
        );
        assert!(!reason.contains("work t2"), "{reason}");
        assert!(!reason.contains("lock_acquire("), "{reason}");
        assert!(!reason.contains("muster lock release"), "{reason}");
        assert!(reason.contains("todo_add("), "{reason}");
    }

    /// Every checkout of a call is checked, not only the first.
    #[test]
    fn every_checkout_is_checked() {
        let todos = scope(project("shop", SHOP), vec![live("t1", ME)]);
        let mine = checkout(SHOP, project("shop", SHOP), Some(ME), Vec::new());
        let theirs = checkout(API, project("api", API), Some(OTHER), Vec::new());
        let reason = refused(&[todos], &[mine, theirs]);
        assert!(reason.contains(&format!("\"{API}\" is held")), "{reason}");
        assert!(!reason.contains(&format!("\"{SHOP}\" is held")), "{reason}");
    }

    /// The suggested lock note fits a lock note's bound however long the
    /// root.
    #[test]
    fn the_suggested_note_fits_a_lock_note() {
        let long = format!("/work/{}", "x".repeat(LOCK_NOTE_MAX_CHARS * 2));
        let todos = scope(project("shop", SHOP), vec![live("t1", ME)]);
        let reason = refused(&[todos], &[checkout(
            &long,
            project("api", API),
            None,
            Vec::new(),
        )]);
        let note = reason
            .split("note=")
            .nth(1)
            .and_then(|rest| rest.split(", ttl_s=").next())
            .unwrap();
        assert!(note.chars().count() <= LOCK_NOTE_MAX_CHARS, "{note}");
    }
}
