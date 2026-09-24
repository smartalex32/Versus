import { describe, expect, it } from "vitest";
import { visibleWindow } from "./windowing";

describe("visibleWindow", () => {
  it("uses bounded rows and preserves total spacer height", () => {
    expect(visibleWindow(1000, 400, 200, 20, 2)).toEqual({ start: 18, end: 32, top: 360, bottom: 19360 });
  });
  it("clamps at list boundaries", () => {
    expect(visibleWindow(3, 999, 100, 20, 2)).toEqual({ start: 3, end: 3, top: 60, bottom: 0 });
  });
});
