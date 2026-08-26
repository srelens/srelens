import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * **The store is the only thing stubbed.** `helmArgv` stays real, because the
 * argv it builds is what the cluster will actually be told to do, and a test
 * that stubbed it would assert the dialog calls a stub. `helmStatus` stays real
 * for the same reason the rest of this migration keeps it real: the word in
 * rollback's hint is core's, not this file's.
 */
const store = vi.hoisted(() => ({ startHelmOperation: vi.fn() }));
vi.mock("../../lib/helmOps", async (orig) => ({
  ...(await orig<typeof import("../../lib/helmOps")>()),
  startHelmOperation: store.startHelmOperation,
}));

import type { HelmRevision } from "@srelens/core";
import { HelmOpDialog, helmArgv, helmCommand } from "./HelmOpDialog";

const CONTEXT = "prod-eu";
const NAMESPACE = "checkout";
const RELEASE = "checkout-api";
const CHART = "bitnami/nginx";

/**
 * A release that has been upgraded twice and whose latest revision failed —
 * the shape a reader is actually in when they reach for rollback.
 */
const HISTORY: HelmRevision[] = [
  { revision: 1, status: "superseded", updated: "2026-08-01", chartVersion: "18.0.0", description: "Install complete" },
  { revision: 2, status: "superseded", updated: "2026-08-10", chartVersion: "18.1.0", description: "Upgrade complete" },
  { revision: 3, status: "deployed", updated: "2026-08-20", chartVersion: "18.2.0", description: "Upgrade complete" },
  { revision: 4, status: "failed", updated: "2026-08-24", chartVersion: "18.3.0", description: "Upgrade failed" },
];

beforeEach(() => {
  vi.clearAllMocks();
  store.startHelmOperation.mockResolvedValue(1);
});

/** The command the dialog is showing, exactly as the reader reads it. */
function shown(): string {
  const node = document.querySelector(".copy-command-text");
  return node?.textContent ?? "";
}

function open(props: Partial<Parameters<typeof HelmOpDialog>[0]> = {}) {
  const onClose = vi.fn();
  render(
    <HelmOpDialog
      kind="install"
      context={CONTEXT}
      namespace={NAMESPACE}
      release={RELEASE}
      chart={CHART}
      onClose={onClose}
      {...props}
    />,
  );
  return onClose;
}

function button(name: string): HTMLButtonElement {
  return screen.getByRole("button", { name }) as HTMLButtonElement;
}

function field(label: string): HTMLElement {
  return screen.getByLabelText(label) as HTMLElement;
}

/**
 * A field whose `Field` carries a hint.
 *
 * The kit renders the hint INSIDE the `<label>`, so the control's accessible
 * name is the label followed by the hint; an exact match would never find it.
 */
function hintedField(label: string): HTMLElement {
  // No word boundary: the label and the hint are adjacent spans, so the
  // computed name runs the two together with nothing between them.
  return screen.getByLabelText(new RegExp(`^${label}`)) as HTMLElement;
}

describe("HelmOpDialog — §A.5's frame", () => {
  it("installs in a 620px dialog titled for the operation, with §A.5's fields", async () => {
    open();
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText(`Install ${RELEASE}`)).toBeTruthy();
    expect((dialog as HTMLElement).style.maxWidth).toBe("620px");
    expect(field("Chart")).toBeTruthy();
    expect(field("Chart version")).toBeTruthy();
    expect(screen.getByRole("tablist", { name: "Panel" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Values" })).toBeTruthy();
    expect(screen.getByRole("tab", { name: "Rendered diff" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "atomic" })).toBeTruthy();
    expect(screen.getByRole("switch", { name: "wait" })).toBeTruthy();
    expect(screen.getByText("Equivalent command")).toBeTruthy();
    expect(button("Cancel")).toBeTruthy();
    expect(button("Install")).toBeTruthy();
  });

  it("titles and names each of the other three the way §A.5 writes them", async () => {
    open({ kind: "upgrade" });
    expect(await screen.findByText(`Upgrade ${RELEASE}`)).toBeTruthy();
    expect(button("Upgrade")).toBeTruthy();
  });

  it("names rollback's button for the revision it will land on", async () => {
    open({ kind: "rollback", history: HISTORY, revision: 4 });
    expect(await screen.findByText(`Roll back ${RELEASE}`)).toBeTruthy();
    expect(button("Roll back to 3")).toBeTruthy();
    // §A.5 gives rollback no chart fields and no panel.
    expect(screen.queryByLabelText("Chart")).toBeNull();
    expect(screen.queryByRole("tablist", { name: "Panel" })).toBeNull();
  });

  it("draws uninstall's own button as the destructive one", async () => {
    open({ kind: "uninstall" });
    expect(await screen.findByText(`Uninstall ${RELEASE}`)).toBeTruthy();
    expect(button("Uninstall").dataset.variant).toBe("danger");
  });

  it("closes on Cancel without starting anything", async () => {
    const onClose = open();
    await userEvent.click(button("Cancel"));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(store.startHelmOperation).not.toHaveBeenCalled();
  });
});

describe("HelmOpDialog — the equivalent command", () => {
  it("reads the argv helm will be given, for each of the four", () => {
    expect(helmCommand({ kind: "install", release: RELEASE, namespace: NAMESPACE, chart: CHART, chartVersion: "", revision: null, atomic: false, wait: false })).toBe(
      `helm install ${RELEASE} ${CHART} --namespace ${NAMESPACE}`,
    );
    expect(helmCommand({ kind: "upgrade", release: RELEASE, namespace: NAMESPACE, chart: CHART, chartVersion: "18.3.0", revision: null, atomic: true, wait: true })).toBe(
      `helm upgrade ${RELEASE} ${CHART} --namespace ${NAMESPACE} --version 18.3.0 --atomic --wait`,
    );
    expect(helmCommand({ kind: "rollback", release: RELEASE, namespace: NAMESPACE, chart: "", chartVersion: "", revision: 3, atomic: false, wait: false })).toBe(
      `helm rollback ${RELEASE} 3 --namespace ${NAMESPACE}`,
    );
    expect(helmCommand({ kind: "uninstall", release: RELEASE, namespace: NAMESPACE, chart: "", chartVersion: "", revision: null, atomic: false, wait: false })).toBe(
      `helm uninstall ${RELEASE} --namespace ${NAMESPACE}`,
    );
  });

  it("follows the fields as they change", async () => {
    open({ kind: "upgrade" });
    await screen.findByRole("dialog");
    expect(shown()).toBe(`helm upgrade ${RELEASE} ${CHART} --namespace ${NAMESPACE}`);
    await userEvent.type(field("Chart version"), "18.3.0");
    expect(shown()).toContain("--version 18.3.0");
    await userEvent.click(screen.getByRole("switch", { name: "atomic" }));
    expect(shown()).toBe(
      `helm upgrade ${RELEASE} ${CHART} --namespace ${NAMESPACE} --version 18.3.0 --atomic`,
    );
    await userEvent.clear(field("Chart version"));
    expect(shown()).toBe(`helm upgrade ${RELEASE} ${CHART} --namespace ${NAMESPACE} --atomic`);
  });

  it("quotes a value a shell would otherwise split, and still submits it as one argument", async () => {
    const onClose = open({ chart: "./charts/my chart" });
    await screen.findByRole("dialog");
    expect(shown()).toBe(
      `helm install ${RELEASE} './charts/my chart' --namespace ${NAMESPACE}`,
    );
    await userEvent.click(button("Install"));
    await waitFor(() => expect(store.startHelmOperation).toHaveBeenCalled());
    expect(store.startHelmOperation.mock.calls[0][0].args).toEqual([
      "install",
      RELEASE,
      "./charts/my chart",
      "--namespace",
      NAMESPACE,
    ]);
    expect(onClose).toHaveBeenCalled();
  });

  it("escapes an apostrophe rather than closing the quote it opened", async () => {
    // The wrapping half of `quoted` is not the whole of it. Left unescaped,
    // the displayed line reads `'./charts/dev's chart'`, which a shell splices
    // into `./charts/devs chart` — no error, and a DIFFERENT path than the one
    // srelens ran. That silent divergence is exactly what showing the command
    // is for.
    open({ chart: "./charts/dev's chart" });
    await screen.findByRole("dialog");
    expect(shown()).toBe(
      `helm install ${RELEASE} './charts/dev'\\''s chart' --namespace ${NAMESPACE}`,
    );
    await userEvent.click(button("Install"));
    await waitFor(() => expect(store.startHelmOperation).toHaveBeenCalled());
    // The argv is unquoted either way: quoting is a fact about shells, and
    // nothing between here and helm is one.
    expect(store.startHelmOperation.mock.calls[0][0].args[2]).toBe("./charts/dev's chart");
  });

  it("submits exactly the argv it displayed", async () => {
    open({ kind: "upgrade" });
    await screen.findByRole("dialog");
    await userEvent.type(field("Chart version"), "18.3.0");
    await userEvent.click(screen.getByRole("switch", { name: "wait" }));
    const displayed = shown();
    await userEvent.click(button("Upgrade"));
    await waitFor(() => expect(store.startHelmOperation).toHaveBeenCalled());
    const { args } = store.startHelmOperation.mock.calls[0][0];
    expect(["helm", ...args].join(" ")).toBe(displayed);
  });
});

describe("HelmOpDialog — uninstall's gate", () => {
  it("stays shut until the release name is typed exactly", async () => {
    open({ kind: "uninstall" });
    await screen.findByRole("dialog");
    const input = field(`Type ${RELEASE} to confirm`);
    expect(button("Uninstall").disabled).toBe(true);

    // A prefix is not the name.
    await userEvent.type(input, "checkout-ap");
    expect(button("Uninstall").disabled).toBe(true);

    // Nor is the name with something after it.
    await userEvent.clear(input);
    await userEvent.type(input, `${RELEASE}-web`);
    expect(button("Uninstall").disabled).toBe(true);

    // Nor is the name with a stray space, which is what a paste leaves.
    await userEvent.clear(input);
    await userEvent.type(input, `${RELEASE} `);
    expect(button("Uninstall").disabled).toBe(true);

    // Nor the same letters in another case.
    await userEvent.clear(input);
    await userEvent.type(input, RELEASE.toUpperCase());
    expect(button("Uninstall").disabled).toBe(true);

    await userEvent.clear(input);
    await userEvent.type(input, RELEASE);
    expect(button("Uninstall").disabled).toBe(false);
  });

  it("stays shut when there is no release name to type", async () => {
    // Task 8 can mount this before a selection resolves, or on a release whose
    // name failed to load. Without the guard `complete` is true, the field
    // reads `Type  to confirm`, and `typed === release` is `"" === ""` — so the
    // danger button opens the moment the dialog does, with nothing typed, and
    // one click submits `helm uninstall "" --namespace <ns>`.
    open({ kind: "uninstall", release: "" });
    await screen.findByRole("dialog");
    expect(button("Uninstall").disabled).toBe(true);
  });

  it("says what an uninstall removes without inventing a count of it", async () => {
    open({ kind: "uninstall" });
    const alert = await screen.findByRole("alert");
    expect(alert.textContent).toContain("every object in the release");

    /**
     * **The exact sentence, on purpose.** A change-detector test over copy is
     * normally a smell — it fails on edits that broke nothing and teaches
     * people to update expectations without reading them. Here that is the
     * mechanism, not the cost.
     *
     * srelens has not counted this release's objects and must not say how many
     * there are, and this is the one screen in the app where being confidently
     * wrong cannot be undone. Every property-shaped pin tried here leaked: a
     * spelled-out-number alternation forbade ordinary prose ("in one pass"),
     * and dropping it let "a dozen objects", "twelve things" and "several
     * resources go" through — a fabricated quantity CAN avoid naming a
     * Kubernetes kind, by counting a generic noun instead. So the pin is the
     * whole sentence: any edit to this alert fails this test, whoever makes it
     * comes back here, re-reads why the count is absent, and updates the
     * expectation deliberately.
     *
     * A narrow, well-justified exception for one irreversible screen. It is
     * not a pattern to spread to other copy.
     */
    const body = alert.querySelector('[data-slot="alert-body"]');
    expect(body?.textContent?.replace(/\s+/g, " ").trim()).toBe(
      "Everything helm created for it goes, and nothing here can undo it. " +
        "Persistent volume claims are kept unless the chart marks them for deletion.",
    );

    // The secondary net, kept for what it says rather than for what it adds:
    // no digit anywhere in the alert, and no Kubernetes kind named as being in
    // this particular release.
    expect(alert.textContent).not.toMatch(/\d/);
    expect(alert.textContent).not.toMatch(
      /\b(pods?|services?|ingress(es)?|configmaps?|secrets?|deployments?|statefulsets?|jobs?)\b/i,
    );
  });
});

describe("HelmOpDialog — rollback's gate", () => {
  it("asks once, without typing, and names the last revision that reached deployed", async () => {
    open({ kind: "rollback", history: HISTORY, revision: 4 });
    await screen.findByRole("dialog");
    // No typed confirmation anywhere: rollback is recoverable, and the reader
    // is here because something is already broken.
    expect(screen.queryByLabelText(`Type ${RELEASE} to confirm`)).toBeNull();

    const hint = screen.getByText(/newest one helm does not report failed/i);
    expect(hint.textContent).toContain("Revision 3");
    // The word is core's `helmStatus`, not a table in the dialog.
    expect(hint.textContent).toContain("deployed");

    const confirm = screen.getByRole("checkbox");
    expect(button("Roll back to 3").disabled).toBe(true);
    await userEvent.click(confirm);
    expect(button("Roll back to 3").disabled).toBe(false);
  });

  it("will not default to a revision whose status this build cannot read", async () => {
    // core's `neutral` is its UNKNOWN bucket as well as its `superseded` one —
    // "it might be a failure this table has no name for". A status word helm
    // adds tomorrow must not become the default target of a rollback, and
    // neither must a revision left behind by `uninstall --keep-history`.
    open({
      kind: "rollback",
      revision: 6,
      history: [
        ...HISTORY,
        { revision: 5, status: "uninstalled", updated: "2026-08-25", chartVersion: "18.3.0", description: "Uninstalled" },
        { revision: 6, status: "quiesced", updated: "2026-08-26", chartVersion: "18.4.0", description: "Something new" },
      ],
    });
    await screen.findByRole("dialog");
    // Revision 3 — the newest one that is actually deployed — not 5 and not 6.
    expect(button("Roll back to 3")).toBeTruthy();
    expect((hintedField("Target revision") as HTMLInputElement).value).toBe("3");
  });

  it("keeps a superseded revision, which is what a good one becomes", async () => {
    // The other half of the same rule: `superseded` shares core's `neutral`
    // bucket with the unknowns, and it is the ordinary state of every revision
    // that WAS deployed and then replaced — the usual thing to roll back to.
    open({
      kind: "rollback",
      revision: 3,
      history: HISTORY.filter((r) => r.revision !== 4),
    });
    await screen.findByRole("dialog");
    expect(button("Roll back to 2")).toBeTruthy();
  });

  it("says which of the three reasons it has no revision to offer", async () => {
    // No history at all.
    const first = render(
      <HelmOpDialog kind="rollback" context={CONTEXT} namespace={NAMESPACE} release={RELEASE} onClose={vi.fn()} />,
    );
    expect(screen.getByText(/no history for this release/i)).toBeTruthy();
    first.unmount();

    // A history with nothing in it but the revision running now: true, and a
    // different fact from having no history.
    const second = render(
      <HelmOpDialog
        kind="rollback"
        context={CONTEXT}
        namespace={NAMESPACE}
        release={RELEASE}
        revision={1}
        history={[HISTORY[0]].map((r) => ({ ...r, revision: 1, status: "deployed" }))}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText(/only the revision running now/i)).toBeTruthy();
    expect(screen.queryByText(/no history for this release/i)).toBeNull();
    second.unmount();

    // A history of earlier revisions, none of them safe to offer. This is the
    // case the old single sentence lied about, and the one where the reader
    // most needs to be told the truth: the release HAS history, and none of it
    // is a way out.
    open({
      kind: "rollback",
      revision: 3,
      history: [
        { revision: 1, status: "failed", updated: "2026-08-01", chartVersion: "18.0.0", description: "Install failed" },
        { revision: 2, status: "quiesced", updated: "2026-08-10", chartVersion: "18.1.0", description: "Something new" },
        { revision: 3, status: "failed", updated: "2026-08-20", chartVersion: "18.2.0", description: "Upgrade failed" },
      ],
    });
    await screen.findByRole("dialog");
    expect(screen.getByText(/no earlier revision is safe to offer/i)).toBeTruthy();
    expect(screen.queryByText(/no history for this release/i)).toBeNull();
    // The rest of the degrade path stays as it was: nothing filled in, no
    // command printed, and the button dead.
    expect((hintedField("Target revision") as HTMLInputElement).value).toBe("");
    expect(screen.getByText("Name a revision to see it.")).toBeTruthy();
    expect(button("Roll back").disabled).toBe(true);
  });

  it("takes another revision when the reader names one", async () => {
    open({ kind: "rollback", history: HISTORY, revision: 4 });
    await screen.findByRole("dialog");
    const target = hintedField("Target revision");
    await userEvent.clear(target);
    await userEvent.type(target, "2");
    expect(shown()).toBe(`helm rollback ${RELEASE} 2 --namespace ${NAMESPACE}`);
    expect(button("Roll back to 2")).toBeTruthy();
    await userEvent.clear(target);
    expect(button("Roll back").disabled).toBe(true);
  });
});

describe("HelmOpDialog — submitting", () => {
  it("hands the store the operation and closes without waiting for it", async () => {
    // A promise that never settles: the dialog must close anyway. Waiting for
    // the operation is the whole thing the store exists to avoid.
    store.startHelmOperation.mockReturnValue(new Promise<number>(() => {}));
    const onClose = open({ kind: "upgrade", values: "replicaCount: 2", extraKubeconfigs: ["/tmp/extra"] });
    await screen.findByRole("dialog");
    await userEvent.click(button("Upgrade"));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(store.startHelmOperation).toHaveBeenCalledTimes(1);
    expect(store.startHelmOperation.mock.calls[0][0]).toMatchObject({
      kind: "upgrade",
      release: RELEASE,
      namespace: NAMESPACE,
      context: CONTEXT,
      values: "replicaCount: 2",
      extraKubeconfigs: ["/tmp/extra"],
      args: helmArgv({
        kind: "upgrade",
        release: RELEASE,
        namespace: NAMESPACE,
        chart: CHART,
        chartVersion: "",
        revision: null,
        atomic: false,
        wait: false,
      }),
    });
  });

  it("sends no values body for the two operations that take none", async () => {
    open({ kind: "uninstall", values: "replicaCount: 2" });
    await screen.findByRole("dialog");
    await userEvent.type(field(`Type ${RELEASE} to confirm`), RELEASE);
    await userEvent.click(button("Uninstall"));
    await waitFor(() => expect(store.startHelmOperation).toHaveBeenCalled());
    expect(store.startHelmOperation.mock.calls[0][0].values).toBeUndefined();
  });

  it("shows the caller's rendered diff on the panel that asks for one", async () => {
    open({
      kind: "upgrade",
      diff: [
        { tag: "same", left: "replicas: 1", right: "replicas: 1" },
        { tag: "replace", left: "image: nginx:1.25", right: "image: nginx:1.27" },
      ],
    });
    await screen.findByRole("dialog");
    await userEvent.click(screen.getByRole("tab", { name: "Rendered diff" }));
    expect(screen.getByText(/image: nginx:1\.27/)).toBeTruthy();
  });
});
