import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const { invokeCommandMock, subscribeMock } = vi.hoisted(() => ({
  invokeCommandMock: vi.fn(),
  subscribeMock: vi.fn(),
}));
vi.mock("../transport/transport", () => ({
  invokeCommand: invokeCommandMock,
  subscribe: subscribeMock,
}));

import { startPodExec, SUBSCRIBE_TIMEOUT_MS } from "./exec";
import { describeError } from "./errors";

/** The subscription token `startPodExec` handed the backend, read back off
 *  the invoke it made — the test never guesses how it is spelled. */
function channelOf(nth = 0): string {
  const calls = invokeCommandMock.mock.calls.filter(([command]) => command === "start_pod_exec");
  return (calls[nth]?.[1] as { channel: string }).channel;
}

/** Records every handler by channel, the way both transports' `subscribe`
 *  does: registration is complete once the promise resolves. */
function captureSubscriptions() {
  const handlers: Record<string, (p: unknown) => void> = {};
  const disposals: string[] = [];
  subscribeMock.mockImplementation(async (channel: string, handler: (p: unknown) => void) => {
    handlers[channel] = handler;
    return () => disposals.push(channel);
  });
  return { handlers, disposals };
}

beforeEach(() => {
  invokeCommandMock.mockReset();
  subscribeMock.mockReset();
});

describe("startPodExec", () => {
  it("starts a session, wires output/exit, and sends input + close", async () => {
    invokeCommandMock.mockResolvedValueOnce(7); // session id
    const { handlers, disposals } = captureSubscriptions();
    const onData = vi.fn();
    const onExit = vi.fn();

    const session = await startPodExec("kind-dev", "default", "web-1", onData, onExit);
    const channel = channelOf();

    expect(invokeCommandMock).toHaveBeenCalledWith("start_pod_exec", {
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
      container: null,
      shell: null,
      command: null,
      channel,
      cols: null,
      rows: null,
    });

    handlers[`exec:out:${channel}`]("hello\n");
    expect(onData).toHaveBeenCalledWith("hello\n");
    handlers[`exec:exit:${channel}`](null);
    expect(onExit).toHaveBeenCalledWith(null);

    session.send("ls\n");
    expect(invokeCommandMock).toHaveBeenCalledWith("exec_input", { session: 7, data: "ls\n" });
    session.resize(120, 40);
    expect(invokeCommandMock).toHaveBeenCalledWith("exec_resize", { session: 7, cols: 120, rows: 40 });
    session.close();
    expect(invokeCommandMock).toHaveBeenCalledWith("exec_close", { session: 7 });
    expect(disposals).toEqual([`exec:out:${channel}`, `exec:exit:${channel}`]);
  });

  it("forwards a container when execing into a specific one", async () => {
    invokeCommandMock.mockResolvedValueOnce(9);
    captureSubscriptions();
    await startPodExec("kind-dev", "default", "web-1", vi.fn(), vi.fn(), "sidecar");
    expect(invokeCommandMock).toHaveBeenCalledWith("start_pod_exec", {
      context: "kind-dev",
      namespace: "default",
      pod: "web-1",
      container: "sidecar",
      shell: null,
      command: null,
      channel: channelOf(),
      cols: null,
      rows: null,
    });
  });

  /**
   * The race this closes: `start_pod_exec` spawns its task and can emit
   * `exec:exit:<channel>` in the same tick — an invalid context, an RBAC
   * refusal — so a listener registered after the invoke resolves misses the
   * only exit event there will ever be. The row then never closes and a node
   * session's privileged debug pod is left running on the node.
   */
  it("delivers an exit the backend emits in the same tick it spawns the session", async () => {
    const { handlers } = captureSubscriptions();
    let exitWasListenedFor = false;
    invokeCommandMock.mockImplementation(async (command: string, args?: Record<string, unknown>) => {
      if (command !== "start_pod_exec") return null;
      const exit = handlers[`exec:exit:${String(args?.channel)}`];
      exitWasListenedFor = exit !== undefined;
      exit?.('pods "web-1" is forbidden');
      return 4;
    });
    const onExit = vi.fn();

    await startPodExec("kind-dev", "default", "web-1", vi.fn(), onExit);

    expect(exitWasListenedFor).toBe(true);
    expect(onExit).toHaveBeenCalledWith('pods "web-1" is forbidden');
  });

  it("gives each session its own channel, so one shell never reads another's output", async () => {
    invokeCommandMock.mockResolvedValue(1);
    captureSubscriptions();
    await startPodExec("kind-dev", "default", "web-1", vi.fn(), vi.fn());
    await startPodExec("kind-dev", "default", "web-2", vi.fn(), vi.fn());
    expect(channelOf(0)).not.toBe(channelOf(1));
  });

  it("drops both listeners when the session could not be started at all", async () => {
    const { disposals } = captureSubscriptions();
    invokeCommandMock.mockRejectedValueOnce("no such context");
    await expect(startPodExec("gone", "default", "web-1", vi.fn(), vi.fn())).rejects.toBe(
      "no such context",
    );
    expect(disposals).toEqual([`exec:out:${channelOf()}`, `exec:exit:${channelOf()}`]);
  });

  /**
   * Subscribing BEFORE the backend starts is right and stays — the exec task
   * can emit its exit event in the same tick it is spawned, and that is the
   * only exit event there will ever be. But on the web `subscribe` resolves
   * only when the `subbed` ack comes back over the socket, with no timeout of
   * its own: if the socket is down, `wsClient` retries forever and the promise
   * stays pending forever. Before the reorder, exec used fire-and-forget `on()`
   * and the first failure came from `invokeCommand` over HTTP, which rejects
   * promptly.
   *
   * So: web session open, server dies, reader clicks "Open shell" — no error,
   * no rejection, a spinner that never resolves. The project's rule is that a
   * timeout surfaces as an error.
   */
  describe("when the subscription is never acknowledged", () => {
    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it("rejects rather than hanging forever", async () => {
      subscribeMock.mockImplementation(() => new Promise(() => {}));
      const started = startPodExec("kind-dev", "default", "web-1", vi.fn(), vi.fn());
      const settled = vi.fn();
      void started.then(settled, settled);

      // Long past any plausible ack, and still nothing had happened.
      await vi.advanceTimersByTimeAsync(SUBSCRIBE_TIMEOUT_MS - 1);
      expect(settled).not.toHaveBeenCalled();

      await vi.advanceTimersByTimeAsync(2);
      await expect(started).rejects.toThrow(/timed out/);
    });

    it("words the failure through describeError rather than at the reader raw", async () => {
      subscribeMock.mockImplementation(() => new Promise(() => {}));
      const started = startPodExec("kind-dev", "default", "web-1", vi.fn(), vi.fn());
      const caught = started.catch((e: unknown) => e);
      await vi.advanceTimersByTimeAsync(SUBSCRIBE_TIMEOUT_MS + 1);
      const friendly = describeError(await caught);
      expect(friendly.title).toBe("Request timed out");
      // And the original still names what actually timed out, one click away.
      expect(friendly.raw).toContain("shell");
    });

    it("never starts the backend session it could not listen for", async () => {
      subscribeMock.mockImplementation(() => new Promise(() => {}));
      const started = startPodExec("kind-dev", "default", "web-1", vi.fn(), vi.fn());
      const caught = started.catch(() => undefined);
      await vi.advanceTimersByTimeAsync(SUBSCRIBE_TIMEOUT_MS + 1);
      await caught;
      // An exec session started with no listener on its exit channel is a row
      // attached forever and, for a node session, a privileged debug pod left
      // running on the node.
      expect(invokeCommandMock).not.toHaveBeenCalled();
    });

    it("drops a subscription that lands after the wait was given up", async () => {
      // The web transport registers the handler synchronously and awaits only
      // the ACK, so a late `subbed` leaves a live handler on a channel nobody
      // reads — and `wsClient` resubscribes it on every reconnect.
      const disposals: string[] = [];
      let release: (() => void) | undefined;
      subscribeMock.mockImplementation(
        (channel: string) =>
          new Promise<() => void>((resolve) => {
            release = () => resolve(() => disposals.push(channel));
          }),
      );
      const started = startPodExec("kind-dev", "default", "web-1", vi.fn(), vi.fn());
      const caught = started.catch(() => undefined);
      await vi.advanceTimersByTimeAsync(SUBSCRIBE_TIMEOUT_MS + 1);
      await caught;

      release?.();
      await vi.advanceTimersByTimeAsync(0);
      expect(disposals).toHaveLength(1);
    });

    it("drops the output listener when it is the EXIT one that never lands", async () => {
      const disposals: string[] = [];
      subscribeMock.mockImplementation(async (channel: string) => {
        if (channel.startsWith("exec:exit:")) return new Promise<() => void>(() => {});
        return () => disposals.push(channel);
      });
      const started = startPodExec("kind-dev", "default", "web-1", vi.fn(), vi.fn());
      const caught = started.catch(() => undefined);
      await vi.advanceTimersByTimeAsync(SUBSCRIBE_TIMEOUT_MS + 1);
      await caught;
      expect(disposals).toHaveLength(1);
      expect(disposals[0]).toMatch(/^exec:out:/);
    });
  });
});
