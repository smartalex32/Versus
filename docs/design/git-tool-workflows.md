# Git diff and merge workflows

Status: implemented. The final confirmation explicitly requires
existing GUI-launched Diff and Merge workflows to retain their current behavior.

## Requested outcome

Versus should work as a configured Git diff tool and interactive Git merge tool.
These are alternative entry workflows that reuse existing comparison, editing,
navigation, and save capabilities, with views focused on the invocation's needs.
When several files need merging, Git invokes Versus separately for each file.
The user chose this standard flow rather than an application-managed multi-file
merge session.

## Current behavior and integration constraints

- The existing [CLI](../../src/cli.rs) accepts two sources with `--diff` or
  `--folder`; [startup](../../src/main.rs) keeps the process alive for the window's
  lifetime. [Git setup](../../README.md#git) already documents difftool invocation,
  including directory comparison with copied inputs.
- These launches use the normal desktop workspace, including the ability to
  switch to editable Merge. A focused Git-only view is not currently implemented.
- The [glossary](../../GLOSSARY.md) distinguishes normal desktop Merge, with two
  independent working copies, from the noninteractive base/ours/theirs CLI merge.
  The existing `--merge` command writes output and exits; it is not an interactive
  conflict-resolution window.
- [Git difftool](https://git-scm.com/docs/git-difftool) supplies comparison inputs;
  its directory mode supplies temporary directory trees. Temporary paths are not
  necessarily useful user-facing labels.
- [Git mergetool](https://git-scm.com/docs/git-mergetool) invokes the configured
  tool for individual conflicted files and supplies Base (when available), Local,
  Remote, and the result destination. A unified multi-file session needs additional
  coordination beyond accepting one invocation's paths. Git can use the tool's
  exit status to determine whether the resolution succeeded.
- With trusted exit codes, Git stages a successful result. On failure it restores
  its saved pre-tool copy of the working-tree file, so writing a draft to the
  result path and then cancelling does not preserve that draft. Git's success
  path does not independently reject remaining conflict markers. Verified against
  the installed Git 2.55.0 `git-mergetool` script and the
  [upstream script](https://github.com/git/git/blob/v2.55.0/git-mergetool.sh).
- The ordinary Merge header already labels the left source Incoming and the
  right source Current in [source controls](../../src/app/source_controls.rs).
  Git Merge will follow that convention: Incoming (Git Remote), Result, Current
  (Git Local). These are input roles; the UI must not infer branch names from them.
- [InteractiveDocument](../../src/core/interactive.rs) initializes undo/redo as
  empty when loading text. Git supplies snapshots rather than previous editing
  history. Reversing a change already present at launch needs an operation on
  its content, distinct from undoing an action performed in the current window.
- Normal Merge arrows select a source changed run or expanded line, then insert
  it at the other pane's caret. Diff identifies the source range but does not map
  it to a destination replacement range. The toolbar's Accept All replaces the
  whole destination. See [editor controls](../../src/app/interactive_view/editor.rs)
  and [transfer actions](../../src/app/interactive_view/editing_actions.rs).

## Reuse map

| Existing capability | Reuse opportunity and boundary |
| --- | --- |
| [Workspace composition](../../src/app/workspace.rs), [File Diff](../../src/app/file_diff.rs), [readonly editor](../../src/app/interactive_view/readonly.rs) | Existing comparison rendering and navigation can support focused Git views. Launch-specific controls and labels need an explicit presentation context. |
| [Retained documents](../../src/app/interactive_view/document.rs) | Existing document identity and editing history can support the editable result. The active editor currently models a pair, not Base/Local/Remote/Result roles. |
| [Working-file saves](../../src/core/work_files.rs), [save actions](../../src/app/interactive_view/editing_actions.rs) | Encoding preservation and external-change checks are useful. Existing side-save targets the working copy's path; a Git result destination must be represented explicitly. |
| [Folder comparison](../../src/app/folder_diff.rs) | Existing directory comparison and navigation can support Git directory Diff. Folder Merge retention and bulk saves are outside the selected per-file Git Merge flow. |
| [Three-way merge engine](../../src/core/merge.rs) | Existing noninteractive merging supplies algorithms to assess for change regions. Git's current working-tree result is authoritative at startup; recomputing a merge must not replace it. |

## Decision tree

Questions within a round are independent. Child decisions wait until their parent
choices and relevant repository facts are settled. Recommendations are proposals,
not accepted decisions.

### Round 1: workflow boundaries

| Question | Decision | Status |
| --- | --- | --- |
| Q1: What does multi-file merge mean? | Use Git's standard per-file invocation. No repository-wide merge session is required. | Accepted |
| Q2: What is editable in Git Merge? | One editable Result, with original Base and both source versions as read-only references. | Accepted; naming clarified in Q4 |
| Q3: What appears in Git Diff? | Fixed comparison sources, useful labels, difference navigation, search, and comparison options; a file list for directory comparisons. | Accepted |

### Round 2: result and presentation

| Question | Decision | Status |
| --- | --- | --- |
| Q4: What does the editable Base-like pane contain initially? | Label it Result and load Git's existing partially merged file, preserving completed merges and saved manual work; retain the original Base as a read-only reference. Show already merged changes so the user can reverse them. | Accepted; reversal controls specified in Q7 |
| Q5: How are merge panes arranged? | Incoming left, Result center, Current (Local) right. The original Base remains available as a reference. This matches the existing manual Merge labels and side ordering. | Accepted; source verified |
| Q6: How does the user finish a file? | Save and Resolve writes the Result and returns success for Git to stage; closing early offers continued editing or cancellation. Preserving an unfinished draft after closing is outside this workflow. | Accepted |

The [Git ownership ADR](../adr/0001-git-owns-merge-sequencing.md) records the
boundary between per-file editing in Versus and sequencing/staging in Git.

### Round 3: reviewing and resolving changes

| Question | Decision | Status |
| --- | --- | --- |
| Q7: How can earlier merges be reversed? | Add a top-level Restore Base button that resets the whole Result to original Base. Keep the initial partial merge visible in the middle, and allow taking Incoming or Current for already merged changes using the comparison. | Accepted |
| Q8: How are conflicts resolved? | Reuse normal Merge's source-arrow interactions, targeting only the editable middle Result. Keep direct editing and explicit manual resolution. Save and Resolve requires no unresolved conflicts. | Accepted; exact arrow destination behavior is Q9 |

Restore Base changes only the in-memory Result until Save and Resolve, and is one
undoable operation using the existing editing-history conventions. It does not
modify either source input. When the caller explicitly supplies no Base, the
button is unavailable with an explanation. A supplied empty Base is valid and
restores an empty Result; an empty file alone cannot prove the ancestor is missing.

### Round 4: arrow destination

| Question | Decision | Status |
| --- | --- | --- |
| Q9: What does a Git Merge arrow change? | Replace the corresponding changed region in Result, including its conflict-marker wrapper when applicable, using the familiar arrow controls. Ordinary Merge keeps its insertion-at-caret behavior. | Accepted |

This choice matters for both unresolved conflicts and changes already merged at
startup: insertion can duplicate text, while replacement needs an explicit
mapping between the selected source region and the current Result. Existing
normal Merge does not provide that destination mapping.

## Result interaction contract

Git Merge opens Incoming on the left, Result in the center, and Current (Local)
on the right. Result initially contains the exact partial merge supplied by Git.
Source-arrow controls point inward and replace the corresponding Result region.
Choosing a source for a complete conflict removes its marker wrapper and records
that conflict as resolved. The same region replacement works for changes already
merged before launch. Each replacement is undoable.

The user can edit Result directly. Manually edited conflicts need an explicit
Mark Resolved action; removing text or marker lines alone does not silently
confirm the user's intent. Both source versions can be retained using an explicit
both-sides choice with Incoming-first or Current-first ordering. Ordinary arrows
always choose one side, so clicking opposite arrows successively replaces the
choice rather than appending duplicate content.

The original Base is available for reference, and a top-level Restore Base button
replaces the entire Result with its contents as one undoable action. Source files
stay read-only. Result text and its resolution state participate together in
Undo/Redo. The initial view shows prior merged changes as well as unresolved
conflicts; a change accepted identically on both sides must not disappear merely
because Incoming and Current match each other. Base-relative changes provide
that context when Base is available.

Save and Resolve is the successful completion action. It requires all known
conflicts to be resolved and the Result to save successfully. The process remains
alive while the window is open. Closing without completing offers continued
editing or cancellation; cancellation returns failure, following Git's own
restoration behavior. Draft persistence after closing is outside this workflow.

## Acceptance scenarios

- Opening a file preserves Git's entire partial Result, including completed
  merges and any manual edits already saved before launch. Full Incoming and
  Current inputs remain available; documented Git configuration disables
  `mergetool.versus.hideResolved` so earlier changes remain reviewable.
- The left source is Incoming, the middle is editable Result, and the right
  source is Current (Local). Source files and original Base are read-only.
- Previously merged changes remain visible and can be changed again. The
  unresolved-conflict count is distinct from the count of all changed regions.
- Restore Base resets the whole in-memory Result in one undoable action. Undo
  restores the preceding Result and associated conflict-resolution state. Reset
  does not silently declare original conflicts resolved merely by removing markers.
- Source transfers and manual edits affect Result. Undo/Redo covers this
  invocation's actions; earlier operation history is not fabricated.
- An arrow replaces the mapped Result region rather than inserting at the caret.
  Source additions and deletions, different-length replacements, and edits made
  after a comparison must preserve the surrounding Result. A stale mapping cannot
  authorize a replacement; it must be refreshed before the action applies.
- Choosing either source for an unresolved conflict removes that conflict's
  wrapper. Taking both preserves the explicitly chosen ordering. Manual resolution
  is explicit, and Undo/Redo restores both text and resolved/unresolved state.
- Save and Resolve cannot complete while known conflicts are unresolved. A failed
  write keeps the window open with the edits retained and reports the failure.
  A successful save closes with success for Git; cancellation reports failure.
- Git advances through conflicted files. Versus does not discover or stage a
  repository-wide list itself.
- Git Diff keeps fixed sources and read-only content, supports empty sides for
  additions/deletions, and uses the existing directory comparison for directory
  input. Normal comparison navigation and relevant options remain available.

## Compatibility and engineering defaults

These defaults follow the existing app's constraints and preserve established
workflows; they do not add a repository-management interface.

- Add explicit Git launch contexts so fixed-source Git views do not change normal
  desktop launches or repurpose the existing noninteractive `--merge` command.
  Native paths, spaces, dash-prefixed names, and caller-relative paths remain
  supported. Labels use the supplied logical filename and role rather than relying
  on Git's temporary filenames; revision names are shown only when supplied.
- Retain the existing 32 MiB text limit and bounded comparison work. The working
  file loader supports UTF-8 with optional BOM and BOM-marked UTF-16 LE/BE;
  direct file comparison currently uses UTF-8. Reuse the appropriate current
  capabilities and report unsupported input clearly instead of silently decoding
  it incorrectly. The result retains its original encoding and line endings.
- Explicitly absent inputs are distinct from paths that fail to load. Support
  absent comparison sides and a missing common ancestor, and reject malformed or
  unavailable required merge inputs without reporting resolution success.
- Existing non-regular-file and symlink restrictions remain. Git directory Diff
  configuration uses copied temporary inputs (`--no-symlinks`). Binary/oversized
  input gets an explanation; it does not become editable text or a successful merge.
- Reuse exact-byte and destination-identity checks before writing Result. An
  external edit or failed write must not be overwritten or reported as success.
  Keep edits in the open window when saving fails.
- Do not infer conflict resolution merely from Incoming and Current differing,
  or infer success merely from an absence of marker-looking text. Preserve
  preexisting manual resolutions, identify actual outstanding conflict regions,
  and keep malformed or unrecognized conflict data from silently completing.
- Document Git configuration with trusted merge exit codes and full source
  snapshots. Configuration is provided for the user to apply; implementation work
  does not silently change their global Git settings.
- Retain Windows and Linux as the supported release targets. Local macOS tests
  provide development evidence; supported-platform packaging/native checks must
  be reported separately.

## Implementation boundaries and validation

1. Build a UI-independent Git merge model that owns immutable source snapshots,
   the authoritative editable Result, change-region mappings, conflict state, and
   undoable actions. Reuse existing comparison and document-history primitives.
   Keep Git region replacement distinct from normal two-way caret insertion.
2. Add CLI parsing and launch/completion state for focused Git Diff and Git Merge.
   Git owns repository sequencing and staging; Versus handles one invocation's
   inputs, editing, saving, and completion status.
3. Compose focused views from existing pane rendering, comparison navigation,
   editor, theme, and save capabilities. Extract shared capabilities where useful;
   avoid copying the whole normal workspace or routing Git behavior through
   implicit global flags.
4. Add focused tests under `tests/` for region replacement, conflict resolution,
   Restore Base, history, encoding/endings, absent inputs, external changes,
   stale comparison results, CLI parsing, UI controls, and process exit behavior.
   Add disposable-repository Git protocol checks for success/staging and
   cancellation/restoration, in addition to headless UI tests.
5. Update the README's Git setup, SRS, architecture/capability routing, and test
   documentation with delivered behavior. Run focused checks, the full existing
   suite, formatting, diff checks, and the offline release build. Report platform
   checks that ran and any that remain unverified.

The shared understanding was confirmed. Git entry points use separate app state
and views; ordinary GUI session transitions, controls, and insertion-at-caret
merge actions remain unchanged. Existing regression suites are part of validation.
