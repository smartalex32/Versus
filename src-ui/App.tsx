import { useEffect, useMemo, useRef, useState } from "react";
import * as bridge from "./bridge";
import { replaceLinesPreservingStyle } from "./lineReplace";
import { visibleWindow } from "./windowing";
import type {
  DirectoryEntry,
  EntryState,
  FileResult,
  LineEnding,
  TextDiff,
  TextHunk,
} from "./types";

type Mode = "directories" | "files";
type Filter = "All" | "Different" | "Same" | "LeftOnly" | "RightOnly";
type FileFilter = "All" | "Differences" | "Same";
type Row = { left?: number; right?: number; hunk?: number; kind?: string };
const emptyDiff: TextDiff = { leftLines: [], rightLines: [], hunks: [] };
const stateLabel: Record<EntryState, string> = {
  Same: "Same",
  Different: "Different",
  LeftOnly: "Left only",
  RightOnly: "Right only",
  TypeMismatch: "Type mismatch",
  Error: "Error",
};

function aligned(diff: TextDiff): Row[] {
  const rows: Row[] = [];
  let l = 0,
    r = 0;
  const shared = (le: number, re: number) => {
    while (l < le || r < re)
      rows.push({
        left: l < le ? l++ : undefined,
        right: r < re ? r++ : undefined,
      });
  };
  diff.hunks.forEach((h, index) => {
    shared(h.leftStart, h.rightStart);
    const le = h.leftEnd,
      re = h.rightEnd;
    while (l < le || r < re)
      rows.push({
        left: l < le ? l++ : undefined,
        right: r < re ? r++ : undefined,
        hunk: index,
        kind: h.kind,
      });
  });
  shared(diff.leftLines.length, diff.rightLines.length);
  return rows;
}
function ending(text: string): LineEnding {
  return text.includes("\r\n") ? "CrLf" : "Lf";
}
function joinPath(root: string, relativePath: string): string {
  return `${root}${root.endsWith("\\") || root.endsWith("/") ? "" : "/"}${relativePath}`;
}
function PathInput({
  value,
  onChange,
  directory,
  onPick,
  label,
  onFocus,
}: {
  value: string;
  onChange(value: string): void;
  directory: boolean;
  onPick(): void;
  label: string;
  onFocus(): void;
}) {
  return (
    <label className="path-input">
      <span>{label}</span>
      <input
        value={value}
        placeholder={directory ? "Folder path" : "File path"}
        onFocus={onFocus}
        onChange={(e) => onChange(e.target.value)}
        onDrop={(e) => {
          e.preventDefault();
          const path = (
            e.dataTransfer.files[0] as (File & { path?: string }) | undefined
          )?.path;
          if (path) onChange(path);
        }}
        onDragOver={(e) => e.preventDefault()}
      />
      <button type="button" onClick={onPick}>
        Browse
      </button>
    </label>
  );
}

export default function App() {
  const saved = (() => {
    try {
      return JSON.parse(localStorage.getItem("versus-preferences") ?? "{}");
    } catch {
      return {};
    }
  })() as Partial<{ dark: boolean; whitespace: boolean; endings: boolean }>;
  const [mode, setMode] = useState<Mode>("directories");
  const [leftDir, setLeftDir] = useState("");
  const [rightDir, setRightDir] = useState("");
  const [leftFile, setLeftFile] = useState("");
  const [rightFile, setRightFile] = useState("");
  const [entries, setEntries] = useState<DirectoryEntry[]>([]);
  const [filter, setFilter] = useState<Filter>("All");
  const [selectedEntry, setSelectedEntry] = useState<string>();
  const [file, setFile] = useState<FileResult>();
  const [leftText, setLeftText] = useState("");
  const [rightText, setRightText] = useState("");
  const [diff, setDiff] = useState<TextDiff>(emptyDiff);
  const [selectedHunk, setSelectedHunk] = useState<number>();
  const [diffDirty, setDiffDirty] = useState(false);
  const [fileFilter, setFileFilter] = useState<FileFilter>("All");
  const [ignoreWhitespace, setIgnoreWhitespace] = useState(
    saved.whitespace ?? false,
  );
  const [ignoreLineEndings, setIgnoreLineEndings] = useState(
    saved.endings ?? true,
  );
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState(
    "Choose two folders or files to compare.",
  );
  const [dark, setDark] = useState(saved.dark ?? false);
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const [editing, setEditing] = useState(false);
  const [dropTarget, setDropTarget] = useState<"left" | "right">("left");
  const [saveTarget, setSaveTarget] = useState<"left" | "right" | null>(null);
  const [savePath, setSavePath] = useState("");
  const [overwrite, setOverwrite] = useState(false);
  const [needsOverwrite, setNeedsOverwrite] = useState(false);
  const [saveAs, setSaveAs] = useState(false);
  const leftScroll = useRef<HTMLDivElement>(null);
  const rightScroll = useRef<HTMLDivElement>(null);
  const treeScroll = useRef<HTMLDivElement>(null);
  const syncingScroll = useRef(false);
  const recalcVersion = useRef(0);
  const [treePosition, setTreePosition] = useState({ top: 0, height: 500 });
  const [diffPosition, setDiffPosition] = useState({ top: 0, height: 500 });
  useEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }, [dark]);
  useEffect(() => {
    localStorage.setItem(
      "versus-preferences",
      JSON.stringify({
        dark,
        whitespace: ignoreWhitespace,
        endings: ignoreLineEndings,
      }),
    );
  }, [dark, ignoreWhitespace, ignoreLineEndings]);
  useEffect(
    () =>
      bridge.onNativePathDrop((paths) => {
        const path = paths[0];
        if (!path) return;
        const set =
          mode === "directories"
            ? dropTarget === "left"
              ? setLeftDir
              : setRightDir
            : dropTarget === "left"
              ? setLeftFile
              : setRightFile;
        set(path);
        setStatus(`Dropped path into the ${dropTarget} field.`);
      }),
    [mode, dropTarget],
  );
  const rows = useMemo(() => aligned(diff), [diff]);
  const visibleRows = useMemo(
    () =>
      rows.filter(
        (row) =>
          fileFilter === "All" ||
          (fileFilter === "Differences"
            ? row.hunk !== undefined
            : row.hunk === undefined),
      ),
    [rows, fileFilter],
  );
  const childrenByPath = useMemo(() => {
    const parents = new Set<string>();
    for (const entry of entries) {
      let parent = entry.relativePath.replace(/[\\/][^\\/]+$/, "");
      while (parent) {
        parents.add(parent);
        parent = parent.replace(/[\\/][^\\/]+$/, "");
      }
    }
    return parents;
  }, [entries]);
  const shown = useMemo(() => {
    const matches = (entry: DirectoryEntry) =>
      filter === "All" ||
      (filter === "Different" && entry.state !== "Same") ||
      entry.state === filter;
    const wanted = new Set(entries.filter(matches).map((e) => e.relativePath));
    for (const path of [...wanted]) {
      let parent = path.replace(/[\\/][^\\/]+$/, "");
      while (parent) {
        wanted.add(parent);
        parent = parent.replace(/[\\/][^\\/]+$/, "");
      }
    }
    return entries
      .filter(
        (e) =>
          wanted.has(e.relativePath) &&
          ![...collapsed].some(
            (parent) =>
              (e.relativePath !== parent &&
                e.relativePath.startsWith(`${parent}/`)) ||
              e.relativePath.startsWith(`${parent}\\`),
          ),
      )
      .sort((a, b) => a.relativePath.localeCompare(b.relativePath));
  }, [entries, filter, collapsed]);
  const treeWindow = visibleWindow(
    shown.length,
    Math.max(0, treePosition.top - 36),
    treePosition.height,
    36,
  );
  const diffWindow = visibleWindow(
    visibleRows.length,
    diffPosition.top,
    diffPosition.height,
    19,
  );
  const choose = async (directory: boolean, set: (v: string) => void) => {
    try {
      const path = await bridge.pickPath(directory);
      if (path) set(path);
    } catch (e) {
      setStatus(String(e));
    }
  };
  const compareDirs = async () => {
    if (!leftDir || !rightDir) return setStatus("Enter both folder paths.");
    setBusy(true);
    setStatus("Comparing folders…");
    try {
      const result = await bridge.compareDirectories(leftDir, rightDir);
      setEntries(result.entries);
      setStatus(
        result.cancelled
          ? "Folder comparison cancelled."
          : `${result.entries.length} entries compared.`,
      );
    } catch (e) {
      setStatus(`Comparison failed: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };
  const loadFiles = async (left = leftFile, right = rightFile) => {
    if (!left || !right) return setStatus("Enter both file paths.");
    setBusy(true);
    setDiffDirty(true);
    setStatus("Comparing files…");
    try {
      const result = await bridge.compareFiles(
        left,
        right,
        ignoreWhitespace,
        ignoreLineEndings,
      );
      recalcVersion.current++;
      setFile(result);
      setLeftText(result.leftText ?? "");
      setRightText(result.rightText ?? "");
      setDiff(result.diff ?? emptyDiff);
      setFileFilter("All");
      setSelectedHunk(undefined);
      setDiffDirty(false);
      setStatus(
        result.equal
          ? "Files are equal."
          : result.kind === "Text"
            ? "Text differences loaded."
            : `${result.kind} comparison loaded.`,
      );
    } catch (e) {
      setStatus(`Comparison failed: ${String(e)}`);
    } finally {
      setBusy(false);
    }
  };
  const recalc = async (left = leftText, right = rightText) => {
    if (file?.kind !== "Text") return;
    setDiffDirty(true);
    const request = ++recalcVersion.current;
    try {
      const next = await bridge.compareTexts(
        left,
        right,
        ignoreWhitespace,
        ignoreLineEndings,
      );
      if (request !== recalcVersion.current) return;
      setDiff(next);
      setDiffDirty(false);
      setStatus("Diff recalculated.");
    } catch (e) {
      if (request === recalcVersion.current)
        setStatus(`Unable to recalculate: ${String(e)}`);
    }
  };
  const openEntry = (entry: DirectoryEntry) => {
    setSelectedEntry(entry.relativePath);
    if (entry.kind !== "File" || entry.state !== "Different") return;
    const left = joinPath(leftDir, entry.relativePath),
      right = joinPath(rightDir, entry.relativePath);
    setLeftFile(left);
    setRightFile(right);
    setMode("files");
    void loadFiles(left, right);
  };
  const copyHunk = (toRight: boolean) => {
    if (selectedHunk === undefined) return;
    const h = diff.hunks[selectedHunk];
    if (!h) return;
    const sourceText = toRight ? leftText : rightText,
      targetText = toRight ? rightText : leftText;
    const sourceLines = (toRight ? diff.leftLines : diff.rightLines).slice(
      toRight ? h.leftStart : h.rightStart,
      toRight ? h.leftEnd : h.rightEnd,
    );
    const targetRange = toRight
      ? { start: h.rightStart, end: h.rightEnd }
      : { start: h.leftStart, end: h.leftEnd };
    const sourceEnd = toRight ? h.leftEnd : h.rightEnd,
      sourceCount = toRight ? diff.leftLines.length : diff.rightLines.length,
      targetEnd = targetRange.end,
      targetCount = toRight ? diff.rightLines.length : diff.leftLines.length;
    const terminal =
      sourceEnd === sourceCount && targetEnd === targetCount
        ? sourceText.endsWith("\n")
        : targetText.endsWith("\n");
    const next = replaceLinesPreservingStyle(
      targetText,
      targetRange,
      sourceLines,
      terminal,
    );
    if (toRight) {
      setRightText(next);
      void recalc(leftText, next);
    } else {
      setLeftText(next);
      void recalc(next, rightText);
    }
  };
  const startSave = (target: "left" | "right", as = false) => {
    const path = as ? "" : target === "left" ? leftFile : rightFile;
    setSaveTarget(target);
    setSavePath(path);
    setSaveAs(as);
    setOverwrite(false);
    setNeedsOverwrite(false);
  };
  const save = async () => {
    if (!saveTarget || !savePath) return;
    const contents = saveTarget === "left" ? leftText : rightText;
    try {
      await bridge.saveText(savePath, contents, ending(contents), overwrite);
      setStatus(`Saved ${savePath}.`);
      setSaveTarget(null);
    } catch (e) {
      const message = String(e);
      const collision =
        message.includes(
          "destination already exists; explicit overwrite is required",
        ) || message.includes("destination appeared while saving");
      if (!overwrite && collision) {
        setNeedsOverwrite(true);
        setStatus(
          "The destination already exists. Confirm overwrite to continue.",
        );
      } else setStatus(`Save failed: ${message}`);
    }
  };
  const moveDifference = (direction: 1 | -1) => {
    if (!diff.hunks.length) return;
    const next =
      selectedHunk === undefined
        ? direction === 1
          ? 0
          : diff.hunks.length - 1
        : (selectedHunk + direction + diff.hunks.length) % diff.hunks.length;
    const index = visibleRows.findIndex((row) => row.hunk === next);
    if (index >= 0) {
      const top = Math.max(0, index * 19 - 190);
      if (leftScroll.current) leftScroll.current.scrollTop = top;
      if (rightScroll.current) rightScroll.current.scrollTop = top;
      setDiffPosition((old) => ({ ...old, top }));
    }
    setSelectedHunk(next);
  };
  const syncScroll = (source: "left" | "right") => {
    if (syncingScroll.current) return;
    const from = source === "left" ? leftScroll.current : rightScroll.current,
      to = source === "left" ? rightScroll.current : leftScroll.current;
    if (!from || !to) return;
    syncingScroll.current = true;
    to.scrollTop = from.scrollTop;
    setDiffPosition({ top: from.scrollTop, height: from.clientHeight });
    syncingScroll.current = false;
  };
  return (
    <main className="app">
      <header>
        <div className="brand">
          <span className="mark">V</span>
          <div>
            <strong>Versus</strong>
            <small>File and folder comparison</small>
          </div>
        </div>
        <nav>
          <button
            className={mode === "directories" ? "active" : ""}
            onClick={() => setMode("directories")}
          >
            Folders
          </button>
          <button
            className={mode === "files" ? "active" : ""}
            onClick={() => setMode("files")}
          >
            Files
          </button>
        </nav>
        <button className="theme" onClick={() => setDark(!dark)}>
          {dark ? "Light theme" : "Dark theme"}
        </button>
      </header>
      <section className="toolbar">
        <label>
          <input
            type="checkbox"
            checked={ignoreWhitespace}
            onChange={(e) => {
              recalcVersion.current++;
              setDiffDirty(true);
              setIgnoreWhitespace(e.target.checked);
            }}
          />{" "}
          Ignore whitespace
        </label>
        <label>
          <input
            type="checkbox"
            checked={ignoreLineEndings}
            onChange={(e) => {
              recalcVersion.current++;
              setDiffDirty(true);
              setIgnoreLineEndings(e.target.checked);
            }}
          />{" "}
          Ignore line endings
        </label>
        <span className="status">{busy ? "Working…" : status}</span>
      </section>
      {mode === "directories" ? (
        <section className="workspace directory">
          <div className="path-pair">
            <PathInput
              label="Left folder"
              value={leftDir}
              onChange={setLeftDir}
              directory
              onFocus={() => setDropTarget("left")}
              onPick={() => void choose(true, setLeftDir)}
            />
            <PathInput
              label="Right folder"
              value={rightDir}
              onChange={setRightDir}
              directory
              onFocus={() => setDropTarget("right")}
              onPick={() => void choose(true, setRightDir)}
            />
            <button
              className="primary"
              disabled={busy}
              onClick={() => void compareDirs()}
            >
              {busy ? "Comparing…" : "Compare folders"}
            </button>
            {busy && (
              <button onClick={() => void bridge.cancelDirectoryCompare()}>
                Cancel
              </button>
            )}
          </div>
          <div className="directory-list">
            <aside>
              <h2>Filter</h2>
              {(
                [
                  "All",
                  "Different",
                  "Same",
                  "LeftOnly",
                  "RightOnly",
                ] as Filter[]
              ).map((f) => (
                <button
                  key={f}
                  className={filter === f ? "selected" : ""}
                  onClick={() => setFilter(f)}
                >
                  {f === "LeftOnly"
                    ? "Left only"
                    : f === "RightOnly"
                      ? "Right only"
                      : f}
                </button>
              ))}
            </aside>
            <div
              className="tree"
              ref={treeScroll}
              onScroll={(event) =>
                setTreePosition({
                  top: event.currentTarget.scrollTop,
                  height: event.currentTarget.clientHeight,
                })
              }
            >
              <div className="tree-head">
                <span>Relative path</span>
                <span>Kind</span>
                <span>Status</span>
              </div>
              <div style={{ height: treeWindow.top }} />
              {shown.slice(treeWindow.start, treeWindow.end).map((e) => {
                const children = childrenByPath.has(e.relativePath);
                const depth = Math.max(
                  0,
                  e.relativePath.split(/[\\/]/).length - 1,
                );
                return (
                  <button
                    className={`tree-row ${selectedEntry === e.relativePath ? "selected" : ""} state-${e.state}`}
                    key={`${e.relativePath}:${e.state}`}
                    title={e.error}
                    onClick={() => openEntry(e)}
                  >
                    <span style={{ paddingLeft: `${depth * 18}px` }}>
                      {e.kind === "Directory" && children ? (
                        <i
                          onClick={(event) => {
                            event.stopPropagation();
                            setCollapsed((old) => {
                              const next = new Set(old);
                              next.has(e.relativePath)
                                ? next.delete(e.relativePath)
                                : next.add(e.relativePath);
                              return next;
                            });
                          }}
                        >
                          {collapsed.has(e.relativePath) ? "▸" : "▾"}
                        </i>
                      ) : (
                        <i>·</i>
                      )}
                      {e.relativePath.split(/[\\/]/).at(-1)}
                    </span>
                    <span>{e.kind}</span>
                    <span>{stateLabel[e.state]}</span>
                  </button>
                );
              })}
              <div style={{ height: treeWindow.bottom }} />
              {entries.length === 0 && (
                <p className="empty">
                  Results will appear here. Drag paths onto either field, paste
                  a path, or use Browse.
                </p>
              )}
            </div>
          </div>
        </section>
      ) : (
        <section className="workspace files">
          <div className="path-pair">
            <PathInput
              label="Left file"
              value={leftFile}
              onChange={setLeftFile}
              directory={false}
              onFocus={() => setDropTarget("left")}
              onPick={() => void choose(false, setLeftFile)}
            />
            <PathInput
              label="Right file"
              value={rightFile}
              onChange={setRightFile}
              directory={false}
              onFocus={() => setDropTarget("right")}
              onPick={() => void choose(false, setRightFile)}
            />
            <button
              className="primary"
              disabled={busy}
              onClick={() => void loadFiles()}
            >
              {busy ? "Comparing…" : "Compare files"}
            </button>
          </div>
          {file && file.kind !== "Text" ? (
            <div className="notice">
              {file.kind === "Binary"
                ? "Binary files cannot be displayed as text."
                : "These files are too large to display."}{" "}
              {file.equal ? "They are equal." : "They differ."}
            </div>
          ) : (
            <>
              <div className="diff-actions">
                <button
                  onClick={() => moveDifference(-1)}
                  disabled={diffDirty || !diff.hunks.length}
                >
                  Previous difference
                </button>
                <button
                  onClick={() => moveDifference(1)}
                  disabled={diffDirty || !diff.hunks.length}
                >
                  Next difference
                </button>
                <button
                  disabled={diffDirty || selectedHunk === undefined}
                  onClick={() => copyHunk(false)}
                >
                  Copy right → left
                </button>
                <button
                  disabled={diffDirty || selectedHunk === undefined}
                  onClick={() => copyHunk(true)}
                >
                  Copy left → right
                </button>
                {(["All", "Differences", "Same"] as FileFilter[]).map(
                  (option) => (
                    <button
                      key={option}
                      disabled={diffDirty}
                      className={fileFilter === option ? "selected" : ""}
                      onClick={() => setFileFilter(option)}
                    >
                      {option}
                    </button>
                  ),
                )}
                <button onClick={() => setEditing(!editing)}>
                  {editing ? "Show aligned diff" : "Edit buffers"}
                </button>
                <button onClick={() => void recalc()}>Recalculate diff</button>
                <span>
                  {diffDirty
                    ? "Diff needs recalculation"
                    : `${diff.hunks.length} change ${diff.hunks.length === 1 ? "group" : "groups"}`}
                </span>
              </div>
              {editing ? (
                <div className="buffer-grid">
                  <div className="editor">
                    <div className="pane-title">
                      Left buffer{" "}
                      <button onClick={() => startSave("left")}>Save</button>
                      <button onClick={() => startSave("left", true)}>
                        Save As
                      </button>
                    </div>
                    <textarea
                      value={leftText}
                      spellCheck={false}
                      onChange={(e) => {
                        recalcVersion.current++;
                        setDiffDirty(true);
                        setLeftText(e.target.value);
                      }}
                      onBlur={() => void recalc()}
                    />
                  </div>
                  <div className="editor">
                    <div className="pane-title">
                      Right buffer{" "}
                      <button onClick={() => startSave("right")}>Save</button>
                      <button onClick={() => startSave("right", true)}>
                        Save As
                      </button>
                    </div>
                    <textarea
                      value={rightText}
                      spellCheck={false}
                      onChange={(e) => {
                        recalcVersion.current++;
                        setDiffDirty(true);
                        setRightText(e.target.value);
                      }}
                      onBlur={() => void recalc()}
                    />
                  </div>
                </div>
              ) : (
                <div className="aligned-grid">
                  <div className="read-pane">
                    <div className="pane-title">
                      Left{" "}
                      <button onClick={() => startSave("left")}>Save</button>
                      <button onClick={() => startSave("left", true)}>
                        Save As
                      </button>
                    </div>
                    <div
                      className="aligned-lines"
                      ref={leftScroll}
                      onScroll={() => syncScroll("left")}
                    >
                      <div style={{ height: diffWindow.top }} />
                      {visibleRows
                        .slice(diffWindow.start, diffWindow.end)
                        .map((row, i) => (
                        <button
                          disabled={diffDirty}
                          data-hunk={row.hunk}
                          key={i}
                          className={`${row.hunk === selectedHunk ? "selected " : ""}${row.kind ?? ""}`}
                          onClick={() =>
                            row.hunk !== undefined && setSelectedHunk(row.hunk)
                          }
                        >
                          <b>{row.left === undefined ? "" : row.left + 1}</b>
                          <code>
                            {row.left === undefined
                              ? ""
                              : diff.leftLines[row.left]}
                          </code>
                        </button>
                        ))}
                      <div style={{ height: diffWindow.bottom }} />
                    </div>
                  </div>
                  <div className="read-pane">
                    <div className="pane-title">
                      Right{" "}
                      <button onClick={() => startSave("right")}>Save</button>
                      <button onClick={() => startSave("right", true)}>
                        Save As
                      </button>
                    </div>
                    <div
                      className="aligned-lines"
                      ref={rightScroll}
                      onScroll={() => syncScroll("right")}
                    >
                      <div style={{ height: diffWindow.top }} />
                      {visibleRows
                        .slice(diffWindow.start, diffWindow.end)
                        .map((row, i) => (
                        <button
                          disabled={diffDirty}
                          data-hunk={row.hunk}
                          key={i}
                          className={`${row.hunk === selectedHunk ? "selected " : ""}${row.kind ?? ""}`}
                          onClick={() =>
                            row.hunk !== undefined && setSelectedHunk(row.hunk)
                          }
                        >
                          <b>{row.right === undefined ? "" : row.right + 1}</b>
                          <code>
                            {row.right === undefined
                              ? ""
                              : diff.rightLines[row.right]}
                          </code>
                        </button>
                        ))}
                      <div style={{ height: diffWindow.bottom }} />
                    </div>
                  </div>
                </div>
              )}
            </>
          )}
        </section>
      )}
      {saveTarget && (
        <div className="modal-backdrop">
          <form
            className="modal"
            onSubmit={(e) => {
              e.preventDefault();
              void save();
            }}
          >
            <h2>
              {saveAs ? "Save As" : "Save"} {saveTarget} text
            </h2>
            <label>
              Destination path
              <input
                autoFocus
                value={savePath}
                onChange={(e) => {
                  setSavePath(e.target.value);
                  setNeedsOverwrite(false);
                  setOverwrite(false);
                }}
              />
            </label>
            {needsOverwrite && (
              <label>
                <input
                  type="checkbox"
                  checked={overwrite}
                  onChange={(e) => setOverwrite(e.target.checked)}
                />{" "}
                I confirm that this existing file may be overwritten.
              </label>
            )}
            <div>
              <button type="button" onClick={() => setSaveTarget(null)}>
                Cancel
              </button>
              <button
                className="primary"
                disabled={!savePath || (needsOverwrite && !overwrite)}
              >
                Save
              </button>
            </div>
          </form>
        </div>
      )}
    </main>
  );
}
