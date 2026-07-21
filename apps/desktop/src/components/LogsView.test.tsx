import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";

const { podLogsMock, getObjectMock, podsForSelectorMock, startLogStreamMock } = vi.hoisted(() => ({
  podLogsMock: vi.fn(),
  getObjectMock: vi.fn(),
  podsForSelectorMock: vi.fn(),
  startLogStreamMock: vi.fn(),
}));
vi.mock("../lib/workloads", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/workloads")>();
  return { ...actual, podLogs: podLogsMock, podsForSelector: podsForSelectorMock };
});
vi.mock("../lib/logsStream", () => ({ startLogStream: startLogStreamMock }));

const { saveTextFileMock } = vi.hoisted(() => ({ saveTextFileMock: vi.fn() }));
vi.mock("../lib/files", () => ({ saveTextFile: saveTextFileMock }));
vi.mock("../lib/manifest", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/manifest")>();
  return { ...actual, getObject: getObjectMock };
});

import { LogsView } from "./LogsView";

beforeEach(() => {
  podLogsMock.mockReset();
  getObjectMock.mockReset();
  podsForSelectorMock.mockReset();
  startLogStreamMock.mockReset();
  podLogsMock.mockResolvedValue({ logs: "" });
  getObjectMock.mockResolvedValue({ object: { spec: { containers: [{ name: "app" }] } } });
  podsForSelectorMock.mockResolvedValue({ pods: [] });
  startLogStreamMock.mockResolvedValue({ stop: vi.fn() });
  saveTextFileMock.mockReset();
  saveTextFileMock.mockResolvedValue("/tmp/web-1.log");
});

describe("LogsView", () => {
  it("fetches and renders logs for the pod's container", async () => {
    podLogsMock.mockResolvedValue({ logs: "line one\nline two" });
    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    await waitFor(() => expect(screen.getByText(/line two/)).toBeDefined());
    await waitFor(() =>
      expect(podLogsMock).toHaveBeenCalledWith(
        "kind-dev",
        "default",
        "web-1",
        undefined,
        expect.objectContaining({ container: "app" }),
      ),
    );
  });

  it("shows an error and can refresh", async () => {
    podLogsMock.mockResolvedValue({ error: "boom" });
    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    await waitFor(() => expect(screen.getByText(/boom/)).toBeDefined());
    podLogsMock.mockResolvedValue({ logs: "now ok" });
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));
    await waitFor(() => expect(screen.getByText(/now ok/)).toBeDefined());
  });

  it("resolves a workload's pods and offers an all-pods selector", async () => {
    getObjectMock.mockImplementation((_ctx: string, kind: string) =>
      kind === "Deployment"
        ? Promise.resolve({ object: { spec: { selector: { matchLabels: { app: "web" } } } } })
        : Promise.resolve({ object: { spec: { containers: [{ name: "app" }] } } }),
    );
    podsForSelectorMock.mockResolvedValue({ pods: [{ name: "web-1" }, { name: "web-2" }] });
    podLogsMock.mockResolvedValue({ logs: "hello" });

    render(
      <LogsView
        context="kind-dev"
        namespace="default"
        source={{ type: "workload", kind: "Deployment", name: "web" }}
      />,
    );

    await waitFor(() =>
      expect(podsForSelectorMock).toHaveBeenCalledWith("kind-dev", "default", { app: "web" }),
    );
    // A pod picker appears with an "all pods" option, and logs are fetched per pod.
    expect(await screen.findByRole("combobox", { name: "Pod" })).toBeDefined();
    await waitFor(() =>
      expect(podLogsMock).toHaveBeenCalledWith(
        "kind-dev",
        "default",
        "web-1",
        undefined,
        expect.objectContaining({ container: "app" }),
      ),
    );
    await waitFor(() =>
      expect(podLogsMock).toHaveBeenCalledWith(
        "kind-dev",
        "default",
        "web-2",
        undefined,
        expect.objectContaining({ container: "app" }),
      ),
    );
  });

  it("saves logs to a file on download", async () => {
    podLogsMock.mockResolvedValue({ logs: "line one\nline two" });
    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    await waitFor(() => expect(screen.getByText("line two")).toBeDefined());

    fireEvent.click(screen.getByRole("button", { name: "Download" }));
    await waitFor(() =>
      expect(saveTextFileMock).toHaveBeenCalledWith("web-1.log", "line one\nline two"),
    );
  });

  it("filters lines with the search box", async () => {
    podLogsMock.mockResolvedValue({ logs: "alpha line\nbeta line" });
    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    await waitFor(() => expect(screen.getByText("alpha line")).toBeDefined());

    fireEvent.change(screen.getByLabelText("Search logs"), { target: { value: "beta" } });
    expect(screen.queryByText("alpha line")).toBeNull();
    expect(screen.getByText("beta line")).toBeDefined();
  });

  it("starts a live-tail stream and appends streamed lines", async () => {
    podLogsMock.mockResolvedValue({ logs: "" });
    let emit: ((source: string, line: string) => void) | undefined;
    const stop = vi.fn();
    startLogStreamMock.mockImplementation(async (_c, _n, _t, onLine) => {
      emit = onLine;
      return { stop };
    });
    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    await waitFor(() => expect(screen.getByLabelText("Live tail")).toBeDefined());

    fireEvent.click(screen.getByLabelText("Live tail"));
    await waitFor(() => expect(startLogStreamMock).toHaveBeenCalled());

    act(() => emit?.("", "streamed line"));
    expect(await screen.findByText("streamed line")).toBeDefined();

    // Toggling off stops the stream.
    fireEvent.click(screen.getByLabelText("Pause live tail"));
    await waitFor(() => expect(stop).toHaveBeenCalled());
  });

  it("waits for container discovery before starting live mode", async () => {
    let resolvePod: ((value: unknown) => void) | undefined;
    getObjectMock.mockReturnValue(new Promise((resolve) => { resolvePod = resolve; }));

    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    fireEvent.click(screen.getByRole("button", { name: "Live tail" }));
    expect(startLogStreamMock).not.toHaveBeenCalled();

    await act(async () => {
      resolvePod?.({ object: { spec: { containers: [{ name: "app" }] } } });
    });
    await waitFor(() =>
      expect(startLogStreamMock).toHaveBeenCalledWith(
        "kind-dev",
        "default",
        [{ pod: "web-1", container: "app", label: "" }],
        expect.any(Function),
        expect.any(Function),
        expect.objectContaining({ tailLines: 200 }),
      ),
    );
  });

  it("scrolls to the newest streamed line while live", async () => {
    let emit: ((source: string, line: string) => void) | undefined;
    startLogStreamMock.mockImplementation(async (_c, _n, _t, onLine) => {
      emit = onLine;
      return { stop: vi.fn() };
    });

    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Container" })).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Live tail" }));
    await waitFor(() => expect(startLogStreamMock).toHaveBeenCalled());

    const viewport = screen.getByRole("log");
    Object.defineProperty(viewport, "scrollHeight", { configurable: true, value: 640 });
    act(() => emit?.("", "newest line"));
    expect(viewport.scrollTop).toBe(640);
  });

  it("reports a live-stream startup failure", async () => {
    startLogStreamMock.mockRejectedValue(new Error("stream unavailable"));
    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Container" })).toBeDefined());

    fireEvent.click(screen.getByRole("button", { name: "Live tail" }));
    expect(await screen.findByText(/stream unavailable/)).toBeDefined();
    // The initial fetch may still be in flight on slow runners; the spinner
    // must clear once everything settles, not synchronously with the error.
    await waitFor(() => expect(screen.queryByLabelText("Loading logs")).toBeNull());
  });

  it("offers a container picker for multi-container pods", async () => {
    getObjectMock.mockResolvedValue({
      object: { spec: { containers: [{ name: "app" }, { name: "sidecar" }] } },
    });
    podLogsMock.mockResolvedValue({ logs: "app logs" });
    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Container" })).toBeDefined());

    await userEvent.click(screen.getByRole("combobox", { name: "Container" }));
    await userEvent.click(await screen.findByRole("option", { name: "sidecar" }));
    await waitFor(() =>
      expect(podLogsMock).toHaveBeenCalledWith(
        "kind-dev",
        "default",
        "web-1",
        undefined,
        expect.objectContaining({ container: "sidecar" }),
      ),
    );
  });

  it("fetches previous-instance logs and disables live tail when enabled", async () => {
    podLogsMock.mockResolvedValue({ logs: "old crash line" });
    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    await waitFor(() => expect(screen.getByText("old crash line")).toBeDefined());

    fireEvent.click(screen.getByRole("button", { name: "Previous instance logs" }));
    await waitFor(() =>
      expect(podLogsMock).toHaveBeenCalledWith(
        "kind-dev",
        "default",
        "web-1",
        undefined,
        expect.objectContaining({ previous: true }),
      ),
    );
    // Previous logs are a terminated-container snapshot; the API can't follow.
    expect((screen.getByRole("button", { name: "Live tail" }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("threads the timestamps option through on toggle", async () => {
    podLogsMock.mockResolvedValue({ logs: "l1" });
    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    await waitFor(() => expect(screen.getByText("l1")).toBeDefined());

    fireEvent.click(screen.getByRole("button", { name: "Timestamps" }));
    await waitFor(() =>
      expect(podLogsMock).toHaveBeenCalledWith(
        "kind-dev",
        "default",
        "web-1",
        undefined,
        expect.objectContaining({ timestamps: true }),
      ),
    );
  });

  it("virtualises a long unwrapped buffer, rendering only a window of rows", async () => {
    // jsdom reports zero layout, so stub a measurable row/viewport size to make
    // computeLogWindow window the list instead of rendering everything.
    const rectSpy = vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockReturnValue({
      height: 16,
      width: 100,
      top: 0,
      left: 0,
      right: 100,
      bottom: 16,
      x: 0,
      y: 0,
      toJSON: () => ({}),
    } as DOMRect);
    const clientHeightSpy = vi
      .spyOn(HTMLElement.prototype, "clientHeight", "get")
      .mockReturnValue(320);

    const lines = Array.from({ length: 300 }, (_, i) => `log line ${i}`).join("\n");
    podLogsMock.mockResolvedValue({ logs: lines });
    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);

    // The top of the buffer renders, but far-off-screen lines are windowed out.
    await waitFor(() => expect(screen.getByText("log line 0")).toBeDefined());
    await waitFor(() => expect(screen.queryByText("log line 299")).toBeNull());
    // Only a small window of the 300 buffered lines is in the DOM.
    expect(screen.getAllByText(/^log line \d+$/).length).toBeLessThan(120);

    rectSpy.mockRestore();
    clientHeightSpy.mockRestore();
  });

  it("downloads a full all-containers dump with per-container headers", async () => {
    getObjectMock.mockResolvedValue({
      object: { spec: { containers: [{ name: "app" }, { name: "sidecar" }] } },
    });
    podLogsMock.mockImplementation((_c: string, _n: string, _p: string, _i: unknown, opts: { container?: string }) =>
      Promise.resolve({ logs: `${opts.container} logs` }),
    );
    render(<LogsView context="kind-dev" namespace="default" source={{ type: "pod", pod: "web-1" }} />);
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Container" })).toBeDefined());

    fireEvent.click(screen.getByRole("button", { name: "Download all containers" }));
    await waitFor(() =>
      expect(saveTextFileMock).toHaveBeenCalledWith(
        "web-1-all.log",
        expect.stringContaining("==> web-1/app <=="),
      ),
    );
    expect(saveTextFileMock.mock.calls[0][1]).toContain("==> web-1/sidecar <==");
    expect(saveTextFileMock.mock.calls[0][1]).toContain("sidecar logs");
  });
});
