---
name: muster
description: How to work with muster. Use when starting any task, when you learn something worth keeping, when work touches another repository, when running or debugging dev processes, and when coordinating with other agents.
---

# Working with muster

muster runs each project's dev processes and holds what agents share: notes,
todos, locks and values, per registered project.

## Start every task

1. `note_read` and `todo_list`: what other agents found and what is left.
   Read them before searching the code or asking.
2. `project_list`: every registered project. Related repositories often live
   in other folders, each its own project.

## Announce work before you edit or run git

muster's plugin guards edits to a project's files, and git or gh commands
that change or publish a checkout (switch, checkout, commit, rebase, reset,
stash, merge, pull, push, cherry-pick, restore, revert, am, clean, rm, mv,
apply, `gh pr create`, `gh pr merge`, `gh pr checkout`). Dry runs and
help requests pass.

- An edit needs a todo you hold in progress in the project holding the
  file; one of those commands, in the project holding the checkout.
  `todo_claim` the todo for the work, or `todo_add` it, then `todo_claim` the
  id it returns, with a long `lease_s` so the claim outlasts the work. Name
  the repository and branch in its title.
- Those git commands also need the checkout's lock: `lock_acquire` with the
  `project` and `name` the refusal gives (`checkout-<folder>-<key>`), which
  may be another project than yours. Hold it for the work, not one command;
  nothing renews it for you, so take it again when it lapses, and
  `lock_release` it when you are done.
- A refusal is the fix: it names the exact call to make, and says when your
  claim lapsed. When another agent holds the checkout, it names that agent,
  the time left and its todo's id: work in a worktree of your own
  (`git worktree add`) or wait, and if that agent is gone, ask the user.
  Only a person clears another agent's lock or runs the command themselves.
- After `git push` or `gh pr create`, put the branch or pull request link on
  your todo with `todo_update`, and mark it `done` when the work lands.

## Keep what you learn where it belongs

Write a note (`note_write`) when you learn something the next agent would
otherwise rediscover. When it is about another project, pass `project` with
that project's name, from `project_list`, so its agents read it there. Every
note, todo, value, lock and process tool takes `project`.

## Run and debug dev processes

- `stack_status` shows what runs, with alerts for crashes since your last call.
- When something breaks: `process_logs` with `errors_only=true`, then again
  with `since` set to the cursor it returned, to read only new output.
- After a code change: `process_restart` with `wait=true`.
- `process_wait` waits for a log line, a port, an HTTP answer or a state.
  No call blocks long; a wait that has not finished returns a cursor to
  continue from.

## Locks or todos

- A todo is a unit of work others can see and pick up: claim it while you
  work it, its lease lapses if you vanish, and done finishes it.
- A lock guards a resource another agent must not touch meanwhile: a
  checkout for the length of the work, a database reset or a migration for
  just that step. Your locks go when your session ends.

## Which tools are loaded

`note_read`, `todo_list`, `stack_status`, `process_logs` and `project_list`
load at the start of a session. The rest (`todo_add`, `todo_claim`,
`todo_update`, `lock_acquire`, `lock_release`, `note_write`,
`process_restart`, `process_wait`, `kv_read`, `kv_write`, ...) load on demand
through tool search.
