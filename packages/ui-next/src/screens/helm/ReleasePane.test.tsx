import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Invoker } from "@srelens/core";
import { ReleasePane, currentOp, type PaneRelease } from "./ReleasePane";
import type { HelmOpRow } from "../../lib/helmOps";

const CONTEXT = "prod-eu";

const CHECKOUT: PaneRelease = {
  name: "checkout",
  namespace: "checkout",
  revision: 119,
  status: "failed",
};

const PAYMENTS: PaneRelease = {
  name: "payments",
  namespace: "payments",
  revision: 62,
  status: "deployed",
};

/**
 * Two rendered manifests that differ in exactly the two places §16's diff
 * differs in. The strings on either side are unique to their side, so a test
 * asserting the diff is NOT on screen cannot be satisfied by the other pane.
 */
const REV_118 = ['replicaCount: 12', 'image:', '  tag: "118a7e"', 'env:', '  DB_POOL_MAX: "40"'].join("\n");
const REV_119 = ['replicaCount: 12', 'image:', '  tag: "4f2a1c"', 'env:', '  DB_POOL_MAX: "5"'].join("\n");

const MANIFESTS: Record<string, Record<number, string>> = {
  checkout: { 118: REV_118, 119: REV_119 },
  payments: { 61: "replicas: 1", 62: "replicas: 4" },
};

/** A `k8s.getHelmRelease` that answers out of {@link MANIFESTS}. */
function invoker() {
  return vi.fn(async (id: string, input?: unknown) => {
    if (id !== "k8s.getHelmRelease") throw new Error(`unexpected capability ${id}`);
    const req = input as { name: string; revision?: number };
    const manifest = MANIFESTS[req.name]?.[req.revision ?? -1];
    if (manifest === undefined) throw new Error(`no revision ${req.revision} of ${req.name}`);
    return {
      name: req.name,
      namespace: req.name,
      revision: req.revision,
      status: "deployed",
      chart: "acme-service",
      chartVersion: "2.4.1",
      appVersion: "4f2a1c",
      updated: "2026-08-24",
      valuesYaml: "",
      manifest,
      notes: "",
      history: [],
    };
  }) as unknown as Invoker;
}

function op(over: Partial<HelmOpRow> = {}): HelmOpRow {
  return {
    id: 1,
    kind: "upgrade",
    release: "checkout",
    namespace: "checkout",
    context: CONTEXT,
    state: "running",
    output: [],
    startedAt: 1_000,
    ...over,
  };
}

function mount(props: Partial<Parameters<typeof ReleasePane>[0]> = {}) {
  const invoke = props.invoke ?? invoker();
  const view = render(
    <ReleasePane context={CONTEXT} release={CHECKOUT} {...props} invoke={invoke} />,
  );
  return { ...view, invoke };
}

/** The pane's head, as the reader reads it. */
function head(): string {
  return document.querySelector(".pane-head")?.textContent ?? "";
}

describe("currentOp", () => {
  it("ignores an operation belonging to another release", () => {
    const other = op({ release: "payments", namespace: "payments" });
    expect(currentOp([other], CONTEXT, CHECKOUT)).toBeNull();
  });

  it("ignores an operation in another cluster", () => {
    expect(currentOp([op({ context: "edge-apac" })], CONTEXT, CHECKOUT)).toBeNull();
  });

  it("ignores a same-named release in another namespace", () => {
    // Helm scopes a release name to a namespace, so `checkout/checkout` and
    // `staging/checkout` are two releases. Matching on the name alone would
    // put staging's upgrade output over production's diff.
    expect(currentOp([op({ namespace: "staging" })], CONTEXT, CHECKOUT)).toBeNull();
  });

  it("ignores an operation that finished cleanly", () => {
    expect(currentOp([op({ state: "done" })], CONTEXT, CHECKOUT)).toBeNull();
  });

  it("prefers the running operation over the failed one, whichever started first", () => {
    const failed = op({ id: 2, state: "failed", error: "boom", startedAt: 9_000 });
    const running = op({ id: 1, state: "running", startedAt: 1_000 });
    expect(currentOp([failed, running], CONTEXT, CHECKOUT)?.id).toBe(1);
    expect(currentOp([running, failed], CONTEXT, CHECKOUT)?.id).toBe(1);
  });

  it("takes the newest of several failures", () => {
    const older = op({ id: 1, state: "failed", error: "older", startedAt: 1_000 });
    const newer = op({ id: 2, state: "failed", error: "newer", startedAt: 2_000 });
    expect(currentOp([newer, older], CONTEXT, CHECKOUT)?.id).toBe(2);
  });

  it("has nothing to consider when no release is selected", () => {
    expect(currentOp([op()], CONTEXT, null)).toBeNull();
  });
});

describe("ReleasePane — an operation outranks the diff", () => {
  it("shows a running operation's output instead of the diff", async () => {
    const { invoke } = mount({
      ops: [op({ output: ["Release \"checkout\" has been upgraded.", "REVISION: 120"] })],
    });

    expect(screen.getByText('Release "checkout" has been upgraded.')).toBeTruthy();
    expect(screen.getByText("REVISION: 120")).toBeTruthy();
    // The diff for this release is fetchable and still loses.
    expect(screen.queryByText('tag: "4f2a1c"')).toBeNull();
    expect(screen.queryByText('tag: "118a7e"')).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
    expect(head()).toContain("upgrade");
    expect(head()).not.toContain("rendered diff");
  });

  it("says an operation is live rather than leaving it to the output", () => {
    mount({ ops: [op({ output: ["waiting for rollout"] })] });
    // A live region, so a stream that starts or stops is announced rather than
    // silently redrawn — and it names the operation, not just its colour.
    expect(screen.getByRole("status").textContent).toMatch(/upgrade running/i);
  });

  it("says nothing is live once the operation has failed", () => {
    mount({ ops: [op({ state: "failed", error: "no such release", output: [] })] });
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("draws no dismiss control on a running operation, because there is no cancel", () => {
    const onDismiss = vi.fn();
    mount({ ops: [op({ output: ["waiting"] })], onDismiss });
    expect(screen.queryByRole("button", { name: /dismiss/i })).toBeNull();
    expect(onDismiss).not.toHaveBeenCalled();
  });

  /**
   * `describeError`'s own copy for a 401 — exactly what the store writes into
   * `error`, character for character.
   *
   * Chosen because it is the case where describing a described sentence does
   * visible damage: this text contains "client certificate", the TLS branch
   * matches on it, and a second pass turns a rejected token into a complaint
   * about the cluster's certificate. Anything shorter would be re-described
   * into itself and the mistake would not show.
   */
  const DESCRIBED =
    "The cluster rejected your credentials. Your token or client certificate may have expired — refresh your kubeconfig credentials and try again.";

  it("keeps a failed operation's reason on screen, with its output", () => {
    mount({
      ops: [
        op({
          state: "failed",
          error: DESCRIBED,
          output: ["Error: UPGRADE FAILED: timed out waiting for the condition"],
        }),
      ],
    });

    // Verbatim: `helmOps` already ran this through `describeError`, and a
    // sentence described twice is classified on its own wording.
    expect(screen.getByText(DESCRIBED)).toBeTruthy();
    expect(screen.queryByText(/TLS certificate couldn't be verified/)).toBeNull();
    expect(screen.getByText("Error: UPGRADE FAILED: timed out waiting for the condition")).toBeTruthy();
    expect(screen.queryByText('tag: "4f2a1c"')).toBeNull();
  });

  it("lets a failed operation be dismissed, and only then", async () => {
    const onDismiss = vi.fn();
    mount({ ops: [op({ id: 7, state: "failed", error: "no such release" })], onDismiss });
    await userEvent.click(screen.getByRole("button", { name: /dismiss/i }));
    expect(onDismiss).toHaveBeenCalledWith(7);
  });
});

describe("ReleasePane — the diff", () => {
  it("renders the selected release's last revision against the one before it", async () => {
    const { invoke } = mount();

    await waitFor(() => expect(screen.getByText('tag: "4f2a1c"')).toBeTruthy());
    expect(screen.getByText('tag: "118a7e"')).toBeTruthy();
    expect(screen.getByText('DB_POOL_MAX: "5"')).toBeTruthy();
    expect(head()).toContain("checkout · 118 → 119 · rendered diff");
    expect(invoke).toHaveBeenCalledTimes(2);
    expect(invoke).toHaveBeenCalledWith(
      "k8s.getHelmRelease",
      expect.objectContaining({ context: CONTEXT, name: "checkout", revision: 118 }),
    );
    expect(invoke).toHaveBeenCalledWith(
      "k8s.getHelmRelease",
      expect.objectContaining({ context: CONTEXT, name: "checkout", revision: 119 }),
    );
  });

  it("tones the head badge with core's verdict for helm's own word", async () => {
    mount();
    await waitFor(() => expect(screen.getByText('tag: "4f2a1c"')).toBeTruthy());
    const badge = document.querySelector(".pane-head .badge");
    expect(badge?.textContent).toBe("failed");
    expect(badge?.getAttribute("data-tone")).toBe("sev");
  });

  it("says a one-revision release has nothing to compare, and asks the cluster nothing", async () => {
    const { invoke } = mount({
      release: { name: "fresh", namespace: "default", revision: 1, status: "deployed" },
    });

    expect(screen.getByText(/nothing to compare/i)).toBeTruthy();
    expect(screen.getByText(/revision 1, its first/i)).toBeTruthy();
    // The claim is NOT "no changes" — that says the two revisions matched.
    expect(screen.queryByText(/render the same manifest/i)).toBeNull();
    expect(invoke).not.toHaveBeenCalled();
    expect(document.querySelector('[data-slot="line"]')).toBeNull();
  });

  it("says two identical revisions match, which is a different claim", async () => {
    const invoke = vi.fn(async () => ({
      name: "steady",
      namespace: "default",
      revision: 4,
      status: "deployed",
      chart: "c",
      chartVersion: "1",
      appVersion: "1",
      updated: "",
      valuesYaml: "",
      manifest: "replicas: 1\n",
      notes: "",
      history: [],
    })) as unknown as Invoker;

    mount({
      release: { name: "steady", namespace: "default", revision: 4, status: "deployed" },
      invoke,
    });

    await waitFor(() => expect(screen.getByText(/render the same manifest/i)).toBeTruthy());
    // ...and NOT the sentence a first revision gets.
    expect(screen.queryByText(/nothing to compare/i)).toBeNull();
    expect(screen.queryByText(/its first/i)).toBeNull();
    // Nor the manifest itself printed back as context lines, which is what
    // `diffTextLines` hands over for an unchanged pair.
    expect(document.querySelector('[data-slot="line"]')).toBeNull();
  });

  it("follows the selection rather than staying on one release", async () => {
    const invoke = invoker();
    const { rerender } = render(
      <ReleasePane context={CONTEXT} release={CHECKOUT} invoke={invoke} ops={[]} />,
    );
    await waitFor(() => expect(screen.getByText('tag: "4f2a1c"')).toBeTruthy());

    rerender(<ReleasePane context={CONTEXT} release={PAYMENTS} invoke={invoke} ops={[]} />);
    await waitFor(() => expect(screen.getByText("replicas: 4")).toBeTruthy());
    expect(screen.queryByText('tag: "4f2a1c"')).toBeNull();
    expect(head()).toContain("payments · 61 → 62 · rendered diff");
  });

  it("changes which operation is considered when the selection changes", async () => {
    const invoke = invoker();
    const rows = [op({ output: ["upgrading checkout"] })];
    const { rerender } = render(
      <ReleasePane context={CONTEXT} release={CHECKOUT} invoke={invoke} ops={rows} />,
    );
    expect(screen.getByText("upgrading checkout")).toBeTruthy();

    rerender(<ReleasePane context={CONTEXT} release={PAYMENTS} invoke={invoke} ops={rows} />);
    await waitFor(() => expect(screen.getByText("replicas: 4")).toBeTruthy());
    expect(screen.queryByText("upgrading checkout")).toBeNull();
  });

  it("describes a refused read rather than printing what the backend said", async () => {
    const invoke = vi.fn(async () => {
      throw new Error("ServiceError: client error (Connect): connection refused");
    }) as unknown as Invoker;
    mount({ invoke });

    await waitFor(() =>
      expect(screen.getByText(/The connection to the API server could not be made/)).toBeTruthy(),
    );
  });

  it("has nothing to show until a release is selected", () => {
    const { invoke } = mount({ release: null });
    expect(screen.getByText(/No release selected/i)).toBeTruthy();
    expect(invoke).not.toHaveBeenCalled();
  });
});

describe("ReleasePane — the frame", () => {
  it("is §16's fixed 420px column", () => {
    mount({ release: null });
    const pane = document.querySelector("aside");
    expect(pane?.style.width).toBe("420px");
  });

  it("scrolls its body rather than the whole pane", () => {
    mount({ release: null });
    expect(document.querySelector('[data-slot="pane-body"]')?.className).toContain("side-rail-body");
  });

  it("offers §16's footer only where a rollback target exists", async () => {
    const onRollback = vi.fn();
    const onValuesEditor = vi.fn();
    mount({ onRollback, onValuesEditor });
    await waitFor(() => expect(screen.getByText('tag: "4f2a1c"')).toBeTruthy());

    await userEvent.click(screen.getByRole("button", { name: "Roll back to 118" }));
    expect(onRollback).toHaveBeenCalledWith(118);
    await userEvent.click(screen.getByRole("button", { name: "Values editor" }));
    expect(onValuesEditor).toHaveBeenCalled();
  });

  it("offers no rollback on a release that has never been upgraded", () => {
    mount({
      release: { name: "fresh", namespace: "default", revision: 1, status: "deployed" },
      onRollback: vi.fn(),
      onValuesEditor: vi.fn(),
    });
    expect(screen.queryByRole("button", { name: /Roll back/ })).toBeNull();
  });

  it("offers no rollback while an operation holds the pane", () => {
    mount({ ops: [op({ output: ["working"] })], onRollback: vi.fn(), onValuesEditor: vi.fn() });
    expect(screen.queryByRole("button", { name: /Roll back/ })).toBeNull();
  });
});
