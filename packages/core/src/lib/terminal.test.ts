import { describe, it, expect, vi, beforeEach } from "vitest";

const { invokeCommand, subscribe } = vi.hoisted(() => ({
  invokeCommand: vi.fn(),
  subscribe: vi.fn(),
}));
vi.mock("../transport/transport", () => ({ invokeCommand, subscribe }));

import { startLocalTerminal } from "./terminal";

beforeEach(() => {
  invokeCommand.mockReset().mockResolvedValue(7);
  subscribe.mockReset().mockResolvedValue(() => {});
});

describe("startLocalTerminal", () => {
  /** What `start_terminal` was sent — the contract the Tauri command reads. */
  const started = () => invokeCommand.mock.calls.find(([command]) => command === "start_terminal")?.[1];

  it("starts a shell with no command of its own", async () => {
    await startLocalTerminal("prod", [], () => {}, () => {}, { cols: 80, rows: 24 });
    expect(started()).toEqual({
      context: "prod",
      extraKubeconfigs: [],
      channel: expect.stringMatching(/^term-/),
      cols: 80,
      rows: 24,
      command: null,
      namespace: null,
    });
  });

  it("hands the host the namespace the shell should start in", async () => {
    await startLocalTerminal("prod", [], () => {}, () => {}, undefined, undefined, "payments");
    expect(started()).toMatchObject({ namespace: "payments", command: null });
  });

  it("hands a command to the host to run first, under the name the host reads it by", async () => {
    await startLocalTerminal("prod", [], () => {}, () => {}, undefined, "kubectl drain node-1");
    expect(started()).toMatchObject({ command: "kubectl drain node-1", cols: null, rows: null });
  });

  it("subscribes to output and exit before the shell is started", async () => {
    const order: string[] = [];
    subscribe.mockImplementation(async (channel: string) => {
      order.push(channel.split(":").slice(0, 2).join(":"));
      return () => {};
    });
    invokeCommand.mockImplementation(async (command: string) => {
      order.push(command);
      return 7;
    });
    await startLocalTerminal("prod", [], () => {}, () => {});
    expect(order).toEqual(["term:out", "term:exit", "start_terminal"]);
  });

  it("drops its subscriptions when the shell cannot be started", async () => {
    const dispose = vi.fn();
    subscribe.mockResolvedValue(dispose);
    invokeCommand.mockRejectedValue(new Error("refused"));
    await expect(startLocalTerminal("prod", [], () => {}, () => {})).rejects.toThrow("refused");
    expect(dispose).toHaveBeenCalledTimes(2);
  });

  it("sends input and resizes to the session the host returned, and closes it", async () => {
    const dispose = vi.fn();
    subscribe.mockResolvedValue(dispose);
    const session = await startLocalTerminal("prod", [], () => {}, () => {});
    session.send("ls\r");
    session.resize(100, 30);
    session.close();
    expect(invokeCommand).toHaveBeenCalledWith("terminal_input", { session: 7, data: "ls\r" });
    expect(invokeCommand).toHaveBeenCalledWith("terminal_resize", { session: 7, cols: 100, rows: 30 });
    expect(invokeCommand).toHaveBeenCalledWith("terminal_close", { session: 7 });
    expect(dispose).toHaveBeenCalledTimes(2);
  });
});
