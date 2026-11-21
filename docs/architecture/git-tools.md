# Git tool workflows

This guide covers the explicit Git difftool and mergetool entry points. It is
the detailed reference for the [README setup](../../README.md#git),
[requirements](../SRS.md#46-cli-and-external-tools), and [architecture map](../ARCHITECTURE.md).
The normal desktop launch and the noninteractive `versus --merge` command keep
their own behavior and owners.

## Choose the workflow

| Need | Command | Result |
| --- | --- | --- |
| Compare two paths in the normal workspace | `versus [--diff | --folder] LEFT RIGHT` | Existing desktop workspace; may switch between Diff and Merge. |
| Open Git's fixed comparison snapshots | `versus --git-diff [--diff | --folder] [--label TEXT] -- LEFT RIGHT` | Read-only focused Git Diff view. |
| Resolve one Git conflict | `versus --git-merge (--base BASE | --no-base) --local LOCAL --remote REMOTE --output MERGED [--label TEXT]` | Focused three-pane editor; successful save returns success to Git. |
| Calculate a three-way text merge and exit | `versus --merge BASE OURS THEIRS --output OUT [--overwrite]` | Existing noninteractive merge command; no window. |

All Git-specific paths are parsed as native OS paths. Quote values in shell
configuration so spaces survive; place Git Diff paths after `--` so a name that
starts with a dash stays a path. Relative paths resolve from Git's caller working
directory. The optional label is for the logical file or comparison name; Git's
temporary snapshot paths remain the actual inputs.

## Git Diff protocol

`--git-diff` takes two paths in left-to-right order: Git's pre-image is on the
left, and its post-image is on the right. File or folder mode is inferred when
neither `--diff` nor `--folder` is supplied. The window fixes both sources for
the invocation and provides read-only content, search, comparison options,
difference navigation, and a selectable file list for folder comparisons. Git
waits for the view to close before advancing to another file. Git's empty side
for an added or deleted file is shown as empty content.

Git's custom difftool command exposes `$LOCAL`, `$REMOTE`, `$MERGED`, and `$BASE`.
`$LOCAL` and `$REMOTE` are temporary snapshot files; `$MERGED` names the path
being compared, and `$BASE` is provided for compatibility with custom merge-tool
commands. The recommended configuration uses `$MERGED` as the label and selects
file/folder mode automatically. For `--dir-diff`, Git may create symlinks by
default; use `--no-symlinks` because Versus does not open symlink targets.

## Git Merge protocol

Git invokes the mergetool once for each conflicted file. Versus receives immutable
Base (when available), Local, Remote, and the current partially merged file at
`MERGED`. The current file is authoritative: it can contain cleanly combined
changes and prior manual resolutions. Versus preserves it exactly as the initial
Result and as the save baseline. `mergetool.versus.hideResolved=false` keeps full
Local and Remote snapshots available for reviewing changes already resolved by
Git.

The focused view lays out Incoming (Git Remote) on the left, editable Result in
the middle, and Current (Git Local) on the right. The original Base is a read-only
reference. Restore Base replaces all Result contents with Base in one undoable
action; if no Base was supplied, the button is unavailable. Source arrows replace
the selected mapped Result region, including a conflict's marker wrapper. A
separate explicit action can keep both source versions in either chosen order.
Manual edits to a known conflict require Mark Resolved. Undo and Redo track edits
made in this window, not chronology Git did not supply.

Save and Resolve requires the model to have no known unresolved conflicts, and
the protected save must succeed before the process returns success. The save
checks the destination identity and original bytes so external changes are not
silently overwritten. A failed save keeps the window open with its edits. Closing
offers Keep Editing or Cancel Merge; cancellation returns failure. Git then
restores its pre-tool result and controls the next file and staging. Versus does
not enumerate repository conflicts or stage files itself. The existing
noninteractive `--merge` command remains a separate operation.

For an add/add conflict with no stage-1 ancestor, Git may create an empty
placeholder Base path for the custom tool. That file is not enough to distinguish
an absent ancestor from a real empty ancestor. The recommended
[README configuration](../../README.md#git) checks `git cat-file -e ":1:$MERGED"`
and chooses `--no-base` only when that stage-1 object is missing;
otherwise it passes `--base "$BASE"`. Git 2.55's `git-mergetool` changes to the
worktree root before evaluating the configured command, so the stage query uses
the correct path even when Git was launched from a subdirectory. Quoting
`":1:$MERGED"` and each CLI operand preserves paths containing spaces. The UI
then explains that Base is unavailable and disables Restore Base. A real empty
Base remains valid and can be restored.

## Code ownership and data flow

| Question | Owner | Responsibility |
| --- | --- | --- |
| Which launch was requested? | [`src/cli.rs`](../../src/cli.rs), [`src/cli/git.rs`](../../src/cli/git.rs) | Ordinary and noninteractive launches stay in the parent; the Git parser handles explicit fixed-input requests. |
| How is a Git window kept alive? | [`src/git_launch.rs`](../../src/git_launch.rs), [`src/main.rs`](../../src/main.rs) | Runs the native window and returns failure by default; Git Merge records success only after a confirmed save. |
| What state does the Git UI own? | [`src/app/git.rs`](../../src/app/git.rs) | Separate eframe app, view dispatch, close confirmation, and completion status. Ordinary `VersusApp` sessions are not used as Git session state. |
| How does Git Diff load and display inputs? | [`src/app/git/diff.rs`](../../src/app/git/diff.rs), [`diff/`](../../src/app/git/diff/) | Parent owns state/jobs; `loading.rs` classifies and compares off-thread; `file.rs` renders and navigates rows; `folder.rs` searches the tree and opens files. |
| How does Git Merge load and save? | [`src/app/git/merge.rs`](../../src/app/git/merge.rs) | Loads Base/Incoming/Current/Result snapshots and asynchronously writes Result through the existing protected working-file save. |
| How do the panes act? | [`src/app/git/merge/view.rs`](../../src/app/git/merge/view.rs), [`view/`](../../src/app/git/merge/view/) | Parent owns presentation state; `toolbar.rs` owns controls, `panes.rs` owns editors and inline arrows, `navigation.rs` owns selection/history shortcuts, and `text.rs` owns shared highlighting and line coordinates. |
| How are regions and conflicts represented? | [`src/core/git_merge.rs`](../../src/core/git_merge.rs), [`git_merge/`](../../src/core/git_merge/) | Parent defines the model. `actions.rs` applies transactions; `mapping.rs` combines regions; `regions.rs` computes bounded alignments; `ledger.rs` preserves conflict identities; `conflicts.rs` parses markers; `history.rs` retains atomic snapshots. |

Keep region matching and conflict state in core so tests do not need egui. Keep
Git-specific mode and controls in `GitApp`; don't route fixed-input launches
through normal `Session` transitions. When changing a shared visual or save
primitive, preserve its ordinary GUI callers and regression tests. The app-level
Merge completion starts as failure; only a confirmed successful Result save can
make it zero. Closing read-only Git Diff advances Git, including while a comparison
is loading; a reported input/comparison error returns failure.

## Test routing

Unit test source lives under `tests/`, while small declarations register suites
under their owning Rust modules:

| Change | Test source | Focused command |
| --- | --- | --- |
| Git CLI parsing | [`tests/unit/cli/mod.rs`](../../tests/unit/cli/mod.rs) | `cargo test --locked --offline --bin versus cli::tests` |
| Git app lifecycle and view behavior | [`tests/unit/app/git/`](../../tests/unit/app/git/) | `cargo test --locked --offline --bin versus app::git` |
| Region mapping, conflict tracking, and history | [`tests/unit/core/git_merge/mod.rs`](../../tests/unit/core/git_merge/mod.rs) | `cargo test --locked --offline --lib core::git_merge::tests` |
| Public merge model contract | [`tests/git_merge.rs`](../../tests/git_merge.rs) | `cargo test --locked --offline --test git_merge` |
| Git's real merge-tool protocol | [`tests/git_protocol.rs`](../../tests/git_protocol.rs) | `cargo test --locked --offline --test git_protocol` |
| Ordinary GUI compatibility | [`tests/unit/app/operating_modes.rs`](../../tests/unit/app/operating_modes.rs), [`tests/unit/app/interactive_view/`](../../tests/unit/app/interactive_view/) | `cargo test --locked --offline --bin versus app::operating_mode_tests` and `cargo test --locked --offline --bin versus app::interactive_view::tests` |

For launch dispatch, view, or shared rendering changes, run the focused Git
suite and the relevant ordinary GUI suite. For a cross-capability change, follow
the broader validation sequence in [`AGENTS_PROJECT_REFERENCE.md`](../../AGENTS_PROJECT_REFERENCE.md).
