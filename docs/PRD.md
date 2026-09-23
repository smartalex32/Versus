Versus

Product Requirements Document

Current implementation focus (September 2026): directory and two-way file
comparison. The three-way compare and merge interface is deferred; its requirements
below remain as historical scope for a later iteration.

1. Summary

Versus is a small, fast, offline desktop utility for comparing files and directories.

It is intended as a simplified alternative to Beyond Compare focused exclusively on the workflows most commonly needed in development and locked-down engineering environments:

* Compare two directories.
* Compare two files.
* Compare and merge three versions of a file.
* Compare files and directories across different drives.
* Compare local files against mapped or network drives.
* Operate completely offline.
* Require no installation of runtimes, package managers, frameworks, or additional dependencies.

Versus will support Windows and Linux.

The application should prioritize simplicity, portability, deterministic behavior, and ease of deployment into air-gapped or highly restricted environments.

⸻

2. Product Goals

Versus should allow a user to download or copy the built application onto a workstation and immediately run it.

Primary goals:

* No Internet connection required.
* No account or authentication system.
* No telemetry.
* No cloud services.
* No runtime downloads.
* No database.
* No background services.
* No administrator privileges required.
* No installer required for the portable distribution.
* Minimal external operating-system dependencies.
* Fast startup.
* Simple interface.
* Reliable operation against local and network-mounted filesystems.

The core interaction should remain:

Select two things → compare them → inspect differences.

Or:

Select three files → compare/merge them.

⸻

3. Target Platforms

Windows

Initial supported target:

* Windows 10
* Windows 11

Supported paths include:

C:\Projects\App
D:\Builds\App
Z:\Shared\App
\\server\share\App
\\server\share\folder\file.txt

Both mapped network drives and UNC paths must be supported.

Linux

Support mainstream x86-64 Linux environments.

Examples:

/home/user/project
/opt/project
/mnt/shared/project
/mnt/nfs/project

Network storage is expected to be mounted through the operating system.

Versus does not implement SMB, NFS, authentication, or other network protocols itself.

⸻

4. Deployment Requirement

Deployment simplicity is a critical product requirement.

Windows distribution

Preferred artifact:

Versus.exe

The release executable should contain everything Versus itself requires to operate.

The user should be able to:

1. Copy Versus.exe onto a machine.
2. Double-click it.
3. Use Versus.

There should be:

* No MSI requirement.
* No administrator requirement.
* No .NET installation requirement.
* No Java requirement.
* No Node.js requirement.
* No Python requirement.
* No package manager requirement.
* No Internet access requirement.

An optional installer may eventually be provided for convenience, but the portable executable remains the canonical release artifact.

⸻

5. Linux Distribution

Preferred artifact:

Versus.AppImage

The AppImage should package the application and its necessary userspace libraries into a single executable artifact.

Typical usage:

chmod +x Versus.AppImage
./Versus.AppImage

No package installation should normally be necessary.

A secondary portable archive may optionally be produced:

versus-linux-x86_64.tar.gz

Containing:

versus
README.txt
LICENSE

The AppImage should be considered the primary Linux distribution.

⸻

6. Air-Gapped Environment Requirements

Versus must be designed specifically to work in disconnected environments.

Runtime operation must never require:

* Internet access
* DNS access
* Update servers
* License servers
* Remote APIs
* CDNs
* Package repositories
* Authentication services
* Analytics servers
* Cloud storage

Versus should not contain HTTP client functionality unless a future feature explicitly requires it.

The application should not attempt outbound connections.

Network filesystem access through paths such as:

\\server\engineering

is permitted because the operating system handles the connection.

This is distinct from Internet connectivity.

⸻

7. Offline Source Build

The source repository must also be capable of being transferred into an air-gapped development environment.

All Rust library dependencies should be vendored into the repository or into an accompanying offline dependency bundle.

Repository structure:

versus/
├── Cargo.toml
├── Cargo.lock
├── .cargo/
│   └── config.toml
├── vendor/
├── src/
├── tests/
├── scripts/
└── README.md

Offline builds should support:

cargo build --release --locked --offline

Dependency versions must be locked.

An Internet-connected build environment may run:

cargo vendor

before transferring the repository into the air-gapped environment.

No build step should unexpectedly attempt dependency resolution from the Internet.

⸻

8. Recommended Technology

Language

Rust

Reasons:

* Produces native binaries.
* Excellent filesystem support.
* Strong Windows/Linux support.
* No managed runtime requirement.
* Good performance for large directory trees and files.
* Dependencies can be vendored.
* Suitable for restricted environments.

UI

egui / eframe

Requirements for the UI framework:

* Native application.
* No browser.
* No local web server.
* No Electron.
* No Node.js runtime.
* No WebView dependency as part of Versus architecture.

Core libraries

Recommended responsibilities:

eframe / egui
    Desktop UI
similar
    File diff and three-way diff algorithms
walkdir
    Recursive directory traversal
rfd or equivalent
    Native file/folder selection dialogs

Dependencies should be kept deliberately small.

⸻

9. Primary Application Modes

Versus contains three primary modes:

Directory
2-Way
3-Way

These should be immediately accessible from the main interface.

⸻

10. Directory Comparison

Purpose

Compare the contents of two directories.

Example:

LEFT
C:\Projects\MyApp
RIGHT
\\engineering-server\Release\MyApp

Versus recursively compares both directory trees.

File states

Each relative path receives one of the following states:

Same
Different
Left Only
Right Only
Type Mismatch
Error

Example:

STATUS       PATH
Same         README.md
Different    src/config.rs
Different    Cargo.toml
Left Only    docs/old.md
Right Only   src/new.rs

Directory comparison logic

Comparison should proceed efficiently.

Recommended initial process:

1. Relative path comparison.
2. File type comparison.
3. File size comparison.
4. Buffered byte comparison when necessary.

Timestamp alone must never determine file equality.

Timestamps may be displayed as metadata but should not be considered authoritative evidence that file contents match.

⸻

11. Directory Filters

The directory view should provide simple filters:

All
Different
Same
Left Only
Right Only

Default:

Different

The user should be able to quickly answer:

What changed?

without scrolling through thousands of identical files.

⸻

12. Directory Navigation

Double-clicking a changed text file should automatically open the file in the 2-Way comparison view.

Example:

C:\Project\src\config.rs
vs
\\server\release\Project\src\config.rs

The user should be able to return to the directory comparison without losing the comparison results.

⸻

13. Two-Way File Comparison

The 2-Way view compares:

LEFT
vs
RIGHT

Example:

C:\Projects\App\config.json
vs
Z:\Release\App\config.json

UI:

┌─────────────────────────┬─────────────────────────┐
│ LEFT                    │ RIGHT                   │
│ config.json             │ config.json             │
├─────────────────────────┼─────────────────────────┤
│ 18 | server: "dev"      │ 18 | server: "prod"     │
│ 19 | port: 3030         │ 19 | port: 8080         │
│ 20 | enabled: true      │ 20 | enabled: true       │
└─────────────────────────┴─────────────────────────┘
Previous Difference       Next Difference

⸻

14. Two-Way Diff Requirements

Required functionality:

* Side-by-side text display.
* Line numbers.
* Added-line highlighting.
* Removed-line highlighting.
* Changed-line highlighting.
* Synchronized vertical scrolling.
* Previous difference.
* Next difference.
* Difference count.
* Current difference indicator.
* Open another file.
* Reload files.

Example:

Difference 4 of 12

⸻

15. Merge / Copy Operations

Users should be able to move content between sides.

Basic controls:

Copy →
← Copy

Operations should operate on the currently selected difference block.

The user must explicitly save changes.

Versus should never silently overwrite a file.

⸻

16. Editing

Both file panes may optionally become editable.

Minimum requirement:

The user must be able to modify the resulting file when resolving differences.

Editing functionality should remain deliberately limited.

Versus is not intended to replace a text editor or IDE.

Required:

* Basic text editing.
* Undo.
* Redo.
* Save.
* Save As.

Not required:

* IntelliSense.
* Language servers.
* Code completion.
* Refactoring.
* Compiler integration.

⸻

17. Three-Way Comparison

Three-way comparison uses:

BASE
LEFT
RIGHT

Example:

BASE
original/config.rs
LEFT
branch-a/config.rs
RIGHT
branch-b/config.rs

Versus identifies:

* Changes only on the left.
* Changes only on the right.
* Matching changes.
* Conflicting changes.

⸻

18. Three-Way Merge

The 3-Way interface should display:

┌──────────────┬──────────────┬──────────────┐
│ BASE         │ LEFT         │ RIGHT        │
├──────────────┼──────────────┼──────────────┤
│              │              │              │
│              │              │              │
└──────────────┴──────────────┴──────────────┘
┌────────────────────────────────────────────┐
│ MERGED RESULT                              │
│                                            │
│                                            │
└────────────────────────────────────────────┘

Conflict resolution controls:

Use Left
Use Right
Edit Result

The output pane should represent the file that will be saved.

⸻

19. Path Selection

Every file/directory path field must support:

* Browse button.
* Direct text entry.
* Paste.
* Drag and drop.

Example:

LEFT
C:\Projects\App
[ Browse ]
RIGHT
\\server\engineering\Releases\App
[ Browse ]
[ Compare ]

Users must not be forced to navigate to network paths exclusively through the file picker.

UNC paths must be directly pasteable.

⸻

20. Network Drive Behavior

Network drives should behave exactly like local files whenever possible.

Versus should rely on the operating system filesystem APIs.

Supported examples:

C:\Folder
↕
Z:\Folder

and:

C:\Folder
↕
\\server\share\Folder

Versus does not:

* Store server credentials.
* Implement SMB.
* Mount shares.
* Authenticate network users.

Authentication remains the responsibility of the operating system.

⸻

21. Network Error Handling

Network filesystems can disappear during comparison.

Versus must handle:

* Disconnected mapped drive.
* Unavailable UNC share.
* Permission denied.
* File removed during comparison.
* Directory removed during traversal.
* Read failure.
* Write failure.
* Connection interruption.

The application must not crash.

Example message:

Unable to read:
\\server\engineering\App\config.json
The file or network location is currently unavailable.

The user should be able to retry.

⸻

22. Large Files

Versus should avoid unnecessarily loading large files entirely into memory.

Directory equality checking should use buffered I/O.

Text diffing may load reasonable-size text files into memory where appropriate.

A configurable internal threshold may be used for very large files.

Example:

File is 1.8 GB.
Text comparison is unavailable for files of this size.
[ Compare Binary Contents ]

Exact thresholds can be selected during implementation.

⸻

23. Binary Files

Versus does not initially provide binary diff visualization.

Binary files receive one of two states:

Binary files match

or

Binary files differ

Binary files may still participate in directory comparison.

⸻

24. Text Encoding

Initial requirements:

* UTF-8
* UTF-8 with BOM
* ASCII

The application should gracefully identify files it cannot safely display as text.

Future releases may add additional legacy encodings.

⸻

25. Line Ending Handling

Versus must correctly handle:

LF
CRLF

Users should have an option:

Ignore line-ending differences

This should default to enabled for textual comparison.

⸻

26. Whitespace Handling

Provide:

Ignore whitespace

Initial default:

Off

Potential future controls:

Ignore leading whitespace
Ignore trailing whitespace
Ignore all whitespace

These are not required for the initial version.

⸻

27. User Interface

The interface should emphasize utility over decoration.

Primary navigation:

Versus
[ Directory ] [ 2-Way ] [ 3-Way ]

No dashboard is necessary.

No home feed is necessary.

No account screen is necessary.

No onboarding sequence is necessary.

Opening the program should immediately present a useful comparison interface.

⸻

28. Recent Comparisons

A small local recent-history feature may be included.

Example:

Recent
C:\Project ↔ Z:\Project
config.old ↔ config.new

Requirements:

* Stored locally.
* Optional.
* Easy to clear.
* Contains no file contents.

This feature may be deferred until after MVP.

⸻

29. Settings

Settings should be minimal.

Initial options:

Ignore line-ending differences
Ignore whitespace
Confirm before overwrite
Remember recent comparisons

Configuration should be stored in a small local file.

No database is required.

⸻

30. Security Requirements

Versus should have an intentionally small attack surface.

Requirements:

* No telemetry.
* No analytics.
* No advertisements.
* No automatic updates.
* No embedded browser.
* No Internet API clients required for application functionality.
* No execution of compared files.
* No macro system.
* No plugin execution system.
* No shell command execution from compared content.
* No automatic file uploads.

Compared files should be treated strictly as data.

⸻

31. File Modification Safety

A comparison must never modify either source simply by opening it.

Writes occur only when the user explicitly invokes an operation such as:

Save
Copy →
← Copy
Save As

If a target file already exists, Versus should request confirmation.

Where practical, saving should use:

write temporary file
→ verify successful write
→ replace destination

to reduce the chance of corrupting a file if a write fails.

⸻

32. Symlink Behavior

Directory traversal must handle symbolic links safely.

Default behavior:

Do not recursively follow directory symlinks.

Versus should identify a symlink as such.

This prevents:

* Recursive loops.
* Unexpected filesystem traversal.
* Traversal outside the selected tree.

Following symlinks can become an advanced option later.

⸻

33. Performance

Target performance philosophy:

Versus should feel instantaneous for normal engineering projects.

Directory traversal should:

* Avoid reading file contents until needed.
* Perform inexpensive comparisons first.
* Keep the UI responsive.
* Run comparison work outside the UI rendering thread.

The application should be usable while scanning larger directory trees.

⸻

34. Cancellation

Long directory comparisons must provide:

Cancel

The user should not have to terminate the program to stop a large scan.

⸻

35. Architecture

Recommended structure:

src/
├── main.rs
├── app.rs
│
├── diff/
│   ├── mod.rs
│   ├── text.rs
│   ├── directory.rs
│   ├── three_way.rs
│   └── binary.rs
│
├── fs/
│   ├── mod.rs
│   ├── reader.rs
│   └── writer.rs
│
├── ui/
│   ├── mod.rs
│   ├── directory.rs
│   ├── two_way.rs
│   ├── three_way.rs
│   └── settings.rs
│
└── config/
    └── mod.rs

The diff engine should not depend on the UI.

Example interfaces:

compare_files(left, right) -> FileDiff
compare_directories(left, right) -> DirectoryDiff
compare_three_way(base, left, right) -> ThreeWayDiff

This separation is important for testing and future CLI support.

⸻

36. Testing

Automated tests should cover:

File comparison

* Identical files.
* One changed line.
* Added lines.
* Deleted lines.
* Empty files.
* Large files.
* CRLF versus LF.
* Unicode.
* Binary files.

Directory comparison

* Identical trees.
* Left-only file.
* Right-only file.
* Changed file.
* Nested directory.
* Empty directories.
* Type mismatch.
* Missing directory.
* Permission failure.

Three-way comparison

* Left-only modification.
* Right-only modification.
* Same modification on both sides.
* Conflicting modification.
* Added lines.
* Deleted lines.

Filesystem behavior

* Windows drive-letter paths.
* Windows UNC paths.
* Linux absolute paths.
* Read-only files.
* Unavailable paths.

⸻

37. Logging

Logging should primarily support troubleshooting.

Logs should:

* Remain local.
* Never be uploaded.
* Avoid recording file contents.
* Avoid recording secrets where practical.

A command-line option may eventually support:

versus --log-level debug

Normal users should not need to interact with logging.

⸻

38. Release Artifacts

Every release should produce:

Windows

Versus.exe

Optional:

Versus-windows-x86_64.zip

containing the portable executable and license/readme.

Linux

Versus.AppImage

Optional:

versus-linux-x86_64.tar.gz

⸻

39. Source Release Bundle

For organizations that must compile Versus internally, provide an offline source package:

versus-source-offline.tar.gz

Containing:

source
Cargo.lock
vendor/
build scripts
licenses
README

A separate Rust toolchain bundle may also be maintained for environments where the target system cannot retrieve Rust through rustup.

The goal should be:

Unpack
→ Build
→ Run

without Internet access.

⸻

40. Dependency Policy

Dependencies should be minimized.

Every new dependency should be evaluated for:

* Necessity.
* Maintenance status.
* License.
* Transitive dependency count.
* Native runtime requirements.
* Offline build compatibility.
* Security exposure.

Avoid adding dependencies for functionality that can reasonably be implemented with the Rust standard library.

The release process should maintain a dependency/license inventory suitable for restricted engineering environments.

⸻

41. MVP

The first production-capable release should include:

* Windows support.
* Linux support.
* Portable Windows executable.
* Portable Linux AppImage.
* Offline operation.
* Local file paths.
* Mapped drives.
* UNC paths.
* Mounted Linux network drives.
* Directory comparison.
* 2-way text comparison.
* 3-way text comparison.
* Basic merge operations.
* Difference navigation.
* Binary equality detection.
* Ignore line-ending differences.
* Optional whitespace ignoring.
* Safe save behavior.
* Error handling for unavailable network files.
* Vendored dependencies.
* Fully offline source build.

⸻

42. Explicit Non-Goals

Initial Versus will not include:

* FTP.
* SFTP.
* HTTP.
* Cloud storage.
* Google Drive.
* OneDrive.
* Dropbox.
* Git hosting integration.
* Git repository management.
* Image comparison.
* PDF comparison.
* Office document comparison.
* Hex editor.
* Binary merge.
* Database comparison.
* Registry comparison.
* Archive browsing.
* Folder synchronization engine.
* Remote shell functionality.
* Plugins.
* User accounts.
* Teams.
* Cloud settings.
* Telemetry.
* Automatic Internet updates.

These exclusions are intentional.

⸻

43. Future Enhancements

Potential future additions after the core utility is stable:

* CLI interface.
* Dark/light themes.
* Syntax highlighting.
* Search within compared files.
* Ignore-pattern configuration.
* Directory exclusion rules.
* Session saving.
* Drag-and-drop directories.
* Checksums.
* Image comparison.
* Folder synchronization.
* Shell context-menu integration.

None of these should compromise the application’s offline and portable nature.

⸻

44. MVP User Flow

Directory comparison

Launch Versus
      ↓
Select Directory
      ↓
Choose Left
      ↓
Choose Right
      ↓
Compare
      ↓
View changed files
      ↓
Double-click file
      ↓
2-Way Diff

File comparison

Launch Versus
      ↓
2-Way
      ↓
Select Left
      ↓
Select Right
      ↓
Compare
      ↓
Navigate differences

Three-way merge

Launch Versus
      ↓
3-Way
      ↓
Select Base
Select Left
Select Right
      ↓
Compare
      ↓
Resolve conflicts
      ↓
Save Result

⸻

45. Definition of Done

Versus v1.0 is complete when a user can take a release onto a clean supported workstation with no Internet connection and:

1. Launch Versus without installing a programming runtime.
2. Compare two local directories.
3. Compare a local directory against a network directory.
4. Compare a C:\ file against a mapped-drive file.
5. Compare a C:\ file against a UNC path.
6. Open changed files directly from directory comparison.
7. Perform a side-by-side 2-way text diff.
8. Navigate between differences.
9. Copy selected differences between files.
10. Perform a three-way comparison.
11. Resolve three-way conflicts into a merged result.
12. Save the resulting file.
13. Identify whether binary files match.
14. Remain functional with no Internet connection.
15. Recover cleanly from unavailable files or network shares.
16. Run without administrative privileges.
17. Build from a fully vendored source tree without contacting the Internet.

The primary product standard is:

Copy Versus onto the machine and run it.

No setup should be required for normal use.
