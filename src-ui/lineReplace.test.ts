import { describe, expect, it } from "vitest";
import { replaceLinesPreservingStyle } from "./lineReplace";

describe("replaceLinesPreservingStyle", () => {
  it("retains a CRLF target and terminal newline", () => {
    expect(replaceLinesPreservingStyle("one\r\ntwo\r\n", { start: 1, end: 2 }, ["changed"], true)).toBe("one\r\nchanged\r\n");
  });
  it("does not invent a terminal newline for an empty target", () => {
    expect(replaceLinesPreservingStyle("", { start: 0, end: 0 }, ["new"], false)).toBe("new");
  });
  it("removes the terminal newline when an EOF replacement has none", () => {
    expect(replaceLinesPreservingStyle("old\n", { start: 0, end: 1 }, ["new"], false)).toBe("new");
  });
});
