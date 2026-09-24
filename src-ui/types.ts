export type EntryKind = "File" | "Directory" | "Symlink" | "Other";
export type EntryState = "Same" | "Different" | "LeftOnly" | "RightOnly" | "TypeMismatch" | "Error";
export interface DirectoryEntry { relativePath: string; kind: EntryKind; state: EntryState; error?: string }
export interface DirectoryResult { entries: DirectoryEntry[]; cancelled: boolean }
export type HunkKind = "Added" | "Removed" | "Changed";
export interface TextHunk { leftStart: number; leftEnd: number; rightStart: number; rightEnd: number; kind: HunkKind }
export interface TextDiff { leftLines: string[]; rightLines: string[]; hunks: TextHunk[] }
export interface FileResult { kind: "Text" | "Binary" | "TooLarge"; equal: boolean; leftText?: string; rightText?: string; diff?: TextDiff }
export type LineEnding = "Lf" | "CrLf";
