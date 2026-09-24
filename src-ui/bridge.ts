import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { DirectoryResult, FileResult, LineEnding, TextDiff } from "./types";

declare global { interface Window { __TAURI_INTERNALS__?: unknown } }
export const isTauri = () => Boolean(window.__TAURI_INTERNALS__);
export const pickPath = (directory: boolean) => invoke<string | null>("pick_path", { directory });
export const compareDirectories = (left: string, right: string) => invoke<DirectoryResult>("compare_directories", { left, right });
export const cancelDirectoryCompare = () => invoke<void>("cancel_directory_compare");
export const compareFiles = (left: string, right: string, ignoreWhitespace: boolean, ignoreLineEndings: boolean) => invoke<FileResult>("compare_files", { left, right, ignoreWhitespace, ignoreLineEndings });
export const compareTexts = (leftText: string, rightText: string, ignoreWhitespace: boolean, ignoreLineEndings: boolean) => invoke<TextDiff>("compare_texts", { leftText, rightText, ignoreWhitespace, ignoreLineEndings });
export const saveText = (path: string, contents: string, lineEnding: LineEnding, overwrite: boolean) => invoke<void>("save_text", { path, contents, lineEnding, overwrite });

/** Native WebView drops include real paths, unlike browser File objects. */
export function onNativePathDrop(handler: (paths: string[]) => void) {
  if (!isTauri()) return () => undefined;
  let unlisten: (() => void) | undefined;
  void getCurrentWindow().onDragDropEvent((event) => {
    const payload = event.payload as { type: string; paths?: string[] };
    if (payload.type === "drop" && payload.paths?.length) handler(payload.paths);
  }).then(stop => { unlisten = stop; });
  return () => unlisten?.();
}
