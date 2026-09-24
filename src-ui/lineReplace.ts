/** Replaces a diff line range while retaining the target's newline convention. */
export function replaceLinesPreservingStyle(
  target: string,
  range: { start: number; end: number },
  replacement: string[],
  sourceTerminalNewline: boolean,
): string {
  const lineEnding = target.includes("\r\n") ? "\r\n" : "\n";
  const lines = target === "" ? [] : target.split(/\r?\n/).filter((line, index, all) => !(index === all.length - 1 && line === "" && target.endsWith("\n")));
  lines.splice(range.start, range.end - range.start, ...replacement.map(line => line.replace(/\r$/, "")));
  const result = lines.join(lineEnding);
  return sourceTerminalNewline && !result.endsWith("\n") ? `${result}${lineEnding}` : result;
}
