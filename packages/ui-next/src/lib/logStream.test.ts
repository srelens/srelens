import { describe, it, expect, expectTypeOf, vi, beforeEach, afterEach } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { LogStatus, LogStreamOptions, LogTarget, SpoolPage } from "@srelens/core";
import type { UseLogStreamOptions } from "./logStream";

const startLogStream = vi.fn();
/** Calls through to the real ring; counts how often the hook folds lines into it. */
const appendLogLines = vi.fn();
/** Likewise for a page put in front of the ring (#914). */
const prependLogLines = vi.fn();

vi.mock("@srelens/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core")>();
  appendLogLines.mockImplementation(actual.appendLogLines);
  prependLogLines.mockImplementation(actual.prependLogLines);
  return {
    ...actual,
    startLogStream: (...args: unknown[]) => startLogStream(...args),
    appendLogLines: (...args: Parameters<typeof actual.appendLogLines>) => appendLogLines(...args),
    prependLogLines: (...args: Parameters<typeof actual.prependLogLines>) => prependLogLines(...args),
  };
});

// Imported after the mock so the module under test picks up the mocked
// `startLogStream` rather than the real Tauri-backed one.
const { useLogStream, EARLIER_PAGE, MAX_LOADED_LINES } = await import("./logStream");

const target: LogTarget = { pod: "web-1", container: "app", label: "" };
const otherTarget: LogTarget = { pod: "web-2", container: "app", label: "" };
/** A labelled fan-out, the way `resolveLogSubject` builds one for a workload. */
const fanOut = (n: number): LogTarget[] =>
  Array.from({ length: n }, (_, i) => ({
    pod: `web-${i + 1}`,
    container: "app",
    label: `web-${i + 1}`,
  }));

/** A `startLogStream` double whose caller controls when the connect promise
 *  settles and can fire lines/status at will through the captured callbacks. */
function fakeStream() {
  let onLine!: (source: string, line: string, truncated: boolean) => void;
  let onStatus: ((status: LogStatus, source: string) => void) | undefined;
  const stop = vi.fn();
  let resolveConnect!: (v: { stop: () => void }) => void;
  let rejectConnect!: (e: unknown) => void;
  const connectPromise = new Promise<{ stop: () => void }>((resolve, reject) => {
    resolveConnect = resolve;
    rejectConnect = reject;
  });
  startLogStream.mockImplementation(
    (
      _context: string,
      _namespace: string,
      _targets: LogTarget[],
      line: (source: string, line: string, truncated: boolean) => void,
      status?: (s: LogStatus, source: string) => void,
    ) => {
      onLine = line;
      onStatus = status;
      return connectPromise;
    },
  );
  return {
    stop,
    line: (source: string, text: string, truncated = false) => onLine(source, text, truncated),
    /** Fire a status AS a given target — the tag the backend now sends. */
    status: (s: LogStatus, source = "") => onStatus?.(s, source),
    connect: () => act(async () => { resolveConnect({ stop }); await Promise.resolve(); }),
    reject: (e: unknown) => act(async () => { rejectConnect(e); await Promise.resolve(); }),
  };
}

beforeEach(() => {
  startLogStream.mockReset();
});

describe("useLogStream", () => {
  it("keeps the host's mark on a line it cut, and nothing on a whole one (#747)", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", fanOut(1)));
    await s.connect();
    act(() => {
      s.line("", "the start of it", true);
      s.line("", "whole");
    });
    await waitFor(() => expect(result.current.lines).toHaveLength(2));
    expect(result.current.lines).toStrictEqual([
      { source: "", text: "the start of it", truncated: true },
      { source: "", text: "whole" },
    ]);
  });

  it("retains completed init logs without counting them as reconnecting or streaming", async () => {
    const s = fakeStream();
    const targets = fanOut(2);
    const { result } = renderHook(() => useLogStream("kind-dev", "default", targets));
    await s.connect();
    act(() => {
      s.line("web-1", "setup complete");
      s.status("completed", "web-1");
      s.status("live", "web-2");
    });
    await waitFor(() => expect(result.current.lines).toHaveLength(1));
    expect(result.current.status).toBe("live");
    expect(result.current.liveTargets).toBe(1);
    expect(result.current.completedTargets).toBe(1);
    expect(result.current.reconnectingTargets).toBe(0);
    act(() => s.status("completed", "web-2"));
    expect(result.current.status).toBe("completed");
  });
  it("lands lines in order", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.connect();

    act(() => {
      s.line("", "one");
      s.line("", "two");
      s.line("", "three");
    });
    await waitFor(() => expect(result.current.lines).toHaveLength(3));
    expect(result.current.lines.map((l) => l.text)).toEqual(["one", "two", "three"]);
  });

  it("lands all hundred lines fired in one tick", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.connect();

    act(() => {
      for (let i = 0; i < 100; i++) s.line("", `line ${i}`);
    });
    await waitFor(() => expect(result.current.lines).toHaveLength(100));
    expect(result.current.lines[0].text).toBe("line 0");
    expect(result.current.lines[99].text).toBe("line 99");
  });

  it("folds a burst into the ring with one append, not one per line", async () => {
    // Each append copies the whole ring: 10,000 lines one at a time onto a full 5,000-line ring took ~80ms where one
    // batch took ~1ms, and the backend can deliver that many between two frames.
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.connect();
    appendLogLines.mockClear();

    act(() => {
      for (let i = 0; i < 100; i++) s.line("", `line ${i}`);
    });
    await waitFor(() => expect(result.current.lines).toHaveLength(100));
    expect(appendLogLines).toHaveBeenCalledTimes(1);
  });

  it("folds a backlog the size of the ring before its commit, and keeps the drop count exact", async () => {
    // A commit that is slow to come (a hidden window) must not let the lines waiting for it grow without bound.
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target], { capacity: 10 }));
    await s.connect();
    appendLogLines.mockClear();

    act(() => {
      for (let i = 0; i < 25; i++) s.line("", `line ${i}`);
    });
    // No commit has run yet: lines 0-9 and 10-19 are in the ring, 20-24 still wait.
    expect(appendLogLines).toHaveBeenCalledTimes(2);
    await waitFor(() => expect(result.current.dropped).toBe(15));
    expect(result.current.lines.map((l) => l.text)).toEqual(
      ["15", "16", "17", "18", "19", "20", "21", "22", "23", "24"].map((n) => `line ${n}`),
    );
    expect(appendLogLines).toHaveBeenCalledTimes(3);
  });

  it("holds the view and counts arrivals while paused", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.connect();

    act(() => s.line("", "before-pause"));
    await waitFor(() => expect(result.current.lines).toHaveLength(1));

    act(() => result.current.togglePause());
    expect(result.current.paused).toBe(true);

    act(() => {
      s.line("", "a");
      s.line("", "b");
      s.line("", "c");
    });
    await waitFor(() => expect(result.current.pendingWhilePaused).toBe(3));
    // The view is frozen: still only the one line that landed before pause.
    expect(result.current.lines).toHaveLength(1);
  });

  it("does not leak a commit into the view when pause lands before it flushes", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.connect();

    // "one" schedules a commit for the next frame that has not run yet when
    // pause is toggled immediately after, in the same synchronous span.
    act(() => {
      s.line("", "one");
      result.current.togglePause();
    });
    expect(result.current.paused).toBe(true);

    // Wait out the frame. If the scheduled commit still lands, the view moves
    // right after pausing — exactly what pause promises not to do.
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(result.current.lines).toHaveLength(0);
  });

  describe("committing once a frame", () => {
    afterEach(() => {
      vi.unstubAllGlobals();
      delete (document as { hidden?: boolean }).hidden;
    });

    /** Frames that run only when the test says so, and that a cancel takes back. */
    function frames() {
      const queued = new Map<number, FrameRequestCallback>();
      let next = 0;
      vi.stubGlobal("requestAnimationFrame", (run: FrameRequestCallback) => {
        queued.set(++next, run);
        return next;
      });
      vi.stubGlobal("cancelAnimationFrame", (id: number) => queued.delete(id));
      return {
        queued,
        run: () =>
          act(() => {
            const runs = [...queued.values()];
            queued.clear();
            runs.forEach((run) => run(0));
          }),
      };
    }

    it("lands lines that arrive over several ticks in the next frame, together", async () => {
      // A microtask per tick re-rendered the screen for every IPC event, hundreds of times between two frames.
      const frame = frames();
      const s = fakeStream();
      const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
      await s.connect();

      for (const text of ["one", "two", "three"]) {
        await act(async () => {
          s.line("", text);
          await Promise.resolve();
        });
      }
      expect(result.current.lines).toHaveLength(0);
      expect(frame.queued.size).toBe(1);
      frame.run();
      expect(result.current.lines.map((l) => l.text)).toEqual(["one", "two", "three"]);
    });

    it("counts a line that came in before a pause, but was not drawn yet, among the lines waiting", async () => {
      // Pause freezes the view it is pressed on. A line that had arrived but not been drawn is not in that view.
      const frame = frames();
      const s = fakeStream();
      const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
      await s.connect();
      act(() => {
        s.line("", "one");
        result.current.togglePause();
      });
      frame.run();
      expect(result.current.lines).toHaveLength(0);
      expect(result.current.pendingWhilePaused).toBe(1);
      act(() => s.line("", "two"));
      frame.run();
      expect(result.current.pendingWhilePaused).toBe(2);
    });

    it("gives back a frame it asked for when the screen goes before it is drawn", async () => {
      const frame = frames();
      const s = fakeStream();
      const { unmount } = renderHook(() => useLogStream("kind-dev", "default", [target]));
      await s.connect();
      act(() => s.line("", "one"));
      expect(frame.queued.size).toBe(1);
      unmount();
      expect(frame.queued.size).toBe(0);
    });

    it("gives back the timer a hidden document gets when the screen goes before it fires", async () => {
      vi.stubGlobal("requestAnimationFrame", undefined);
      const s = fakeStream();
      const { unmount } = renderHook(() => useLogStream("kind-dev", "default", [target]));
      await s.connect();
      vi.useFakeTimers();
      try {
        appendLogLines.mockClear();
        act(() => s.line("", "one"));
        unmount();
        vi.advanceTimersByTime(100);
        expect(appendLogLines).not.toHaveBeenCalled();
      } finally {
        vi.useRealTimers();
      }
    });

    it("still commits when there are no frames to wait for", async () => {
      vi.stubGlobal("requestAnimationFrame", undefined);
      const s = fakeStream();
      const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
      await s.connect();
      act(() => s.line("", "one"));
      await waitFor(() => expect(result.current.lines).toHaveLength(1));
    });

    describe("in a document nobody is looking at", () => {
      /** `document.hidden`, as the test says it is. The enclosing `afterEach` puts the real one back. */
      function visibility(initiallyHidden: boolean) {
        let hidden = initiallyHidden;
        Object.defineProperty(document, "hidden", { configurable: true, get: () => hidden });
        return {
          hide: () => {
            hidden = true;
            act(() => void document.dispatchEvent(new Event("visibilitychange")));
          },
          show: () => {
            hidden = false;
            act(() => void document.dispatchEvent(new Event("visibilitychange")));
          },
        };
      }
      /** A hidden document gets a timer of about a frame, so wait a few of them. */
      const aFrame = () => act(() => new Promise((r) => setTimeout(r, 50)));

      it("takes lines into the ring but does not show them: drawing 50,000 rows for nobody starved the browser", async () => {
        visibility(true);
        const s = fakeStream();
        const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
        await s.connect();
        appendLogLines.mockClear();
        act(() => {
          s.line("", "one");
          s.line("", "two");
        });
        await aFrame();
        // The timer path still folds, so a long-hidden tab holds a ring, not a backlog...
        expect(appendLogLines).toHaveBeenCalledTimes(1);
        // ...and the view, which every row of is a render, stays as it was.
        expect(result.current.lines).toHaveLength(0);
        act(() => s.line("", "three"));
        await aFrame();
        expect(result.current.lines).toHaveLength(0);
      });

      it("shows what it took in the moment the document is visible again", async () => {
        const doc = visibility(true);
        const s = fakeStream();
        const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
        await s.connect();
        act(() => {
          s.line("", "one");
          s.line("", "two");
        });
        await aFrame();
        act(() => s.line("", "three"));
        await aFrame();
        expect(result.current.lines).toHaveLength(0);

        doc.show();
        expect(result.current.lines.map((l) => l.text)).toEqual(["one", "two", "three"]);
        expect(result.current.pendingWhilePaused).toBe(0);
        // And it follows again from there.
        act(() => s.line("", "four"));
        await waitFor(() => expect(result.current.lines).toHaveLength(4));
      });

      it("takes back the frame it asked for before the document was hidden, and shows its lines on return", async () => {
        const frame = frames();
        const doc = visibility(false);
        const s = fakeStream();
        const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
        await s.connect();
        act(() => s.line("", "one"));
        expect(frame.queued.size).toBe(1);
        // The frame asked for never runs in a document that is hidden, and the line waits.
        doc.hide();
        expect(result.current.lines).toHaveLength(0);
        doc.show();
        expect(result.current.lines.map((l) => l.text)).toEqual(["one"]);
        expect(frame.queued.size).toBe(0);
      });

      type Hook = ReturnType<typeof useLogStream>;
      it.each([
        ["paused", (h: Hook) => h.togglePause(), (h: Hook) => h.togglePause()],
        ["held", (h: Hook) => h.hold(true), (h: Hook) => h.hold(false)],
      ])("leaves a %s view as it was when the document is visible again, and counts what waited", async (_, freeze, release) => {
        const doc = visibility(false);
        const s = fakeStream();
        const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
        await s.connect();
        act(() => s.line("", "before"));
        await waitFor(() => expect(result.current.lines).toHaveLength(1));
        act(() => freeze(result.current));
        // The document goes away, and lines arrive while it is.
        doc.hide();
        act(() => {
          s.line("", "a");
          s.line("", "b");
          s.line("", "c");
        });
        await aFrame();

        doc.show();
        expect(result.current.lines.map((l) => l.text)).toEqual(["before"]);
        expect(result.current.pendingWhilePaused).toBe(3);
        // Its own release is what lets them in.
        act(() => release(result.current));
        expect(result.current.lines.map((l) => l.text)).toEqual(["before", "a", "b", "c"]);
      });

      it("does not listen for the document being shown once the screen has gone", async () => {
        const added = vi.spyOn(document, "addEventListener");
        const removed = vi.spyOn(document, "removeEventListener");
        try {
          const s = fakeStream();
          const { unmount } = renderHook(() => useLogStream("kind-dev", "default", [target]));
          await s.connect();
          const listener = added.mock.calls.find(([type]) => type === "visibilitychange")?.[1];
          expect(listener).toBeDefined();
          unmount();
          expect(removed).toHaveBeenCalledWith("visibilitychange", listener);
        } finally {
          added.mockRestore();
          removed.mockRestore();
        }
      });
    });
  });

  it("holds the view while the screen asks, counts what arrives, and lets it in on release, leaving pause alone", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.connect();
    act(() => s.line("", "before"));
    await waitFor(() => expect(result.current.lines).toHaveLength(1));

    act(() => result.current.hold(true));
    expect(result.current.held).toBe(true);
    expect(result.current.paused).toBe(false);
    act(() => {
      s.line("", "a");
      s.line("", "b");
    });
    await waitFor(() => expect(result.current.pendingWhilePaused).toBe(2));
    expect(result.current.lines).toHaveLength(1);

    act(() => result.current.hold(false));
    expect(result.current.held).toBe(false);
    expect(result.current.lines.map((l) => l.text)).toEqual(["before", "a", "b"]);
    expect(result.current.pendingWhilePaused).toBe(0);
  });

  it("keeps the reader's pause and a hold apart: letting go of one does not let go of the other", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.connect();
    const aFrame = () => act(() => new Promise((r) => setTimeout(r, 50)));

    // Paused, then held and let go: still paused, and nothing let in.
    act(() => result.current.togglePause());
    act(() => result.current.hold(true));
    act(() => s.line("", "a"));
    act(() => result.current.hold(false));
    await aFrame();
    expect(result.current.paused).toBe(true);
    expect(result.current.lines).toHaveLength(0);
    expect(result.current.pendingWhilePaused).toBe(1);

    // Held, then resumed: the reader is following again, but nothing is let in until the hold goes.
    act(() => result.current.hold(true));
    act(() => result.current.togglePause());
    await aFrame();
    expect(result.current.paused).toBe(false);
    expect(result.current.lines).toHaveLength(0);
    act(() => result.current.hold(false));
    expect(result.current.lines.map((l) => l.text)).toEqual(["a"]);
  });

  it("does not re-tail on resume, and folds in what arrived while paused", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.connect();
    expect(startLogStream).toHaveBeenCalledTimes(1);

    act(() => result.current.togglePause());
    act(() => {
      s.line("", "a");
      s.line("", "b");
    });
    await waitFor(() => expect(result.current.pendingWhilePaused).toBe(2));

    act(() => result.current.togglePause());
    expect(result.current.paused).toBe(false);
    expect(startLogStream).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(result.current.lines).toHaveLength(2));
    expect(result.current.pendingWhilePaused).toBe(0);
  });

  it("stops the stream on unmount", async () => {
    const s = fakeStream();
    const { unmount } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.connect();

    unmount();
    expect(s.stop).toHaveBeenCalledTimes(1);
  });

  it("stops a stream whose connect promise resolves after unmount", async () => {
    const s = fakeStream();
    const { unmount } = renderHook(() => useLogStream("kind-dev", "default", [target]));

    // Unmount before the connect promise settles — the dangerous window.
    unmount();
    await s.connect();

    expect(s.stop).toHaveBeenCalledTimes(1);
  });

  it("surfaces a status change", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    expect(result.current.status).toBe("connecting");
    await s.connect();
    await waitFor(() => expect(result.current.status).toBe("connecting"));

    act(() => s.status("live"));
    await waitFor(() => expect(result.current.status).toBe("live"));

    act(() => s.status("reconnecting"));
    await waitFor(() => expect(result.current.status).toBe("reconnecting"));
  });

  it("treats an empty target list as a quiet not-yet state, never calling startLogStream or erroring", async () => {
    // Mirrors the real `startLogStream`'s guard (`packages/core/src/lib/logsStream.ts`):
    // it throws a plain string on an empty target list that `describeError`
    // can't classify. If the hook ever called it with `[]`, this mock would
    // reproduce that exact failure.
    startLogStream.mockImplementation(() =>
      Promise.reject(new Error("cannot start live logs without a pod target")),
    );
    const { result } = renderHook(() => useLogStream("kind-dev", "default", []));

    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(startLogStream).not.toHaveBeenCalled();
    expect(result.current.status).not.toBe("error");
    expect(result.current.error).toBeUndefined();
  });

  it("connects for the first time, not a restart, once targets arrive after starting empty", async () => {
    const s = fakeStream();
    const { result, rerender } = renderHook(
      ({ targets }: { targets: LogTarget[] }) => useLogStream("kind-dev", "default", targets),
      { initialProps: { targets: [] as LogTarget[] } },
    );
    expect(startLogStream).not.toHaveBeenCalled();

    rerender({ targets: [target] });
    await s.connect();

    expect(startLogStream).toHaveBeenCalledTimes(1);
    expect(result.current.restartCount).toBe(0);
  });

  it("flags the stream reconnecting the moment ONE target of many drops out", async () => {
    // The question the indicator answers is "am I seeing everything?", and
    // the answer while one of three pods is down is no. The counts say how
    // much of the tail is still moving, so one blip never has to read as a
    // total outage.
    const s = fakeStream();
    const targets = fanOut(3);
    const { result } = renderHook(() => useLogStream("kind-dev", "default", targets));
    await s.connect();

    act(() => {
      s.status("live", "web-1");
      s.status("live", "web-2");
      s.status("live", "web-3");
    });
    await waitFor(() => expect(result.current.status).toBe("live"));
    expect(result.current.liveTargets).toBe(3);

    act(() => s.status("reconnecting", "web-2"));
    await waitFor(() => expect(result.current.status).toBe("reconnecting"));
    expect(result.current.liveTargets).toBe(2);
    expect(result.current.reconnectingTargets).toBe(1);
    expect(result.current.totalTargets).toBe(3);

    act(() => s.status("live", "web-2"));
    await waitFor(() => expect(result.current.status).toBe("live"));
    expect(result.current.liveTargets).toBe(3);
    expect(result.current.reconnectingTargets).toBe(0);
  });

  it("surfaces a down target on a ten-pod workload at once, not after ten backoff cycles", async () => {
    // What the streak counter cost: a persistently-down target needed as many
    // consecutive drop signals as the stream had targets before it surfaced —
    // about two seconds each. One target's own report is enough now.
    const s = fakeStream();
    const targets = fanOut(10);
    const { result } = renderHook(() => useLogStream("kind-dev", "default", targets));
    await s.connect();

    act(() => s.status("reconnecting", "web-10"));
    await waitFor(() => expect(result.current.status).toBe("reconnecting"));
    expect(result.current.reconnectingTargets).toBe(1);
    expect(result.current.totalTargets).toBe(10);
  });

  it("does not let a healthy target's success clear another target's outage", async () => {
    // The streak counter reset on any "live", so a target flapping between up
    // and down could hide behind its neighbours forever. A drop is now the
    // dropped target's own state, and only that target can clear it.
    const s = fakeStream();
    const targets = fanOut(3);
    const { result } = renderHook(() => useLogStream("kind-dev", "default", targets));
    await s.connect();

    act(() => s.status("reconnecting", "web-1"));
    await waitFor(() => expect(result.current.status).toBe("reconnecting"));

    act(() => {
      s.status("live", "web-2");
      s.status("live", "web-3");
    });
    await waitFor(() => expect(result.current.liveTargets).toBe(2));
    expect(result.current.status).toBe("reconnecting");
    expect(result.current.reconnectingTargets).toBe(1);
  });

  it("keeps a target's LATEST status, not a tally of its events", async () => {
    // One target retrying every couple of seconds emits a drop per cycle. It
    // is still one target down, not four.
    const s = fakeStream();
    const targets = fanOut(2);
    const { result } = renderHook(() => useLogStream("kind-dev", "default", targets));
    await s.connect();

    act(() => {
      s.status("live", "web-1");
      s.status("reconnecting", "web-2");
      s.status("reconnecting", "web-2");
      s.status("reconnecting", "web-2");
    });
    await waitFor(() => expect(result.current.status).toBe("reconnecting"));
    expect(result.current.reconnectingTargets).toBe(1);
    expect(result.current.liveTargets).toBe(1);
    expect(result.current.totalTargets).toBe(2);
  });

  it("stays connecting until every target has reported, never calling a partial tail live", async () => {
    const s = fakeStream();
    const targets = fanOut(2);
    const { result } = renderHook(() => useLogStream("kind-dev", "default", targets));
    await s.connect();

    act(() => s.status("live", "web-1"));
    await waitFor(() => expect(result.current.liveTargets).toBe(1));
    expect(result.current.status).toBe("connecting");

    act(() => s.status("live", "web-2"));
    await waitFor(() => expect(result.current.status).toBe("live"));
  });

  it("counts a repeated label once rather than waiting forever for a target that cannot report", async () => {
    // A source tag is how a target identifies itself; two targets sharing one
    // tag are one source to everything downstream, lines included. Counting
    // them as two would leave the aggregate stuck below its own denominator.
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target, otherTarget]));
    await s.connect();

    expect(result.current.totalTargets).toBe(1);
    act(() => s.status("live", ""));
    await waitFor(() => expect(result.current.status).toBe("live"));
  });

  it("forgets what it knew about the old targets when the stream restarts", async () => {
    const s = fakeStream();
    const { result, rerender } = renderHook(
      ({ targets }: { targets: LogTarget[] }) => useLogStream("kind-dev", "default", targets),
      { initialProps: { targets: fanOut(2) } },
    );
    await s.connect();
    act(() => {
      s.status("live", "web-1");
      s.status("live", "web-2");
    });
    await waitFor(() => expect(result.current.status).toBe("live"));

    const s2 = fakeStream();
    rerender({ targets: [target] });
    await s2.connect();

    await waitFor(() => expect(result.current.status).toBe("connecting"));
    expect(result.current.liveTargets).toBe(0);
    expect(result.current.reconnectingTargets).toBe(0);
    expect(result.current.totalTargets).toBe(1);
  });

  it("surfaces a start failure through describeError", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.reject(new Error("connection refused"));

    await waitFor(() => expect(result.current.status).toBe("error"));
    expect(result.current.error?.title).toBe("Can't reach the cluster");
  });

  it("restarts and clears the buffer when targets change, and says so", async () => {
    const s = fakeStream();
    const { result, rerender } = renderHook(
      ({ targets }: { targets: LogTarget[] }) => useLogStream("kind-dev", "default", targets),
      { initialProps: { targets: [target] } },
    );
    await s.connect();
    act(() => s.line("", "before-restart"));
    await waitFor(() => expect(result.current.lines).toHaveLength(1));
    expect(result.current.restartCount).toBe(0);

    const s2 = fakeStream();
    rerender({ targets: [otherTarget] });
    await s2.connect();

    await waitFor(() => expect(result.current.restartCount).toBe(1));
    expect(result.current.lines).toHaveLength(0);
    expect(result.current.dropped).toBe(0);
    expect(startLogStream).toHaveBeenCalledTimes(2);
  });

  it("ignores a line from a stream cancelled by a dependency change before its connect promise settles", async () => {
    // Narrowing `sinceSeconds` (or changing container/tailLines) tears down
    // the in-flight connect and starts a new one. The old stream's connect
    // promise has not resolved yet, so cleanup cannot call `stop()` on it —
    // there is nothing to stop yet. If the old stream's initial tail line
    // arrives on the channel before its stray `.then()` gets around to
    // stopping it, that line must NOT land in the buffer the new effect just
    // cleared.
    const s1 = fakeStream();
    const { result, rerender } = renderHook(
      ({ sinceSeconds }: { sinceSeconds?: number }) =>
        useLogStream("kind-dev", "default", [target], { sinceSeconds }),
      { initialProps: { sinceSeconds: undefined as number | undefined } },
    );
    expect(startLogStream).toHaveBeenCalledTimes(1);
    // s1's connect promise is deliberately left unresolved here.

    const s2 = fakeStream();
    rerender({ sinceSeconds: 300 });
    expect(startLogStream).toHaveBeenCalledTimes(2);

    // The stale stream's initial tail line lands after the dependency change
    // cancelled it, but before its connect promise resolves and stop() runs.
    act(() => s1.line("", "stale-tail-line"));

    await s2.connect();
    act(() => s2.line("", "fresh-line"));
    await waitFor(() => expect(result.current.lines).toHaveLength(1));
    expect(result.current.lines[0].text).toBe("fresh-line");

    // The stale stream resolving late must still be stopped (existing
    // behaviour), but must not have contributed any lines.
    await s1.connect();
    expect(s1.stop).toHaveBeenCalledTimes(1);
    expect(result.current.lines).toHaveLength(1);
  });

  it("clears the buffer on demand without restarting the stream", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.connect();
    act(() => s.line("", "one"));
    await waitFor(() => expect(result.current.lines).toHaveLength(1));

    act(() => result.current.clear());
    expect(result.current.lines).toHaveLength(0);
    expect(startLogStream).toHaveBeenCalledTimes(1);
  });

  it("does not bring back a line that arrived before a clear, though it had not been committed yet", async () => {
    const s = fakeStream();
    const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
    await s.connect();
    act(() => {
      s.line("", "before-clear");
      result.current.clear();
      s.line("", "after-clear");
    });
    await waitFor(() => expect(result.current.lines).toHaveLength(1));
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(result.current.lines.map((l) => l.text)).toEqual(["after-clear"]);
  });

  it("does not carry a line the old stream sent, not yet committed, into the restarted one", async () => {
    const s = fakeStream();
    const { result, rerender } = renderHook(
      ({ targets }: { targets: LogTarget[] }) => useLogStream("kind-dev", "default", targets),
      { initialProps: { targets: [target] } },
    );
    await s.connect();
    const s2 = fakeStream();
    act(() => {
      s.line("", "old-stream");
      rerender({ targets: [otherTarget] });
    });
    await s2.connect();
    act(() => s2.line("", "new-stream"));
    await waitFor(() => expect(result.current.lines).toHaveLength(1));
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(result.current.lines.map((l) => l.text)).toEqual(["new-stream"]);
  });
});

// #567: an app's pod binding streams logs over its own source, and the pod log
// view follows it through the same buffer, pause and status counting. A log
// provider (#569) plugs in the same way.
describe("useLogStream with another log source", () => {
  it("opens the source it is given, not the cluster's own, and lands its lines", async () => {
    startLogStream.mockReset();
    let onLine!: (source: string, line: string, truncated: boolean) => void;
    let onStatus!: (status: LogStatus, source: string) => void;
    const stop = vi.fn();
    const open = vi.fn(async (_targets: LogTarget[], line: typeof onLine, status: typeof onStatus, _options: unknown) => {
      onLine = line;
      onStatus = status;
      return { stop };
    });
    const labelled: LogTarget = { pod: "web-1", container: "app", label: "web-1/app" };
    const { result, unmount } = renderHook(() =>
      useLogStream("ctx", "team", [labelled], { tailLines: 50, source: { key: "app:org.example/logs", open } }),
    );
    await waitFor(() => expect(open).toHaveBeenCalledTimes(1));
    expect(startLogStream).not.toHaveBeenCalled();
    expect(open.mock.calls[0][0]).toEqual([labelled]);
    expect(open.mock.calls[0][3]).toMatchObject({ tailLines: 50 });
    act(() => {
      onStatus("live", "web-1/app");
      onLine("web-1/app", "hello", false);
    });
    await waitFor(() => expect(result.current.lines.map((l) => l.text)).toEqual(["hello"]));
    expect(result.current.status).toBe("live");
    unmount();
    expect(stop).toHaveBeenCalled();
  });
});

/** A stream that keeps history: the caller fires frames and decides what a read returns. */
function fakeSpooledStream() {
  let options!: LogStreamOptions;
  const stop = vi.fn();
  const readEarlier = vi.fn<(before: number, limit: number) => Promise<SpoolPage>>();
  const exportSpool = vi.fn(async () => "/home/u/web.log");
  startLogStream.mockImplementation(
    (_c: string, _n: string, _t: LogTarget[], _line: unknown, _status: unknown, opts: LogStreamOptions) => {
      options = opts;
      return Promise.resolve({ stop, readEarlier, exportSpool });
    },
  );
  const lines = (texts: string[]) => texts.map((text) => ({ source: "", text }));
  return {
    readEarlier,
    exportSpool,
    options: () => options,
    frame: (seq: number, texts: string[], first = 1) => act(() => options.onLines!(lines(texts), { seq, first })),
    page: (seq: number, texts: string[], first = 1): SpoolPage => ({ first, seq, lines: lines(texts) }),
    failed: (reason: string) => act(() => options.onSpoolFailed!(reason)),
  };
}

const texts = (r: { current: { lines: readonly { text: string }[] } }) => r.current.lines.map((l) => l.text);
/** `n` line texts, `l<from>` onwards: the text of line `from` is `l<from>`. */
const many = (from: number, n: number) => Array.from({ length: n }, (_, i) => `l${from + i}`);

/** A hook on a stream that keeps history, once the stream is open. */
async function openSpooled(options: UseLogStreamOptions = {}) {
  const s = fakeSpooledStream();
  const hook = renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024, ...options }));
  await waitFor(() => expect(startLogStream).toHaveBeenCalled());
  return { s, ...hook };
}

/** The next read waits until the returned function gives it a page. */
function holdRead(s: ReturnType<typeof fakeSpooledStream>) {
  let arrive!: (page: SpoolPage) => void;
  s.readEarlier.mockReturnValue(new Promise<SpoolPage>((resolve) => (arrive = resolve)));
  return (page: SpoolPage) => arrive(page);
}

describe("useLogStream with history on disk (#914)", () => {
  it("asks for history and frames only when told to", async () => {
    const s = fakeSpooledStream();
    renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024 }));
    await waitFor(() => expect(startLogStream).toHaveBeenCalled());
    expect(s.options().spoolMaxBytes).toBe(1024);
    expect(typeof s.options().onLines).toBe("function");

    startLogStream.mockReset();
    const t = fakeSpooledStream();
    renderHook(() => useLogStream("ctx", "ns", [target]));
    await waitFor(() => expect(startLogStream).toHaveBeenCalled());
    expect(t.options().spoolMaxBytes).toBeUndefined();
    expect(t.options().onLines).toBeUndefined();
  });

  it("shows a frame's lines and where history starts", async () => {
    const s = fakeSpooledStream();
    const { result } = renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024 }));
    await waitFor(() => expect(startLogStream).toHaveBeenCalled());
    s.frame(1, ["a", "b"]);
    await waitFor(() => expect(texts(result)).toEqual(["a", "b"]));
    expect(result.current.history).toEqual({ first: 1 });
    expect(result.current.dropped).toBe(0);
  });

  it("counts the lines a frame skipped as dropped and starts again at the frame", async () => {
    const s = fakeSpooledStream();
    const { result } = renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024 }));
    await waitFor(() => expect(startLogStream).toHaveBeenCalled());
    s.frame(1, ["a", "b"]);
    s.frame(10, ["j"]);
    await waitFor(() => expect(texts(result)).toEqual(["j"]));
    expect(result.current.dropped).toBe(9);
  });

  it("loads the page before the oldest line shown and lowers dropped by as many", async () => {
    const s = fakeSpooledStream();
    const { result } = renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024 }));
    await waitFor(() => expect(startLogStream).toHaveBeenCalled());
    s.frame(4, ["d", "e"]);
    await waitFor(() => expect(texts(result)).toEqual(["d", "e"]));
    s.readEarlier.mockResolvedValueOnce(s.page(1, ["a", "b", "c"]));
    await act(() => result.current.loadEarlier());
    expect(s.readEarlier).toHaveBeenCalledWith(4, 5000);
    expect(texts(result)).toEqual(["a", "b", "c", "d", "e"]);
    expect(result.current.dropped).toBe(0);
  });

  it("keeps loaded lines while new ones arrive: a load leaves a page of room above them", async () => {
    const s = fakeSpooledStream();
    const { result } = renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024 }));
    await waitFor(() => expect(startLogStream).toHaveBeenCalled());
    s.frame(5001, many(5001, 5000));
    await waitFor(() => expect(result.current.lines.length).toBe(5000));
    s.readEarlier.mockResolvedValueOnce(s.page(1, many(1, 5000)));
    await act(() => result.current.loadEarlier());
    expect(result.current.lines.length).toBe(10000);
    s.frame(10001, ["new"]);
    await waitFor(() => expect(result.current.lines.length).toBe(10001));
    expect(result.current.lines[0].text).toBe("l1");
  });

  it("while paused, loads into the frozen view and leaves the pending count alone", async () => {
    const s = fakeSpooledStream();
    const { result } = renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024 }));
    await waitFor(() => expect(startLogStream).toHaveBeenCalled());
    s.frame(4, ["d", "e"]);
    await waitFor(() => expect(texts(result)).toEqual(["d", "e"]));
    act(() => result.current.togglePause());
    s.frame(6, ["f"]);
    await waitFor(() => expect(result.current.pendingWhilePaused).toBe(1));
    s.readEarlier.mockResolvedValueOnce(s.page(1, ["a", "b", "c"]));
    await act(() => result.current.loadEarlier());
    expect(texts(result)).toEqual(["a", "b", "c", "d", "e"]);
    expect(result.current.pendingWhilePaused).toBe(1);
    act(() => result.current.togglePause());
    expect(texts(result)).toEqual(["a", "b", "c", "d", "e", "f"]);
  });

  it("keeps the view frozen when the reader pauses while a page is being read", async () => {
    const s = fakeSpooledStream();
    const { result } = renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024 }));
    await waitFor(() => expect(startLogStream).toHaveBeenCalled());
    s.frame(4, ["d", "e"]);
    await waitFor(() => expect(texts(result)).toEqual(["d", "e"]));
    let arrive!: (page: SpoolPage) => void;
    s.readEarlier.mockReturnValueOnce(new Promise<SpoolPage>((resolve) => (arrive = resolve)));
    let load!: Promise<void>;
    act(() => {
      load = result.current.loadEarlier();
    });
    act(() => result.current.togglePause());
    s.frame(6, ["f"]);
    await waitFor(() => expect(result.current.pendingWhilePaused).toBe(1));
    await act(async () => {
      arrive(s.page(1, ["a", "b", "c"]));
      await load;
    });
    // The page joins the frozen view, but the line that came in during the pause stays out of it.
    expect(texts(result)).toEqual(["a", "b", "c", "d", "e"]);
    expect(result.current.pendingWhilePaused).toBe(1);
    act(() => result.current.togglePause());
    expect(texts(result)).toEqual(["a", "b", "c", "d", "e", "f"]);
  });

  it("does not show a page that no longer joins the lines shown, and says why", async () => {
    const s = fakeSpooledStream();
    const { result } = renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024 }));
    await waitFor(() => expect(startLogStream).toHaveBeenCalled());
    s.frame(4, ["d"]);
    await waitFor(() => expect(texts(result)).toEqual(["d"]));
    s.readEarlier.mockResolvedValueOnce(s.page(1, ["a", "b"])); // ends at 3, not 4
    await act(() => result.current.loadEarlier());
    expect(texts(result)).toEqual(["d"]);
    expect(result.current.loadError).toBeInstanceOf(Error);
  });

  it("reports a failed read and keeps the lines", async () => {
    const s = fakeSpooledStream();
    const { result } = renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024 }));
    await waitFor(() => expect(startLogStream).toHaveBeenCalled());
    s.frame(4, ["d"]);
    await waitFor(() => expect(texts(result)).toEqual(["d"]));
    s.readEarlier.mockRejectedValueOnce(new Error("this log stream has ended"));
    await act(() => result.current.loadEarlier());
    expect(texts(result)).toEqual(["d"]);
    expect(String(result.current.loadError)).toContain("has ended");
    expect(result.current.loadingEarlier).toBe(false);
  });

  it("clear counts the cleared lines as on disk", async () => {
    const s = fakeSpooledStream();
    const { result } = renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024 }));
    await waitFor(() => expect(startLogStream).toHaveBeenCalled());
    s.frame(1, ["a", "b", "c"]);
    await waitFor(() => expect(texts(result)).toEqual(["a", "b", "c"]));
    act(() => result.current.clear());
    expect(texts(result)).toEqual([]);
    expect(result.current.dropped).toBe(3);
  });

  it("reports when history stopped, and exports through the stream", async () => {
    const s = fakeSpooledStream();
    const { result } = renderHook(() => useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024 }));
    await waitFor(() => expect(result.current.exportSpool).toBeDefined());
    await expect(result.current.exportSpool!("web.log", 600)).resolves.toBe("/home/u/web.log");
    expect(s.exportSpool).toHaveBeenCalledWith("web.log", 600);
    s.frame(1, ["a"]);
    s.failed("No space left on device");
    await waitFor(() => expect(result.current.history).toEqual({ first: 1, failed: "No space left on device" }));
  });

  it("keeps the oldest line shown in a full ring while a page is read and lines arrive", async () => {
    const { s, result } = await openSpooled();
    s.frame(5001, many(5001, 5000));
    await waitFor(() => expect(result.current.lines.length).toBe(5000));
    const arrive = holdRead(s);
    let load!: Promise<void>;
    act(() => {
      load = result.current.loadEarlier();
    });
    s.frame(10001, ["new"]);
    await waitFor(() => expect(texts(result).at(-1)).toBe("new"));
    await act(async () => {
      arrive(s.page(1, many(1, 5000)));
      await load;
    });
    expect(result.current.loadError).toBeUndefined();
    expect(result.current.lines.length).toBe(10001);
    expect(texts(result).slice(4999, 5002)).toEqual(["l5000", "l5001", "l5002"]);
    expect(texts(result).at(-1)).toBe("new");
    expect(result.current.dropped).toBe(0);
  });

  it("takes back the room it made for a page when the read fails, and loses no line doing so", async () => {
    const { s, result } = await openSpooled();
    s.frame(5001, many(5001, 5000));
    await waitFor(() => expect(result.current.lines.length).toBe(5000));
    let fail!: (e: unknown) => void;
    s.readEarlier.mockReturnValueOnce(new Promise<SpoolPage>((_, reject) => (fail = reject)));
    let load!: Promise<void>;
    act(() => {
      load = result.current.loadEarlier();
    });
    s.frame(10001, ["during"]);
    await waitFor(() => expect(result.current.lines.length).toBe(5001));
    await act(async () => {
      fail(new Error("this log stream has ended"));
      await load;
    });
    expect(result.current.loadError).toBeDefined();
    // The line that came while the page was read is still there: the ring was not cut back to its old size.
    expect(result.current.lines.length).toBe(5001);
    // And the room is gone: another line pushes the oldest out, where a ring a page wider would just grow.
    s.frame(10002, ["after"]);
    await waitFor(() => expect(texts(result).at(-1)).toBe("after"));
    expect(result.current.lines.length).toBe(5001);
    expect(result.current.dropped).toBe(5001);
  });

  it("does not let repeated failed reads grow the ring", async () => {
    const { s, result } = await openSpooled();
    s.frame(5001, many(5001, 5000));
    await waitFor(() => expect(result.current.lines.length).toBe(5000));
    for (let i = 0; i < 12; i++) {
      s.readEarlier.mockRejectedValueOnce(new Error("this log stream has ended"));
      await act(() => result.current.loadEarlier());
    }
    // Twelve pre-raises of a page would have taken a ring of 5,000 to its 50,000 ceiling.
    s.frame(10001, many(10001, 100));
    await waitFor(() => expect(texts(result).at(-1)).toBe("l10100"));
    expect(result.current.lines.length).toBe(5000);
    expect(result.current.dropped).toBe(5100);
  });

  it("takes back the room it made when the page does not join the lines in memory", async () => {
    const { s, result } = await openSpooled();
    s.frame(5001, many(5001, 5000));
    await waitFor(() => expect(result.current.lines.length).toBe(5000));
    // Ends at 4,001, not at the 5,001 the ring starts from.
    s.readEarlier.mockResolvedValueOnce(s.page(1, many(1, 4000)));
    await act(() => result.current.loadEarlier());
    expect(result.current.loadError).toBeInstanceOf(Error);
    s.frame(10001, ["after"]);
    await waitFor(() => expect(texts(result).at(-1)).toBe("after"));
    expect(result.current.lines.length).toBe(5000);
    expect(result.current.dropped).toBe(5001);
  });

  it("takes back the room it made when the page comes back empty", async () => {
    const { s, result } = await openSpooled();
    s.frame(5001, many(5001, 5000));
    await waitFor(() => expect(result.current.lines.length).toBe(5000));
    // The history had rotated past everything asked for.
    s.readEarlier.mockResolvedValueOnce(s.page(5001, []));
    await act(() => result.current.loadEarlier());
    s.frame(10001, ["after"]);
    await waitFor(() => expect(texts(result).at(-1)).toBe("after"));
    expect(result.current.lines.length).toBe(5000);
  });

  it("takes back the room it made when the reader paused during the read and the page no longer joins the view", async () => {
    const { s, result } = await openSpooled();
    s.frame(5001, many(5001, 5000));
    await waitFor(() => expect(result.current.lines.length).toBe(5000));
    const arrive = holdRead(s);
    let load!: Promise<void>;
    act(() => {
      load = result.current.loadEarlier();
    });
    act(() => result.current.togglePause());
    await act(async () => {
      arrive(s.page(1, many(1, 4000)));
      await load;
    });
    expect(result.current.loadError).toBeInstanceOf(Error);
    act(() => result.current.togglePause());
    s.frame(10001, ["after"]);
    await waitFor(() => expect(texts(result).at(-1)).toBe("after"));
    expect(result.current.lines.length).toBe(5000);
  });

  it("does not ask another source for history", async () => {
    const open = vi.fn(async (_targets: LogTarget[], _line: unknown, _status: unknown, _options: LogStreamOptions) => ({
      stop: vi.fn(),
    }));
    const { result } = renderHook(() =>
      useLogStream("ctx", "ns", [target], { spoolMaxBytes: 1024, source: { key: "app:org.example/logs", open } }),
    );
    await waitFor(() => expect(open).toHaveBeenCalledTimes(1));
    const given = open.mock.calls[0][3];
    expect(given).not.toHaveProperty("spoolMaxBytes");
    expect(given).not.toHaveProperty("onLines");
    expect(given).not.toHaveProperty("onSpoolFailed");
    expect(startLogStream).not.toHaveBeenCalled();
    expect(result.current.history).toBeUndefined();
    expect(result.current.exportSpool).toBeUndefined();
  });

  it("while held, loads into the frozen view just as when paused", async () => {
    const { s, result } = await openSpooled();
    s.frame(4, ["d", "e"]);
    await waitFor(() => expect(texts(result)).toEqual(["d", "e"]));
    act(() => result.current.hold(true));
    s.frame(6, ["f"]);
    await waitFor(() => expect(result.current.pendingWhilePaused).toBe(1));
    s.readEarlier.mockResolvedValueOnce(s.page(1, ["a", "b", "c"]));
    await act(() => result.current.loadEarlier());
    expect(texts(result)).toEqual(["a", "b", "c", "d", "e"]);
    expect(result.current.pendingWhilePaused).toBe(1);
    act(() => result.current.hold(false));
    expect(texts(result)).toEqual(["a", "b", "c", "d", "e", "f"]);
    expect(result.current.pendingWhilePaused).toBe(0);
  });

  it("reads nothing once the view holds as many lines as it may", async () => {
    const { s, result } = await openSpooled({ capacity: MAX_LOADED_LINES });
    s.frame(1001, many(1001, MAX_LOADED_LINES));
    await waitFor(() => expect(result.current.lines.length).toBe(MAX_LOADED_LINES));
    expect(result.current.dropped).toBe(1000);
    // Were it asked, the answer would be an empty page: what fails below is the read, not a crash.
    s.readEarlier.mockResolvedValue(s.page(1, []));
    await act(() => result.current.loadEarlier());
    expect(s.readEarlier).not.toHaveBeenCalled();
    expect(result.current.loadingEarlier).toBe(false);
    expect(result.current.loadError).toBeUndefined();
  });

  it("asks only for as many earlier lines as still fit under that limit", async () => {
    const { s, result } = await openSpooled({ capacity: MAX_LOADED_LINES });
    s.frame(9001, many(9001, MAX_LOADED_LINES - 2000));
    await waitFor(() => expect(result.current.lines.length).toBe(MAX_LOADED_LINES - 2000));
    s.readEarlier.mockResolvedValueOnce(s.page(7001, many(7001, 2000)));
    await act(() => result.current.loadEarlier());
    expect(s.readEarlier).toHaveBeenCalledWith(9001, 2000);
    expect(result.current.lines.length).toBe(MAX_LOADED_LINES);
    expect(result.current.dropped).toBe(7000);
  });

  it("while paused, still shows a page in the view when the ring has moved on past the view's start", async () => {
    const { s, result } = await openSpooled({ capacity: 10 });
    s.frame(4, ["d", "e"]);
    await waitFor(() => expect(texts(result)).toEqual(["d", "e"]));
    act(() => result.current.togglePause());
    // More than the ring holds: it keeps the last ten, and no longer starts where the view does.
    s.frame(6, many(6, 15));
    await waitFor(() => expect(result.current.pendingWhilePaused).toBe(15));
    s.readEarlier.mockResolvedValueOnce(s.page(1, ["a", "b", "c"]));
    await act(() => result.current.loadEarlier());
    expect(texts(result)).toEqual(["a", "b", "c", "d", "e"]);
    expect(result.current.pendingWhilePaused).toBe(15);
    // The ring did not take the page: it would leave a hole between the page and its lines.
    act(() => result.current.togglePause());
    expect(texts(result)).toEqual(many(11, 10));
    expect(result.current.dropped).toBe(10);
  });

  it("ignores a page that arrives after the screen has gone", async () => {
    const { s, result, unmount } = await openSpooled();
    s.frame(4, ["d", "e"]);
    await waitFor(() => expect(texts(result)).toEqual(["d", "e"]));
    const arrive = holdRead(s);
    let load!: Promise<void>;
    act(() => {
      load = result.current.loadEarlier();
    });
    unmount();
    prependLogLines.mockClear();
    await act(async () => {
      arrive(s.page(1, ["a", "b", "c"]));
      await load;
    });
    expect(prependLogLines).not.toHaveBeenCalled();
  });

  it("keeps the same exportSpool while frames arrive", async () => {
    const { s, result } = await openSpooled();
    await waitFor(() => expect(result.current.exportSpool).toBeDefined());
    const exportSpool = result.current.exportSpool;
    s.frame(1, ["a"]);
    await waitFor(() => expect(texts(result)).toEqual(["a"]));
    s.frame(2, ["b"]);
    await waitFor(() => expect(texts(result)).toEqual(["a", "b"]));
    expect(result.current.exportSpool).toBe(exportSpool);
  });

  it("starts again at a gap without copying the ring for lines it is about to drop", async () => {
    const { s, result } = await openSpooled();
    appendLogLines.mockClear();
    s.frame(1, ["a", "b"]);
    s.frame(10, ["j"]);
    await waitFor(() => expect(texts(result)).toEqual(["j"]));
    expect(result.current.dropped).toBe(9);
    // Only the frame that starts the ring again is folded in; the lines before the gap are on disk.
    expect(appendLogLines).toHaveBeenCalledTimes(1);
  });

  it("ignores a second request for earlier lines while one is being read", async () => {
    const { s, result } = await openSpooled();
    s.frame(4, ["d", "e"]);
    await waitFor(() => expect(texts(result)).toEqual(["d", "e"]));
    const arrive = holdRead(s);
    let first!: Promise<void>;
    let second!: Promise<void>;
    act(() => {
      first = result.current.loadEarlier();
      second = result.current.loadEarlier();
    });
    expect(s.readEarlier).toHaveBeenCalledTimes(1);
    await act(async () => {
      arrive(s.page(1, ["a", "b", "c"]));
      await Promise.all([first, second]);
    });
    expect(texts(result)).toEqual(["a", "b", "c", "d", "e"]);
    expect(result.current.loadError).toBeUndefined();
    expect(result.current.loadingEarlier).toBe(false);
  });

  it("offers a caller only the options the hook uses", () => {
    // Checked by `tsc`, not at run time: these two are set by the hook itself.
    expectTypeOf<UseLogStreamOptions>().not.toHaveProperty("onLines");
    expectTypeOf<UseLogStreamOptions>().not.toHaveProperty("onSpoolFailed");
  });
});
