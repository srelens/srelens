import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";

const { listResourceMock, listCrdsMock } = vi.hoisted(() => ({
  listResourceMock: vi.fn(),
  listCrdsMock: vi.fn(),
}));
vi.mock("../lib/manifest", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/manifest")>();
  return { ...actual, listResource: listResourceMock };
});
vi.mock("../lib/crds", () => ({ listCrds: listCrdsMock }));

const { deleteResourceMock, rolloutRestartMock } = vi.hoisted(() => ({
  deleteResourceMock: vi.fn(),
  rolloutRestartMock: vi.fn(),
}));
vi.mock("../lib/actions", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../lib/actions")>();
  return { ...actual, deleteResource: deleteResourceMock, rolloutRestart: rolloutRestartMock };
});

const { notifyMock } = vi.hoisted(() => ({
  notifyMock: { success: vi.fn(), error: vi.fn(), info: vi.fn() },
}));
vi.mock("../lib/notify", () => ({ notify: notifyMock }));

import { CommandPalette } from "./CommandPalette";

const widgetCrd = {
  name: "widgets.example.com",
  group: "example.com",
  version: "v1",
  kind: "Widget",
  plural: "widgets",
  namespaced: true,
};

beforeEach(() => {
  localStorage.clear();
  listResourceMock.mockReset();
  listCrdsMock.mockReset();
  deleteResourceMock.mockReset();
  rolloutRestartMock.mockReset();
  notifyMock.success.mockReset();
  notifyMock.error.mockReset();
  listResourceMock.mockImplementation((_c: string, kind: string) =>
    Promise.resolve({
      items:
        kind === "Pod"
          ? [{ name: "web-1", namespace: "default", age: "1m" }]
          : kind === "Deployment"
            ? [{ name: "web-deploy", namespace: "default", age: "1m" }]
            : [],
    }),
  );
  listCrdsMock.mockResolvedValue({ crds: [widgetCrd] });
});

function setup(extra?: Partial<React.ComponentProps<typeof CommandPalette>>) {
  const onOpenView = vi.fn();
  const onOpenResource = vi.fn();
  const onOpenCrd = vi.fn();
  const onAfterAction = vi.fn();
  const r = render(
    <CommandPalette
      open
      onOpenChange={vi.fn()}
      context="kind-dev"
      onOpenView={onOpenView}
      onOpenResource={onOpenResource}
      onOpenCrd={onOpenCrd}
      onAfterAction={onAfterAction}
      {...extra}
    />,
  );
  return { ...r, onOpenView, onOpenResource, onOpenCrd, onAfterAction };
}

describe("CommandPalette", () => {
  it("opens a view and records it in Recent", async () => {
    const { onOpenView, unmount } = setup();
    await userEvent.click(await screen.findByText("Pods"));
    expect(onOpenView).toHaveBeenCalledWith("pods");
    unmount();

    // Reopening shows the just-opened view under "Recent".
    setup();
    expect(await screen.findByText("Recent")).toBeDefined();
  });

  it("can navigate to the workload controller views", async () => {
    const { onOpenView } = setup();
    const search = screen.getByPlaceholderText(/Search resources/);
    for (const [label, kind] of [
      ["StatefulSets", "statefulsets"],
      ["DaemonSets", "daemonsets"],
      ["CronJobs", "cronjobs"],
    ] as const) {
      await userEvent.clear(search);
      await userEvent.type(search, label);
      await userEvent.click(await screen.findByText(label));
      expect(onOpenView).toHaveBeenCalledWith(kind);
    }
  });

  it("surfaces CRDs in Go to and opens them", async () => {
    const { onOpenCrd } = setup();
    await userEvent.type(screen.getByPlaceholderText(/Search resources/), "widget");
    await userEvent.click(await screen.findByText("Widget (CRD)"));
    expect(onOpenCrd).toHaveBeenCalledWith(widgetCrd);
  });

  it("drills into a pod's actions instead of opening it directly, then opens details on demand", async () => {
    const { onOpenResource } = setup();
    await waitFor(() => expect(listResourceMock).toHaveBeenCalled());
    await userEvent.type(screen.getByPlaceholderText(/Search resources/), "web-1");
    await userEvent.click(await screen.findByText("web-1"));

    // Selecting the pod drills into action mode instead of opening it.
    expect(onOpenResource).not.toHaveBeenCalled();
    expect(await screen.findByText("Actions for web-1")).toBeDefined();
    expect(screen.getByText("Delete")).toBeDefined();
    expect(screen.getByText("Evict pod")).toBeDefined();

    await userEvent.click(screen.getByText("Open details"));
    expect(onOpenResource).toHaveBeenCalledWith("pods", "default", "web-1");
  });

  it("drills into a resource's actions and runs a non-destructive one directly", async () => {
    rolloutRestartMock.mockResolvedValue({ ok: true });
    const { onAfterAction } = setup();
    await waitFor(() => expect(listResourceMock).toHaveBeenCalled());
    await userEvent.type(screen.getByPlaceholderText(/Search resources/), "web-deploy");
    await userEvent.click(await screen.findByText("web-deploy"));
    expect(await screen.findByText("Actions for web-deploy")).toBeDefined();

    await userEvent.click(screen.getByText("Rollout restart"));

    await waitFor(() =>
      expect(rolloutRestartMock).toHaveBeenCalledWith("kind-dev", "Deployment", "default", "web-deploy"),
    );
    await waitFor(() => expect(onAfterAction).toHaveBeenCalled());
  });

  it("calls onOpenResource for opensDialog actions (scale), never invoking a run handler", async () => {
    const { onOpenResource } = setup();
    await waitFor(() => expect(listResourceMock).toHaveBeenCalled());
    await userEvent.type(screen.getByPlaceholderText(/Search resources/), "web-deploy");
    await userEvent.click(await screen.findByText("web-deploy"));
    expect(await screen.findByText("Actions for web-deploy")).toBeDefined();

    await userEvent.click(screen.getByText("Scale…"));

    expect(onOpenResource).toHaveBeenCalledWith("deployments", "default", "web-deploy");
  });

  it("requires confirmation before running a destructive action, then dispatches it", async () => {
    deleteResourceMock.mockResolvedValue({ ok: true });
    const { onAfterAction } = setup();
    await waitFor(() => expect(listResourceMock).toHaveBeenCalled());
    await userEvent.type(screen.getByPlaceholderText(/Search resources/), "web-deploy");
    await userEvent.click(await screen.findByText("web-deploy"));
    expect(await screen.findByText("Actions for web-deploy")).toBeDefined();

    await userEvent.click(screen.getByText("Delete"));

    // The mutation must not fire before the confirm dialog is accepted.
    expect(deleteResourceMock).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeDefined();

    // The dialog hides the outside content, so its own "Delete" button is the
    // only one reachable now.
    await userEvent.click(await screen.findByRole("button", { name: "Delete" }));

    await waitFor(() =>
      expect(deleteResourceMock).toHaveBeenCalledWith("kind-dev", "Deployment", "default", "web-deploy"),
    );
    await waitFor(() => expect(onAfterAction).toHaveBeenCalled());
  });

  it("surfaces a failed action's error via notify", async () => {
    deleteResourceMock.mockResolvedValue({ error: "forbidden" });
    setup();
    await waitFor(() => expect(listResourceMock).toHaveBeenCalled());
    await userEvent.type(screen.getByPlaceholderText(/Search resources/), "web-deploy");
    await userEvent.click(await screen.findByText("web-deploy"));
    await userEvent.click(await screen.findByText("Delete"));
    await userEvent.click(await screen.findByRole("button", { name: "Delete" }));

    await waitFor(() => expect(notifyMock.error).toHaveBeenCalledWith("Failed: Delete", "forbidden"));
  });

  it("goes back to browsing via the Back item without dispatching anything", async () => {
    const { onOpenResource } = setup();
    await waitFor(() => expect(listResourceMock).toHaveBeenCalled());
    await userEvent.type(screen.getByPlaceholderText(/Search resources/), "web-deploy");
    await userEvent.click(await screen.findByText("web-deploy"));
    expect(await screen.findByText("Actions for web-deploy")).toBeDefined();

    await userEvent.click(screen.getByText("← Back"));

    expect(screen.queryByText("Actions for web-deploy")).toBeNull();
    expect(onOpenResource).not.toHaveBeenCalled();
    expect(deleteResourceMock).not.toHaveBeenCalled();
  });

  it("ranks recents for the current view kind under a Quick group before Recent and Go to", async () => {
    // Seed a recent pod so it shows up once the palette is reopened.
    const { unmount, onOpenResource } = setup({ currentViewKind: "pods" });
    await waitFor(() => expect(listResourceMock).toHaveBeenCalled());
    await userEvent.type(screen.getByPlaceholderText(/Search resources/), "web-1");
    await userEvent.click(await screen.findByText("web-1"));
    await userEvent.click(await screen.findByText("Open details"));
    expect(onOpenResource).toHaveBeenCalledWith("pods", "default", "web-1");
    unmount();

    setup({ currentViewKind: "pods" });
    expect(await screen.findByText("Quick: Pods")).toBeDefined();
  });

  it("narrows to the given kind when the query starts with a known kind token", async () => {
    listResourceMock.mockImplementation((_c: string, kind: string) =>
      Promise.resolve({
        items:
          kind === "Pod"
            ? [{ name: "nginx", namespace: "default", age: "1m" }]
            : kind === "Service"
              ? [{ name: "nginx", namespace: "default", age: "1m" }]
              : [],
      }),
    );
    setup();
    await waitFor(() => expect(listResourceMock).toHaveBeenCalled());

    await userEvent.type(screen.getByPlaceholderText(/Search resources/), "pod ng");

    // Both a pod and a service named "nginx" are indexed, but "pod ng" must
    // only surface the pod.
    expect(await screen.findByText("nginx")).toBeDefined();
    expect(screen.getByText("Pod")).toBeDefined();
    expect(screen.queryByText("Service")).toBeNull();
  });
});
