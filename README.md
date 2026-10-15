# Versus

Versus is an offline engineering workspace for comparing files or folders and
merging text files. It uses operating-system filesystem paths, so
mapped drives and UNC paths on Windows and mounted network filesystems on Linux
work through the same local APIs as other paths. Versus does not implement network
authentication or make outbound network requests at runtime.

## Use

Choose **Folder Diff** or **File Diff** in the centered header to start a
fresh workspace. The adjacent **Diff / Merge** header toggle switches the current
workspace without reopening sources; the centered buttons become **Folder Merge**
and **File Merge** when Merge is selected. Diff is the default. Merge turns
the existing two panes into independent editable working copies; there is no third
Base file. Changes remain in memory until you explicitly save. Switching between
Diff and Merge preserves the open files and edits.

Merge does not need the diff engine. **Run Diff** is available in Merge mode and
starts off. While it is off, Versus does not compare the open files; edits,
selection transfers, chunks, and ignored ranges remain available. Turn it on for
difference highlighting, navigation, and destination suggestions within the
editors. Diff work is debounced for the open pair and can be cancelled. Turning
Run Diff off cancels pending work and clears highlighting.
The three comparison options (Show only differences, Ignore whitespace, Ignore
line endings) have separate settings in Diff and Merge. Enabling Run Diff for the
first time defaults all three to on; later toggles preserve your Merge choices
without changing Diff preferences. Show only differences hides unchanged lines
without removing them from the working copies. Turn the filter off to edit a
selection that spans hidden lines.

Run Diff sits in the header beside the theme button; when enabled, the refresh
icon appears immediately to its right. Merge keeps New comparison (+) on the left
of the button line, followed by a magnifying-glass **Find and Replace** icon in
folder workspaces. Zoom remains on the right. With Run Diff enabled, the comparison options appear immediately to
the right of zoom, followed by difference navigation. A change count appears
beside the left-side actions, using the same style as Diff. The legends are
centered below the panes with full icons and text, including Conflict in Merge.
The full legend stays visible regardless of Run Diff. It shares the footer with
**Save Both** and **Unsaved Changes** in Merge; entries wrap within that footer
when zoom or window width requires it. Turning Run Diff off hides the diff
controls and change count while retaining the Merge settings. Each file and folder
pane loads, expands, and scrolls independently; a single selected source opens immediately. Transfer
controls and file comparison assistance become available once both files are open.

With Run Diff enabled, Merge uses File Diff's row colors, status icons, changed-text
highlights, and line-ending badges. Transfer arrows are drawn toward the destination and use the source side's color
and sit in two bordered tracks between the file panes: left-to-right copies in
the left track, right-to-left copies in the right track. The line-expansion
buttons stay inside the file panes, separated from highlighted text.
Difference navigation subtly highlights the current changed line. One-sided
changes move only their own pane; paired changes scroll and highlight both
corresponding lines.
Switching to File Diff keeps one path header per side and provides Refresh, with
the same aligned comparison rows as a new File Diff. Edits and undo history remain
available when returning to Merge; Refresh compares the retained working copies.

Both panes show editable text, line numbers, and separate unsaved status. Select a
contiguous line range in either pane, choose an insertion point or explicitly
select a replacement range in the other pane, then use the transfer control to
copy it across. Transfers preserve the source text and are undoable. Versus groups
recognized code blocks and blank-line-separated paragraphs into chunks; these are
heuristics rather than a full language parser. You can create, split, combine, and
adjust manual chunks. Ignored line ranges are managed independently on each side.
They stay in the file and block transfers or replacement targets that would
overwrite protected lines; you can still type directly in an ignored range. The
automatic transfer and search replacement paths respect those protections.
The centered toolbar has **Accept All Left**, **Accept All Right**, **Undo**, and
**Redo** icons with tooltips. Accept All copies that side into the other as one
undoable operation, subject to protection. Automatic conflict merging belongs to
the separate three-way CLI workflow. Undo and Redo cover edits, transfers, chunk changes,
and ignored-range changes. They are also available in the toolbar;
Ctrl/Command+Z and Ctrl+Y or Command+Shift+Z work while editing.

Use the **Save Left** or **Save Right** disk icon immediately to the right of
each pane’s Browse button, or **Save Both** in the bottom row, to review a preview
and write the selected working copies. Save Both is enabled only when either
per-file Save is active, and previews only files ready to save. Versus checks that the files have not
changed on disk since they were opened before replacing them. Interactive text files are limited
to 32 MiB. UTF-8 with or without a BOM and UTF-16 little- and big-endian text retain
their encoding; original line endings are preserved where possible. Cancelled or
failed saves leave the working copies intact. Unsaved changes are marked
independently for each pane.

For folder pairs, Merge opens a lazy browser that enumerates directory entries and
basic metadata as folders are expanded. It does not compare file contents unless
Run Diff is enabled; files load when opened. Double-clicking a file opens only
that pane; the other pane can keep browsing a folder or editing a file. Each
file pane has a back arrow before its LEFT/RIGHT label to return to its own tree.
Open files independently on each side, including files with different names or
relative paths. Each file pair keeps its own undo history as you navigate or pair files again. Folder-wide **Find and
Replace** searches both trees, with plain text or regular
expressions, case and whole-word options. Results include file and line, and
replacements apply immediately as undoable batches to the working copies.
Batch undo is atomic across participating files; undo newer edits or ignored-range
changes in those files first. Binary, non-text, and files over 32 MiB are skipped.
Saving those changes remains explicit and uses the external-change check.
Find and Replace opens a compact dialog centered on the app; click outside,
press Escape, or use its close icon to dismiss it. The larger fields use muted,
italic placeholders. Choose **LEFT**, **RIGHT**, or **BOTH**; the side colors match
the pane labels. **Match Case**, **Match Whole Word**, and **Use Regular Expression**
are tooltip-equipped VS Code icons inside the Find field. Search starts after
250 ms without typing; button actions start immediately. Every search collects
both sides, and matches appear progressively in the right results
panel, which fills the right half by default with a centered empty-state hint.
A neutral **LEFT / RIGHT** toggle spans that panel’s width. Choosing **LEFT** or
**RIGHT** switches the results to that side, disables the opposite toggle, and
limits replacement to the chosen side. **BOTH** enables both toggles. Scope changes
reuse cached results immediately without another search. Rows show the line
number and matching text; hover over a row and use its **X** to exclude it from
replacement.
Undo, Redo, and Refresh sit beside the scope controls, with search status below.
**Refresh** repopulates excluded results and rereads files to include external
changes, while preserving unsaved working copies. The **Replace Next** and
**Replace All** icons beside the Replace field immediately replace the next
included match or all included matches in working copies, with no preview or Apply
step. Each action is one undoable batch. Remaining original matches advance after
each action,
so repeated Replace Next actions stay accurate when replacement lengths change.
Undo, Redo, Refresh, and search-option buttons start any needed search immediately;
the typing delay applies only to edits in Find.
With Run Diff enabled, folder statuses include staged working copies and ignored
ranges. The up/down controls jump to changed files and reveal their parent folders.

Each comparison mode has a folder or file icon and the active mode is highlighted,
with Folder Diff selected initially. Clicking a path area or browse icon in
either pane opens that mode's native folder or file picker directly. Selecting the
second source starts comparison automatically in Diff mode. File Diff opens
the line comparison without a Back button.

The **Recently compared** list appears when Versus opens and after **New
comparison**. Click a file or folder pair to recompare it without browsing again.
The list shows only the active comparison type, with left-aligned path pairs
separated by a bold **vs** and subtle alternating row backgrounds.
The 20 most recent pairs are saved locally per user, newest first; reopening a
pair moves it to the top. Paths and comparison type are saved, never file contents.
Unavailable or changed sources show an error and can be selected again.

Choosing another Folder/File header option starts a fresh comparison and cancels
pending work. Clicking the active mode preserves the current comparison; clicking
the Folder button while viewing a file from its tree returns to that tree. The **+**
button on the left of the toolbar has the tooltip **New comparison** and clears both selections,
results, and pending work while keeping the selected Folder/File comparison mode
and theme. Switching between Diff and Merge preserves the current session.

Both pane headers remain available for browsing in directly selected file views.
Merge paths stay on one line and trim the beginning with a leading ellipsis when
needed, keeping the folder or file name visible. The full path appears on hover.
Diff folder paths wrap; Diff file paths use the same leading ellipsis. LEFT/RIGHT
and browse stay centered beside the path.
The refresh icon at the upper right reloads the file comparison you are viewing,
including files opened from a folder tree. In the folder view it compares the same
folders again while preserving selection, expansion, and scroll position; a
selection is cleared only if the entry no longer exists. Local paths,
mapped drives, UNC shares, and mounted Linux paths all use the same filesystem
APIs; paths can be entered through the native dialog.

Click a folder in either tree to expand or collapse the corresponding relative
path in both. Scrolling is linked, and a dash marks the empty position opposite
a one-sided item so matching paths stay aligned. **Expand all** and **Collapse
all** are the stacked plus/minus icons at the upper right and control both trees. Hover
over an action icon for its label. Tab focuses tree rows; Enter or Space toggles a
folder, and the left/right arrow keys collapse/expand a focused folder. Up/Down
moves through visible rows while keeping the active pane and selected entry in
view. Keyboard focus has a distinct blue outline. Native screen readers receive
side, path, kind, status, and size labels for tree entries and numbered text for
visible file lines; focused file panes also support arrow keys, Page Up/Down,
and Home/End.

- Amber unequal sign: changed files or shared folders containing differences.
- Cyan left arrow: files and folders present only on the left.
- Violet right arrow: files and folders present only on the right.
- Red boxed cross: different entry types.
- Red warning triangle: a filesystem read error. Hover for details.

Identical files, folders, and lines use ordinary gray text without a status icon
or a legend entry. The legend is centered at the bottom with icons and text; the same icons replace
status words in each compact tree row. File sizes and recursive folder totals appear beside the status
icon, independently for each side, using binary units (KiB, MiB, and so on). Sizes
reflect filesystem metadata collected during the scan. Folder
totals count regular-file bytes and exclude symlink targets. Empty folders show
zero bytes; unknown sizes or incomplete totals show a dash. Hover for status and
error details. The cursor becomes a pointer over files and folders.

Comparison runs in the background with a refresh icon in the header and
cancellation beside progress.
Files are compared by content, not timestamps. Symlinks are compared by their
target paths rather than target contents and are never recursively followed.
This keeps traversal within the selected roots and avoids loops or repeated
comparisons when links point to ancestors or outside folders. Empty folders are
included. Changing a path clears the prior result so it cannot be mistaken for
the new selection.
The footer keeps updating guidance and completed-action messages on the left,
with the legend in the middle. Action progress appears only while work is running,
to the right of the legend in Diff and immediately left of **Save Both** in Merge.
Errors appear in that area as **1 error** or **N errors**; hover the summary to
read the details. Moving or clicking inside either pane dismisses the errors.

Desktop diagnostics are written to **versus.log** beside the executable (beside
the outer AppImage on Linux). The log records comparison progress, folder-cache
timing, paths, and failures without recording compared file contents. It rotates
at 5 MiB and keeps **versus.log.1** as the previous log. If that folder is not
writable, the status error area reports the logging failure. Comparing a folder
with itself is supported and finishes with zero changes; worker failures stop
the operation and appear in the same error area.
Long messages are shortened on screen; hover for the complete text. Measured
stages show percentage complete. Folder discovery and line alignment have no
reliable total and show activity instead. Comparisons can be cancelled beside the
progress status.
The change count and current difference position stay beside + in the toolbar.

The **+** button stays on the left of the toolbar. The other icon controls at the far right
apply to both comparison views:

- **Show only differences** hides identical lines and tree entries. File line
  numbers still refer to the original files; changed folder ancestors stay visible.
  Filtering preserves the top visible entry, or the nearest remaining entry.
  It starts on.
- **Ignore whitespace** ignores inline Unicode whitespace, including spaces and
  tabs, while preserving line boundaries. It starts on.
- **Ignore line endings** ignores CRLF, LF, CR, and final-terminator differences.
  It starts on. Turn it off to compare endings exactly; changed endings appear
  as badges beside their lines.
- **Previous difference** and **Next difference**, the up/down arrows, scroll
  both panes to the previous or next changed line or file. Folder navigation skips
  folders and expands the target file's parents, including when they were
  collapsed. Navigation stops at the first or last difference; the status area
  shows **Change N of M** for the current target.

Selected option icons have a blue outline. Changing an ignore option recomputes
the comparison in the background; filtering does not reread the sources. Options
remain selected when starting a new comparison or changing modes. Folder ignore
options apply to valid UTF-8 text up to 32 MiB; binary, invalid UTF-8, and larger
files retain byte-for-byte comparison.

Drag one file or folder from your file explorer into the large view on the left
or right, below the path bars, to replace that side. Empty panes have persistent
dashed outlines and drop hints. A blue border and drop label
identify LEFT or RIGHT above the pane contents, including the empty chooser view.
Windows follows the live cursor during native drags even when ordinary pointer
events are unavailable. The source type
selects the corresponding view automatically. Diff explains why mixed file/folder
pairs cannot compare; Merge opens each source independently. A drop in a file
opened from the folder tree keeps the other displayed side. In Diff, it starts
a direct comparison. Multiple-item drops are rejected
without replacing the current selection.
On Linux, Versus uses X11 or XWayland when `DISPLAY` is available so native
file drops work. Pure Wayland sessions retain browse selection; the windowing
backend does not support file drops there.

When comparing folders, double-click a regular file on either side to open a
read-only, line-by-line file comparison. The same legend colors and icons identify changed, left-only, and
right-only lines; line numbers and empty placeholders keep both sides aligned.
File headers use the same LEFT/RIGHT styling as folder headers. Long file paths
show a leading ellipsis so the filename remains visible; hover for the full path.
Changed text within a line has a stronger highlight in that line's status color;
unchanged text remains unhighlighted. Very long lines or exhausted highlight
processing time use a whole-line highlight to keep loading responsive.
Within a changed block, similar lines are aligned even when an added or removed
line shifts their original numbers. For example, an inserted left line appears
opposite an empty placeholder, and following code aligns with its commented-out
version on the right while the comment text remains highlighted as a change.
Similarity alignment uses bounded extra work; very large changed blocks or an
exhausted processing budget retain positional pairing.
In Diff, vertical scrolling stays linked, and each pane can scroll horizontally
for long lines. Merge panes scroll independently. The **back arrow**, on the left of the toolbar, returns to the folder view
with its selection, expansion, and scroll position preserved. Enter or Space on a
focused file also opens it.

Diff file contents load in the background. Missing files show an empty side;
binary or non-UTF-8 files, files above the 32 MiB text limit, and comparisons
exceeding the two-second diff processing limit show an explanation. Type mismatches
do not open folders or symlink targets. Read errors are shown in the file view;
Back remains available for files opened from a folder comparison. Line endings are
normalized when **Ignore line endings** is selected. Diff mode remains read-only;
only explicit Merge saves write working copies.

The sun/moon icon at the upper right switches between light and dark themes in
either view. The logo and window icon preserve the blue half and use a white half
in dark mode or the original dark half in light mode.

The toolbar places **Zoom** and its percentage immediately before adjacent
magnifying-glass **−** and **+** buttons, just left of **Show only differences**.
Click them to reduce or enlarge the interface. Hold
**Ctrl** and scroll the mouse wheel up/down to zoom in/out; the wheel alone
scrolls the comparison. Command + wheel also works on macOS. Use
Ctrl + / Ctrl - / Ctrl 0 (Command on macOS) for the keyboard shortcuts,
or click the percentage to reset to 100%. Zoom is restored across launches;
source paths and compared content are not saved. Row and control heights follow font metrics while staying compact at
the default size. Toolbar buttons highlight when pressed and activate on release;
releasing outside cancels the action.

## Git and IDE integration

Launch Versus with two paths to open their comparison immediately:

```sh
versus --diff -- "left file.txt" "right file.txt"
versus --folder -- "left folder" "right folder"
```

Without a mode flag, two paths select file or folder comparison automatically.
With no arguments, the normal empty workspace opens. Relative paths resolve from
the caller's working directory; quote paths containing spaces. `--` lets filenames
begin with a dash. `--diff` requires regular files and `--folder` requires folders;
unavailable or incompatible sources show an explanation in the window. Git's
`/dev/null` (and `NUL` on Windows) represents an empty file side for additions and
deletions. Other devices and symlink targets are not opened.

Versus runs in the foreground until its window closes, keeping Git/IDE temporary
inputs available. `--wait` is accepted for callers that supply it and has the same
behavior. Each invocation opens its own window. Closing normally returns zero,
including when files differ; this is a visual comparison, not a command-line
equality check. `--help` and `--version` print without opening a window. Invalid
arguments return 2; failure to start the window returns 1. Read failures appear
in the window. Save editor buffers first: Versus reads files from disk and never
writes back to them.

Use the installed executable on your PATH, or replace `versus` with its quoted
full path (`Versus.exe` on Windows or `Versus.AppImage` on Linux). In PowerShell,
invoke a quoted executable path with `&`:

```powershell
& 'C:\Tools\Versus.exe' --diff -- 'left file.txt' 'right file.txt'
```

### Git

Run these once in a shell (Git Bash on Windows). Omit `--global` to configure only
the current repository:

```sh
git config --global diff.tool versus
git config --global diff.guitool versus
git config --global difftool.versus.cmd 'versus --diff -- "$LOCAL" "$REMOTE"'
```

For an executable outside PATH, set the command with its full path instead:

```sh
git config --global difftool.versus.cmd '"C:/Program Files/Versus/Versus.exe" --diff -- "$LOCAL" "$REMOTE"'
```

Then compare working-tree changes, staged changes, or two revisions:

```sh
git difftool --no-prompt
git difftool --no-prompt --cached
git difftool --no-prompt HEAD~1 HEAD -- path/to/file.txt
```

Close each Versus window to advance to the next changed file. For a single folder
comparison, temporarily select folder mode and ask Git to copy working-tree
files instead of creating symlinks:

```sh
git -c difftool.versus.cmd='versus --folder -- "$LOCAL" "$REMOTE"' difftool --tool=versus --dir-diff --no-symlinks
```

The command receives Git's pre-image on the left and post-image on the right.
See the [Git difftool reference](https://git-scm.com/docs/git-difftool) for revision
and path options. These commands open the read-only Diff view.

For a conventional three-way merge, run `--merge` with the common base, ours,
theirs, and an output path. The command writes a merged text file with conflict
markers where both sides changed the same base section. Exit status is 0 when no
conflicts were found, 1 when conflict markers were written, and 2 when parsing,
reading, or writing failed. Existing output is rejected unless `--overwrite` is
passed:

```sh
versus --merge base.txt ours.txt theirs.txt --output merged.txt
```

### VS Code and other IDEs

Add the following tasks to your project's `.vscode/tasks.json` (merge them into
any existing tasks). Replace the first task's `command` with your executable path
if Versus is outside PATH. Open a saved file, then use **Tasks: Run Task** to
compare it with another file or view its Git changes in a Versus window. The Git
task uses the configuration above. Tasks require an open workspace folder.

```json
{
  "version": "2.0.0",
  "tasks": [
    {
      "label": "Versus: Compare active file",
      "type": "process",
      "command": "versus",
      "args": ["--diff", "--", "${file}", "${input:versusOtherFile}"],
      "options": { "cwd": "${workspaceFolder}" },
      "problemMatcher": []
    },
    {
      "label": "Versus: Git changes for active file",
      "type": "process",
      "command": "git",
      "args": ["difftool", "--tool=versus", "--no-prompt", "--", "${file}"],
      "options": { "cwd": "${fileDirname}" },
      "problemMatcher": []
    }
  ],
  "inputs": [
    {
      "id": "versusOtherFile",
      "type": "promptString",
      "description": "Other file path (absolute or relative to the workspace)"
    }
  ]
}
```

The process tasks pass paths as separate arguments, including spaces. See VS
Code's [external-tool tasks](https://code.visualstudio.com/docs/debugtest/tasks)
and [input variables](https://code.visualstudio.com/docs/reference/variables-reference).
For an IDE with an external diff-tool setting, choose the Versus executable and
set its arguments to `--diff -- <left-file> <right-file>`, substituting that IDE's
two filename placeholders. Enable waiting for the tool to exit if the IDE offers
that option.

## Releases

GitHub Actions builds two separate native Linux x86-64 distributions in both CI
and tagged releases:

| GitHub artifact | Build environment / glibc baseline | AppImage |
| --- | --- | --- |
| `linux-glibc-2.35-x86_64` | Ubuntu 22.04 / glibc 2.35 | `Versus-linux-x86_64-glibc-2.35.AppImage` |
| `linux-glibc-2.28-x86_64` | Rocky Linux 8.10 / glibc 2.28, automatic legacy X11 fallback | `Versus-linux-x86_64-glibc-2.28.AppImage` |

Each artifact also includes a correspondingly named `.tar.gz` containing the plain
executable, README and license. The AppImage bundles runtime libraries; the plain
executable requires its native libraries on the host. Linux filenames include the
baseline so both variants can coexist in a release without overwriting each other.

Check your system with `getconf GNU_LIBC_VERSION`. Use the 2.28 artifact on Rocky 8.
Download the artifact from a successful CI run, unzip it, and run:

```bash
chmod +x Versus-linux-x86_64-glibc-2.28.AppImage
./Versus-linux-x86_64-glibc-2.28.AppImage
```

For the normal 2.35 build, substitute `2.35` in the filename. If the host cannot
mount AppImages through FUSE, append `--appimage-extract-and-run`; this optional
fallback needs no FUSE installation or administrator access. Both builds run the
same app.
The 2.35 AppImage and plain executables use the host display and graphics stack.

The 2.28 package preserves Rocky’s original crypto libraries and their vendor
integrity files so the display helpers can perform FIPS self-tests. It leaves
host crypto policies unchanged. CI exercises vendor FIPS activation and integrity
checks; this is not a claim of FIPS certification.

The **2.28 AppImage automatically detects X11 displays without XInput2**, including
traditional X2Go/nxagent sessions. On those displays it starts a bundled private
display and software renderer within the existing desktop. Launch it with the
same command above; no additional installation, remote service, or launch option
is needed. Normal X11 and Wayland-only sessions retain the native launch path. The
AppImage also accepts `--compat-x11` to explicitly select the fallback, including
when used as a Git/IDE diff tool. `--help` and `--version` do not open a display.

Compatibility mode retains comparisons, navigation, highlighting, filtering,
browsing and zoom. It increases package size and may use more CPU for rendering.
Clipboard contents and file drops from the surrounding desktop are not bridged
into the private display; use Browse or command-line paths. Automatic selection
checks XInput2 support, not every possible graphics-driver incompatibility.

On a shared X11 host, Xephyr may print `server already running` while its
race-safe display allocator skips occupied display numbers. If the app opens,
these messages do not indicate a second Versus instance. A warning about the
owner of `/tmp/.X11-unix` concerns the host's shared socket directory; Versus
leaves its ownership and permissions unchanged.

The 2.28 artifact also includes `linux-runtime-sources-glibc-2.28.tar.gz` with
matching runtime sources and the helper rebuild recipe; it is not needed to run.

Both build jobs run formatting, unit/integration tests and a release build in their
baseline container. Packaging checks the glibc requirements of the final AppImage
runtime and all bundled ELF files, then tests the packaged CLI and GUI startup
with Xvfb/software rendering in that same environment. The Rocky job additionally
tests automatic fallback through real nxagent, with host graphics/picker resources
hidden to check that the bundle supplies them. It exercises resizing, Ctrl+wheel,
file/folder pickers, private-display authentication and normal window closing.
OpenSSL, libgcrypt and GnuTLS self-tests also run with vendor FIPS activation
enabled, including integrity-failure controls and checks for crypto errors in
the running GUI’s log.
This verifies those configurations, not every user's display, kernel or driver.

Tagged releases additionally publish:

- `Versus.exe` and `Versus-windows-x86_64.zip` for Windows 10 and 11.
- `versus-source-offline.tar.gz`: source, lockfile, vendor tree, build scripts, and
  generated dependency/license inventory for disconnected builds.

Versus is licensed under [MIT](LICENSE). Each release includes
`DEPENDENCY-LICENSES.md`, generated from Cargo metadata; review third-party
licenses before redistributing. Find and Replace also embeds Microsoft's
[VS Code Codicons](https://github.com/microsoft/vscode-codicons), licensed under
CC BY 4.0. Their [attribution](assets/codicons/NOTICE.md) and license are included
in the source and binary packages (`licenses/codicons` in portable bundles).

## Build from source

Install Rust 1.95 on a connected development machine, clone the repository, and run:

```powershell
cargo build --release --locked --offline
```

The checked-in `.cargo/config.toml` directs Cargo to `vendor/` and forces offline
resolution. The command must work without DNS, package repositories, or Internet
access when `Cargo.lock` and `vendor/` are present.

To refresh locked dependencies on a connected machine after intentionally changing
`Cargo.toml` or `Cargo.lock`, run:

```powershell
./scripts/vendor-dependencies.ps1
cargo build --release --locked --offline
./scripts/export-dependency-licenses.ps1 -OutputPath dist/DEPENDENCY-LICENSES.md
```

Review and commit the resulting `vendor/`, `.cargo/config.toml`, lockfile, and
inventory output used for a release. Do not run the vendor refresh in an air-gapped
environment. A separate Rust toolchain bundle is required where Rust is not already
installed; it is intentionally outside this repository.
The vendor tree includes a [native drop-position patch](docs/vendor-winit-drop-position.md).
Reapply it and update its checksums after refreshing dependencies.

On Linux, install the host development libraries required by the native windowing
backend, then use the same Cargo command. `scripts/build-linux.sh` lists the
Ubuntu 22.04 and Rocky 8.10 build packages used by CI and releases. On a machine
with Docker, reproduce either Linux build with:

```bash
docker run --rm --platform linux/amd64 -v "$PWD:/workspace" -w /workspace ubuntu:22.04 bash scripts/build-linux.sh 2.35
docker run --rm --platform linux/amd64 -v "$PWD:/workspace" -w /workspace rockylinux/rockylinux:8.10 bash scripts/build-linux.sh 2.28
```

Use separate clean checkouts for concurrent baseline builds. Container setup and
packaging need network access; Cargo compilation uses the vendored dependencies.

## Development checks

```powershell
cargo fmt --check
cargo test --locked --offline
cargo build --release --locked --offline
git diff --check
```

Run the narrow relevant test target first while developing. The full commands above
are the release readiness baseline.
