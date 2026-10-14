import fluxManifest from "../../../../examples/extensions/flux.json";
import argoManifest from "../../../../examples/extensions/argocd.json";
const declaredMeta = Object.fromEntries([...fluxManifest.actions.filter(action=>action.resource==="helmreleases").map(action=>({...action,name:action.name.replace("helmreleases-","")})),...argoManifest.actions].map(action=>[action.name,{title:action.title,availableWhen:("availableWhen" in action?action.availableWhen:[]) as import("@srelens/core").ActionPredicate[],impact:"medium" as const,confirm:null}]));
import { act, render, screen, fireEvent, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
    reconcile: { ...declaredMeta.reconcile, impact: "medium" as const, confirm: "Reconcile {kind} {namespace}/{name} in cluster {cluster}?" },
    suspend: { ...declaredMeta.suspend, impact: "high" as const, confirm: "Suspend {kind} {namespace}/{name} in cluster {cluster}?" },
    resume: { ...declaredMeta.resume, impact: "medium" as const, confirm: "Resume {kind} {namespace}/{name} in cluster {cluster}?" },
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
    expect(result.textContent).toContain("No acceptance was confirmed for the 2");
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

it.each(["Escape", "Cancel"])("returns keyboard focus after %s closes review", async (close) => {
  const user = userEvent.setup();
  render(<ExtensionBulkActions target={target} selection={rows(2)} onClear={vi.fn()} />);
  const trigger = await screen.findByRole("button", { name: "Reconcile" });
  trigger.focus();
  await user.keyboard("{Enter}");
  const dialog = screen.getByRole("dialog");
  expect(dialog.contains(document.activeElement)).toBe(true);
  if (close === "Escape") await user.keyboard("{Escape}");
  else await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(document.activeElement).toBe(trigger);
});

it("keeps keyboard focus on the running operation after confirmation", async () => {
  const held = gate();
  vi.mocked(actOnExtensionResource).mockImplementation(async () => {
    await held.promise;
    return { requested: true };
  });
  render(<ExtensionBulkActions target={target} selection={rows(2)} onClear={vi.fn()} />);
  await openConfirmation();
  fireEvent.click(screen.getByRole("button", { name: "Reconcile 2 resources" }));
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "Cancel remaining" }));
  await act(async () => held.open());
});

it.each(["inspection", "transport", "empty"])("reports %s errors as failures without claiming a rejection", async (failure) => {
  const held = gate();
  render(<ExtensionBulkActions target={target} selection={rows(2)} onClear={vi.fn()} />);
  await openConfirmation();
  if (failure === "inspection") {
    vi.mocked(inspectExtensionResource).mockImplementation(async (s) => {
      if (s.name === "app-0") throw new Error("Could not read the resource");
      return detailFor(s.name);
    });
  }
  vi.mocked(actOnExtensionResource).mockImplementation(async (s) => {
    if (s.name === "app-0") throw new Error(failure === "empty" ? "" : "Connection lost");
    await held.promise;
    return { requested: true };
  });
  fireEvent.click(screen.getByRole("button", { name: "Reconcile 2 resources" }));
  await waitFor(() => expect(screen.getByTestId("bulk-item-team/app-0").textContent).toContain("Failed"));
  expect(screen.getByRole("status").textContent).toBe("1 of 2 completed");
  await act(async () => held.open());
  const result = await screen.findByTestId("bulk-result");
  expect(result.textContent).toContain("1 failed");
  expect(result.textContent).not.toMatch(/rejected/i);
  expect(screen.getByTestId("bulk-failures").textContent).toContain(
    failure === "inspection" ? "Could not read the resource" : failure === "empty" ? "The operation failed without a reason." : "Connection lost",
  );
  if (failure === "inspection") expect(vi.mocked(actOnExtensionResource).mock.calls.map(([s]) => s.name)).toEqual(["app-1"]);
});

it("bounds long failure reasons in both progress and results", async () => {
  const held = gate();
  vi.mocked(actOnExtensionResource).mockImplementation(async (s) => {
    if (s.name === "app-0") throw new Error("x".repeat(1000));
    await held.promise;
    return { requested: true };
  });
  render(<ExtensionBulkActions target={target} selection={rows(2)} onClear={vi.fn()} />);
  await openConfirmation();
  fireEvent.click(screen.getByRole("button", { name: "Reconcile 2 resources" }));
  const expected = "x".repeat(79) + "…";
  await waitFor(() => expect(screen.getByTestId("bulk-item-team/app-0").querySelector(".extension-bulk-reason")?.textContent).toBe(expected));
  await act(async () => held.open());
  expect((await screen.findByTestId("bulk-failures")).querySelector(".extension-bulk-reason")?.textContent).toBe(expected);
});

it("stops obsolete availability queues when the selection changes", async () => {
  const held = gate();
  const inspected: string[] = [];
  vi.mocked(inspectExtensionResource).mockImplementation(async (s) => {
    inspected.push(s.name);
    if (s.name.startsWith("app-")) await held.promise;
    return detailFor(s.name);
  });
  const view = render(<ExtensionBulkActions target={target} selection={rows(12)} onClear={vi.fn()} />);
  await waitFor(() => expect(inspected).toHaveLength(4));
  view.rerender(<ExtensionBulkActions target={target} selection={[{namespace:"team",name:"new"}]} onClear={vi.fn()} />);
  await screen.findByRole("button", { name: "Reconcile" });
  await act(async () => held.open());
  expect(inspected).toEqual(["app-0", "app-1", "app-2", "app-3", "new"]);
});

it("reveals distinct full escaped identities on focus and hover throughout a bulk run", async () => {
  const prefix = "a".repeat(100);
  const names = [`${prefix}-east`, `${prefix}-west\u202e`];
  const selection = names.map(name => ({ namespace: "team", name }));
  const held = gate();
  vi.mocked(actOnExtensionResource).mockImplementation(async (s) => {
    await held.promise;
    if (s.name === names[1]) throw new Error("Connection lost");
    return { requested: true };
  });
  render(<ExtensionBulkActions target={target} selection={selection} onClear={vi.fn()} />);
  await openConfirmation();
  const checkNames = (container: HTMLElement) => {
    const fields = [...container.querySelectorAll<HTMLElement>(".extension-bulk-name")];
    for (const [i, field] of fields.entries()) {
      const full = `team/${prefix}-${i === 0 ? "east" : "west\\u202e"}`;
      expect(field.textContent).toHaveLength(80);
      expect(field.tabIndex).toBe(0);
      expect(field.getAttribute("title")).toBe(full);
      fireEvent.focus(field);
      expect(field.textContent).toBe(full);
      fireEvent.blur(field);
      expect(field.textContent).toHaveLength(80);
      fireEvent.mouseEnter(field);
      expect(field.textContent).toBe(full);
      fireEvent.mouseLeave(field);
      expect(field.textContent).toHaveLength(80);
      expect(field.textContent).not.toContain("\u202e");
    }
  };
  checkNames(screen.getByTestId("bulk-resources"));
  fireEvent.click(screen.getByRole("button", { name: "Reconcile 2 resources" }));
  await waitFor(() => expect(actOnExtensionResource).toHaveBeenCalledTimes(2));
  checkNames(screen.getByTestId("bulk-progress"));
  await act(async () => held.open());
  // Final accepted and failed names use the same focusable identity field.
  const result = await screen.findByTestId("bulk-result");
  const fields = result.querySelectorAll<HTMLElement>(".extension-bulk-name");
  expect(fields).toHaveLength(2);
  for (const field of fields) {
    fireEvent.focus(field);
    expect(field.textContent).toBe(field.getAttribute("title"));
    expect(field.textContent!.length).toBeGreaterThan(100);
  }
});
