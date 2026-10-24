# References cluster

Use the upstream Herdr checkout to keep easy-herdr current while preserving the fork's own
behavior. Run these commands from the easy-herdr repository root on Linux or macOS:

```bash
make references-mainline
make references-cluster
```

`references-mainline` initializes and updates `references/herdr` to upstream `master` and prints
its SHA. It retains Git history for comparisons. `references-cluster` uses the current reference
checkout without updating it; you can run it again against the same revision.

The cluster asks for two agent CLI names: `claude` by default for planner, executor and reviewer1,
and `codex` for reviewer2. Enter a command name, including a supported shell function or alias,
without arguments. Install `easy-herdr` on `PATH` first. The first agent stays in your terminal;
the other three open in tabs or windows in the same directory's switch session. Only the planner
receives the initial prompt. Existing `just` recipes remain the build and test entry points.

## Identify the four roles

The planner names the new cluster agents so it can assign work to the correct panes.

1. Read the repository's `AGENTS.md` and run `easy-herdr agent list`. Your own pane is
   `$HERDR_PANE_ID`; name it with `easy-herdr agent rename <pane-id> planner`.
2. Identify the three other agents launched by this cluster. Rename the other two using the
   planner's command to `executor` and `reviewer1`, and the agent using the second command to
   `reviewer2`.
3. The list reports agent kinds rather than the exact launch commands. If the kinds coincide,
   existing agents or role names make identification ambiguous, or fewer than four agents
   started, ask the user before assigning work. Do not rename unrelated agents.

## Compare upstream with the fork

The reference checkout is evidence for changes, not a replacement for easy-herdr's source tree.

Record the following in `.tmp/references-cluster/plan.md` before planning changes:

```bash
git rev-parse HEAD
git status --short
git ls-tree HEAD references/herdr
git -C references/herdr rev-parse HEAD
git -C references/herdr status --short
```

If the submodule is uninitialized, initialize its recorded revision with
`git submodule update --init -- references/herdr`. If it has local edits, ask the user to resolve
them before using it as upstream evidence. Keep the selected reference revision fixed throughout
the run. A reference SHA alone does not prove its changes are already implemented in the fork.

Compare upstream source, tests and build tooling with their counterparts in this repository.
Include applicable fixes, features and compatibility changes supported by the reference code.
For each finding, cite the upstream SHA, file and line, describe the current fork behavior, and
mark the proposed change as adopt, adapt or skip with a reason. Do not merge upstream wholesale.

Preserve these fork behaviors when adapting upstream changes:

- The `easy-herdr` binary, `herdr` install/uninstall behavior, native-terminal switch mode,
  cluster command and agent shortcuts.
- The easy-herdr config/state directories and config migration, disabled binary self-update
  and background version checks, and existing agent-detection manifest update policy.
- Fork packaging, release workflows and four-part release numbering; the Cargo version keeps
  the three-part upstream base version. Keep the fork's READMEs rather than replacing them
  with upstream copies.

Follow `AGENTS.md` for runtime/client boundaries, endpoint compatibility, platform isolation,
vendored patches, detection smoke tests and release documentation. User-facing unreleased docs
belong in `docs/next`; normal follow-up work must not edit its curated changelog, published docs,
release channel files or the stable Herdr skill.

## Implement and review

The planner coordinates changes; the executor edits project files and both reviewers independently
check the result against the plan and upstream evidence.

```text
Role       | Analyze       | Implement       | Review        | Finish
-----------|---------------|-----------------|---------------|-----------
planner    | evidence/plan | dispatch        | collect/fix   | summary
executor   |               | patch/check     | fix           |
reviewer1  |               |                 | read/review   |
reviewer2  |               |                 | read/review   |
```

Working notes go in `.tmp/references-cluster/`, which is already git-ignored.

1. planner: write a numbered plan in `plan.md` with evidence, protected fork behavior and
   acceptance checks for each item. Record existing work and check failures before editing.
2. planner: send one item at a time with
   `easy-herdr agent prompt executor "<item and acceptance checks>" --wait`.
   The executor follows the project's test-first rules where applicable and runs the smallest
   relevant `just` checks after each batch. Only the executor edits project files; the planner
   and reviewers may write their own working notes.
3. planner: dispatch reviews to reviewer1 and reviewer2 together using
   `easy-herdr agent prompt <reviewer-name> "<review instructions>" --wait`.
   Each reviewer reads `git diff`, new untracked project files, `plan.md` and the reference
   checkout, and writes `review-reviewer1.md` or `review-reviewer2.md`.
4. planner: reconcile findings and send actionable fixes to the executor. Repeat implementation
   and both reviews until neither reviewer has a blocking finding. Keep statuses consistent:
   `[ ]` pending, `[v]` handled and passed, `[x]` handled but not passed.
5. executor: run `just check` on the final result. Report any failed or unavailable check with
   its exact cause; do not weaken checks or claim completion while required checks remain open.
6. planner: write `summary.md` with the reference SHA, adopted/adapted/skipped items, upstream
   evidence, validation results and remaining blockers, then report to the user.

Do not commit, push, publish releases or create/manage worktrees. Leave startup and approval
dialogs to the user. Confirm command syntax with `easy-herdr --help` and the relevant subcommand's
help before using it. If a prompt stalls or times out, inspect the agent state before retrying
so you do not submit the same task twice.
