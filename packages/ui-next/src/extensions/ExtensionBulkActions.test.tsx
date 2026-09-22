import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async (original) => ({
  ...(await original<typeof import("@srelens/core")>()),
  inspectExtensionResource: vi.fn(),
  actOnExtensionResource: vi.fn(),
  listExtensions: vi.fn(),
}));
import { actOnExtensionResource, inspectExtensionResource, listExtensions } from "@srelens/core";
import { ExtensionBulkActions } from "./ExtensionBulkActions";
import type { BulkResource } from "./bulkActions";

const target = {
  id: "org.srelens.flux",
  revision: 1,
  capability: "kustomizations",
  context: "cluster/a",
};
const rows = (count: number): BulkResource[] =>
  Array.from({ length: count }, (_, i) => ({ namespace: "team", name: `app-${i}` }));

const detailFor = (name: string) => ({
  resource: {
    apiVersion: "kustomize.toolkit.fluxcd.io/v1",
    kind: "Kustomization",
    metadata: { name, namespace: "team", uid: `uid-${name}`, resourceVersion: "12" },
    spec: {},
    status: {},
  },
  actions: ["suspend", "resume", "reconcile"],
  actionMeta: {
    reconcile: { impact: "medium" as const, confirm: "Reconcile {kind} {namespace}/{name} in cluster {cluster}?" },
    suspend: { impact: "high" as const, confirm: "Suspend {kind} {namespace}/{name} in cluster {cluster}?" },
    resume: { impact: "medium" as const, confirm: "Resume {kind} {namespace}/{name} in cluster {cluster}?" },
  },
});

/** Resolves when the test says so, so a run can be held mid-flight. */
function gate() {
  let open!: () => void;
  const promise = new Promise<void>((resolve) => {
    open = resolve;
  });
  return { promise, open };
}

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(listExtensions).mockResolvedValue({ plugins: [] } as never);
  vi.mocked(inspectExtensionResource).mockImplementation(async (selection) =>
    detailFor(selection.name),
  );
  vi.mocked(actOnExtensionResource).mockResolvedValue({ requested: true });
});

const openConfirmation = async (label = "Reconcile") => {
  fireEvent.click(await screen.findByRole("button", { name: label }));
  return screen.findByRole("dialog");
};

describe("bulk actions over an app's resource table", () => {
  it("asks once for the whole selection and names every resource in it", async () => {
    render(<ExtensionBulkActions target={target} selection={rows(12)} onClear={vi.fn()} />);
    expect(await screen.findByTestId("bulk-count")).toBeTruthy();
    expect(screen.getByTestId("bulk-count").textContent).toBe("12 selected");
    const dialog = await openConfirmation();
    // The one host confirmation (#552), naming the selection as a count.
    expect(screen.getByTestId("host-confirm-target").textContent).toBe("12 resources");
    expect(screen.getByTestId("host-confirm-impact").textContent).toBe("Medium impact");
    expect(dialog.textContent).toContain("cluster/a");
    const listed = screen.getByTestId("bulk-resources").textContent ?? "";
    expect(listed).toContain("team/app-0");
    expect(listed).toContain("team/app-11");
    // Nothing is written before the reader confirms.
    expect(actOnExtensionResource).not.toHaveBeenCalled();
  });

  it("says how much of the selection the action applies to", async () => {
    const available = (action: string, resource: BulkResource) =>
      action !== "reconcile" || !["app-0", "app-1", "app-2"].includes(resource.name);
    render(
      <ExtensionBulkActions target={target} selection={rows(12)} onClear={vi.fn()} available={available} />,
    );
    await openConfirmation();
    expect(screen.getByTestId("bulk-applies").textContent).toBe("applies to 9 of 12");
    expect(screen.getByTestId("host-confirm-target").textContent).toBe("9 resources");
    fireEvent.click(screen.getByRole("button", { name: "Reconcile 9 resources" }));
    await waitFor(() => expect(actOnExtensionResource).toHaveBeenCalledTimes(9));
    expect(
      vi.mocked(actOnExtensionResource).mock.calls.map(([selection]) => selection.name),
    ).not.toContain("app-0");
  });

  it("records one write per resource rather than one for the batch", async () => {
    render(<ExtensionBulkActions target={target} selection={rows(3)} onClear={vi.fn()} />);
    await openConfirmation();
    fireEvent.click(screen.getByRole("button", { name: "Reconcile 3 resources" }));
    await waitFor(() => expect(actOnExtensionResource).toHaveBeenCalledTimes(3));
    // Each call is the audited `extensions.action` for ONE resource, pinned to
    // the version this run read for it.
    expect(vi.mocked(actOnExtensionResource).mock.calls.map((call) => call.slice(1))).toEqual([
      ["reconcile", "uid-app-0", "12"],
      ["reconcile", "uid-app-1", "12"],
      ["reconcile", "uid-app-2", "12"],
    ]);
  });

  it("keeps no more than four writes in flight at once", async () => {
    let inFlight = 0;
    let peak = 0;
    const gates = new Map(rows(10).map((row) => [row.name, gate()]));
    vi.mocked(actOnExtensionResource).mockImplementation(async (selection) => {
      inFlight += 1;
      peak = Math.max(peak, inFlight);
      await gates.get(selection.name)!.promise;
      inFlight -= 1;
      return { requested: true };
    });
    render(<ExtensionBulkActions target={target} selection={rows(10)} onClear={vi.fn()} />);
    await openConfirmation();
    fireEvent.click(screen.getByRole("button", { name: "Reconcile 10 resources" }));
    await waitFor(() => expect(inFlight).toBe(4));
    for (const g of gates.values()) g.open();
    await screen.findByTestId("bulk-result");
    expect(peak).toBe(4);
  });

  it("cancels the pending resources and leaves the accepted ones alone", async () => {
    const gates = new Map(rows(10).map((row) => [row.name, gate()]));
    const sent: string[] = [];
    vi.mocked(actOnExtensionResource).mockImplementation(async (selection) => {
      sent.push(selection.name);
      await gates.get(selection.name)!.promise;
      return { requested: true };
    });
    render(<ExtensionBulkActions target={target} selection={rows(10)} onClear={vi.fn()} />);
    await openConfirmation();
    fireEvent.click(screen.getByRole("button", { name: "Reconcile 10 resources" }));
    await waitFor(() => expect(sent).toHaveLength(4));
    fireEvent.click(screen.getByRole("button", { name: "Cancel remaining" }));
    // The four already sent finish; nothing recalls them.
    for (const g of gates.values()) g.open();
    const result = await screen.findByTestId("bulk-result");
    expect(sent).toHaveLength(4);
    expect(result.getAttribute("data-status")).toBe("partial");
    expect(result.textContent).toContain("6 not requested");
    expect(screen.getByTestId("bulk-cancelled").textContent).toContain("team/app-9");
  });

  it("reports a mixed run as partial and never as a success", async () => {
    vi.mocked(actOnExtensionResource).mockImplementation(async (selection) => {
      if (selection.name === "app-1") throw new Error("admission webhook denied the request");
      if (selection.name === "app-2") return { requested: false };
      return { requested: true };
    });
    render(<ExtensionBulkActions target={target} selection={rows(3)} onClear={vi.fn()} />);
    await openConfirmation();
    fireEvent.click(screen.getByRole("button", { name: "Reconcile 3 resources" }));
    const result = await screen.findByTestId("bulk-result");
    expect(result.getAttribute("data-status")).toBe("partial");
    expect(result.getAttribute("role")).toBe("alert");
    expect(result.textContent).not.toMatch(/\bsucceeded\b|\bSuccess\b/);
    const failures = screen.getByTestId("bulk-failures").textContent ?? "";
    expect(failures).toContain("team/app-1");
    expect(failures).toContain("admission webhook denied the request");
    expect(failures).toContain("team/app-2");
    // An unacknowledged write is a failure, not a quiet success.
    expect(failures).toContain("not acknowledged");
  });

  it("reports a run the cluster refused entirely as failed", async () => {
    vi.mocked(actOnExtensionResource).mockRejectedValue(new Error("forbidden"));
    render(<ExtensionBulkActions target={target} selection={rows(2)} onClear={vi.fn()} />);
    await openConfirmation();
    fireEvent.click(screen.getByRole("button", { name: "Reconcile 2 resources" }));
    const result = await screen.findByTestId("bulk-result");
    expect(result.getAttribute("data-status")).toBe("failed");
    expect(result.textContent).toContain("None of the 2");
  });

  it("draws a cluster's reason and an over-long name as plain text that cannot reorder the list", async () => {
    const long = "a".repeat(200);
    const selection: BulkResource[] = [
      { namespace: "team", name: long },
      { namespace: "team", name: "app-1" },
    ];
    vi.mocked(actOnExtensionResource).mockImplementation(async (s) => {
      if (s.name === "app-1") throw new Error("denied ‮gnidnep‬ by policy");
      return { requested: true };
    });
    render(<ExtensionBulkActions target={target} selection={selection} onClear={vi.fn()} />);
    await openConfirmation();
    fireEvent.click(screen.getByRole("button", { name: "Reconcile 2 resources" }));
    const result = await screen.findByTestId("bulk-result");
    const failures = screen.getByTestId("bulk-failures");
    // The override is written out, not drawn, so it cannot reverse the line.
    expect(failures.textContent).toContain("\\u202e");
    expect(failures.textContent).not.toContain("‮");
    // A 200-character name is cut to the confirmation's ceiling rather than
    // pushing the state out of the row.
    const succeeded = screen.getByTestId("bulk-succeeded").textContent ?? "";
    expect(succeeded).toContain("…");
    expect(succeeded.length).toBeLessThan(120);
    expect(result.getAttribute("data-status")).toBe("partial");
  });

  it("does not offer an action the host does not list for this kind", async () => {
    vi.mocked(inspectExtensionResource).mockImplementation(async (selection) => ({
      ...detailFor(selection.name),
      actions: ["reconcile"],
    }));
    render(<ExtensionBulkActions target={target} selection={rows(2)} onClear={vi.fn()} />);
    expect(await screen.findByRole("button", { name: "Reconcile" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Suspend" })).toBeNull();
  });

  it("says the host's menu could not be read rather than showing no actions", async () => {
    vi.mocked(inspectExtensionResource).mockRejectedValue(new Error("Access denied"));
    render(<ExtensionBulkActions target={target} selection={rows(2)} onClear={vi.fn()} />);
    expect((await screen.findByRole("alert")).textContent).toContain("Access denied");
    expect(screen.queryByRole("button", { name: "Reconcile" })).toBeNull();
  });
});
