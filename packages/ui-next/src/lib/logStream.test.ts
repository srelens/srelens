import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { LogStatus, LogTarget } from "@srelens/core";

const startLogStream = vi.fn();
/** Calls through to the real ring; counts how often the hook folds lines into it. */
const appendLogLines = vi.fn();

vi.mock("@srelens/core", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/core")>();
  appendLogLines.mockImplementation(actual.appendLogLines);
  return {
    ...actual,
    startLogStream: (...args: unknown[]) => startLogStream(...args),
    appendLogLines: (...args: Parameters<typeof actual.appendLogLines>) => appendLogLines(...args),
  };
});

// Imported after the mock so the module under test picks up the mocked
// `startLogStream` rather than the real Tauri-backed one.
const { useLogStream } = await import("./logStream");

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

    /** Frames that run only when the test says so. */
    function frames() {
      const queued: FrameRequestCallback[] = [];
      vi.stubGlobal("requestAnimationFrame", (run: FrameRequestCallback) => queued.push(run));
      return {
        queued,
        run: () => act(() => queued.splice(0).forEach((run) => run(0))),
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
      expect(frame.queued).toHaveLength(1);
      frame.run();
      expect(result.current.lines.map((l) => l.text)).toEqual(["one", "two", "three"]);
    });

    it.each([
      [
        "the document is hidden, which draws no frames",
        () => {
          frames();
          Object.defineProperty(document, "hidden", { configurable: true, get: () => true });
        },
      ],
      ["there are no frames to wait for", () => vi.stubGlobal("requestAnimationFrame", undefined)],
    ])("still commits when %s", async (_, setUp) => {
      setUp();
      const s = fakeStream();
      const { result } = renderHook(() => useLogStream("kind-dev", "default", [target]));
      await s.connect();
      act(() => s.line("", "one"));
      await waitFor(() => expect(result.current.lines).toHaveLength(1));
    });
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
