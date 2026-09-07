import { computeVisibleRange } from "./Table";

/** Below this many lines, virtualising isn't worth the spacer bookkeeping. */
const DEFAULT_THRESHOLD = 100;
/** Extra rows rendered above/below the viewport to hide scroll gaps. */
const DEFAULT_OVERSCAN = 12;

/** The slice of log lines to render plus the spacer heights that reserve the
 *  scroll extent of the off-screen rows. */
export interface LogWindow {
  start: number;
  end: number;
  topPad: number;
  bottomPad: number;
  virtualized: boolean;
}

/**
 * Decide which log lines to actually render. Only fixed-height rows can be
 * windowed by arithmetic, so we bail out (render everything) when lines are
 * wrapped (variable height), below the threshold, or before a row height has
 * been measured (jsdom, first paint). Otherwise we render the scrolled slice
 * and reserve the rest with top/bottom spacer heights so the scrollbar and
 * scroll position stay correct.
 */
export function computeLogWindow({
  total,
  scrollTop,
  viewportHeight,
  rowHeight,
  wrap,
  overscan = DEFAULT_OVERSCAN,
  threshold = DEFAULT_THRESHOLD,
}: {
  total: number;
  scrollTop: number;
  viewportHeight: number;
  rowHeight: number;
  wrap: boolean;
  overscan?: number;
  threshold?: number;
}): LogWindow {
  const virtualized = !wrap && total > threshold && rowHeight > 0;
  if (!virtualized) {
    return { start: 0, end: total, topPad: 0, bottomPad: 0, virtualized: false };
  }
  const { start, end } = computeVisibleRange({ scrollTop, viewportHeight, rowHeight, total, overscan });
  return {
    start,
    end,
    topPad: start * rowHeight,
    bottomPad: (total - end) * rowHeight,
    virtualized: true,
  };
}
