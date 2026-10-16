# Versus

Language for comparing filesystem sources and editing retained text working copies.

## Language

**Comparison mode**:
The choice of File or Folder sources.
_Avoid_: workspace mode when referring to source type

**Workspace mode**:
The choice of read-only Diff or editable Merge for the current sources.
_Avoid_: comparison mode when referring to editability

**Session**:
The current sources, comparison results, retained edits, and navigation state.
Starting a new comparison replaces the session.

**Working copy**:
Editable text retained in memory with its saved baseline and source encoding.
An edit becomes a disk change only through an explicit save.
_Avoid_: merged result for either independent editable side

**Document identity**:
The identity of the physical file represented by a working copy, shared by its
path aliases. Its native write destination can retain the user's original path.

**File pair**:
The two documents currently displayed together, each with its own working copy.
Documents may participate in different pairs during a session.

**Chunk**:
A detected structural block or blank-line paragraph available for text transfer.
_Avoid_: difference block when the chunk exists without comparison

**Display projection**:
A view of source text with display normalization or hidden unchanged lines.
The original working copy remains authoritative.

**Replacement batch**:
One undoable collection of validated search replacements across participating
working copies.
_Avoid_: saved batch, filesystem transaction

**Working-copy overlay**:
A folder comparison status reflecting retained working text in place of the
corresponding disk contents.

**Run Diff**:
Optional comparison assistance in Merge, independent of editing and transfers.

**CLI merge**:
The separate base/ours/theirs merge that produces an output file and conflict
markers without opening the desktop workspace.
_Avoid_: three-way desktop Merge
