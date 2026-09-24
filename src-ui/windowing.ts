export interface VisibleWindow { start: number; end: number; top: number; bottom: number }

/** Returns a bounded row slice plus spacer heights for fixed-height virtual lists. */
export function visibleWindow(count: number, scrollTop: number, viewportHeight: number, rowHeight: number, overscan = 8): VisibleWindow {
  const first = Math.min(count, Math.max(0, Math.floor(scrollTop / rowHeight) - overscan));
  const end = Math.min(count, Math.ceil((scrollTop + viewportHeight) / rowHeight) + overscan);
  return { start: first, end, top: first * rowHeight, bottom: Math.max(0, (count - end) * rowHeight) };
}
