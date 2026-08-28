import { describe, it, expect, vi, beforeEach } from "vitest";
import { createElement, useLayoutEffect, type ReactElement, type ReactNode } from "react";
import { render as renderBare, screen, waitFor, within, type RenderOptions } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { CrdRef, EventSummary, K8sObject } from "@srelens/core";
import { toneColor } from "@srelens/ui-kit";
import type { KindDescriptor, ListRow } from "../../lib/kinds/types";

// `useObject` reads `getObject`; the YAML and Events panes read `getManifest`
// and `listEvents` directly, and the YAML pane also reads `listCrds` to
// resolve a custom resource's group/version/plural before fetching its
// manifest. All four are core's, mocked here so a test controls what "the
// cluster said" without one — `importOriginal` keeps everything else
// (K8S_KIND, and the real types) intact.
const { getObject, getManifest, listEvents, listCrds, deleteResource } = vi.hoisted(() => ({
  getObject: vi.fn(async (): Promise<{ object?: K8sObject; error?: string }> => ({})),
  getManifest: vi.fn(async (): Promise<{ yaml?: string; error?: string }> => ({ yaml: "" })),
  listEvents: vi.fn(async (): Promise<{ events?: EventSummary[]; error?: string }> => ({ events: [] })),
  listCrds: vi.fn(async (): Promise<{ crds?: CrdRef[]; error?: string }> => ({ crds: [] })),
  // The footer's actions are the row menu's, so the one write a test reaches
  // for is mocked here too — a confirm that is never taken must reach nothing.
  deleteResource: vi.fn(async (): Promise<{ error?: string }> => ({})),
}));

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  getObject,
  getManifest,
  listEvents,
  listCrds,
  deleteResource,
}));

// The shell asks the same descriptor the list screen resolves, only to read
// `panes` off it — mocked so a test can hand it a kind with or without
// Containers/Metrics without depending on which real kinds have that set.
const { descriptorFor } = vi.hoisted(() => ({
  descriptorFor: vi.fn((_slug: string): KindDescriptor<ListRow> | undefined => undefined),
}));

vi.mock("../../lib/kinds/descriptors", () => ({ descriptorFor }));

// The kit's `CodeEditor`, unchanged — wrapped only to record what the YAML
// pane hands it. CodeMirror compiles its sizing into a generated stylesheet
// with hashed class names and jsdom applies no CSS, so what the editor was
// ASKED for is the only thing observable here (`CodeEditor.test.tsx` says the
// same, at greater length). Everything else in the kit stays real.
const { codeEditorProps } = vi.hoisted(() => ({ codeEditorProps: [] as Record<string, unknown>[] }));

vi.mock("@srelens/ui-kit", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/ui-kit")>();
  return {
    ...actual,
    CodeEditor: (props: Record<string, unknown>) => {
      codeEditorProps.push({ ...props });
      return createElement(actual.CodeEditor, props as never);
    },
  };
});

import { ConsoleProvider, useConsole } from "../../console";
import { loadSectionFolds, setSectionOpen } from "../../lib/sectionFolds";
import { detailFacts } from "./detailData";
import { ResourceDetailView } from "./ResourceDetailView";
import { ResourceTabView } from "./ResourceTabView";

/** Every question the console was handed, in order. */
const asked: string[] = [];

/** Stands in for the console dock: the thing `ask` delivers to. */
function AskProbe() {
  const { registerSubmit } = useConsole();
  useLayoutEffect(() => registerSubmit((question) => void asked.push(question)), [registerSubmit]);
  return null;
}

/**
 * Every render goes through the provider the real shell mounts at the root:
 * the pane's footer reaches `useConsole()` for its Ask button, and that hook
 * throws rather than quietly handing back nothing. The probe rides along so a
 * test can read what was actually asked.
 */
function Wrapper({ children }: { children: ReactNode }) {
  return (
    <ConsoleProvider>
      <AskProbe />
      {children}
    </ConsoleProvider>
  );
}

function render(ui: ReactElement, options?: Omit<RenderOptions, "wrapper">) {
  return renderBare(ui, { wrapper: Wrapper, ...options });
}

const POD: K8sObject = {
  kind: "Pod",
  apiVersion: "v1",
  metadata: { name: "web-1", namespace: "default" },
};

const POD_2: K8sObject = {
  kind: "Pod",
  apiVersion: "v1",
  metadata: { name: "web-2", namespace: "default" },
};

const CONFIGMAP: K8sObject = {
  kind: "ConfigMap",
  apiVersion: "v1",
  metadata: { name: "cm-1", namespace: "default" },
};

/** Scans the whole rendered document for a substring — text content, `title`,
 *  `aria-label`, `data-*`, everything, including markup a screen reader or a
 *  DOM inspector would see even while visually hidden. A boolean assertion
 *  rather than an element query, so a failure here never prints the secret
 *  text into the test output. Same helper, and the same reasoning, as
 *  `SecretBody.test.tsx`'s. */
function documentContains(value: string): boolean {
  return document.body.innerHTML.includes(value);
}

/**
 * Every fact the body derived, by its LABEL.
 *
 * By label rather than by markup on purpose: the peek reads its facts down a
 * two-column list and the full tab reads them across three columns of
 * label-above-value, so the two hosts' DOM differs by design. What must not
 * differ is which facts were derived at all.
 */
const factLabels = (): string[] =>
  Array.from(document.querySelectorAll(".kv-k")).map((el) => el.textContent ?? "");

/** The labels of one block's rows, in the order they read — the peek's lead
 *  fact list (`.fact-list`), or the full tab's grid. Scoped, because "equals
 *  the derived list" is only a claim about the block that draws it. */
const labelsIn = (selector: string): string[] =>
  Array.from(document.querySelectorAll(`${selector} .kv-k`)).map((el) => el.textContent ?? "");

// Obviously-fake fixture text — never anything that reads as a real
// credential, per this screen's secrecy ruling.
const FIXTURE_B64 = "ZmFrZS1maXh0dXJlLW5vdC1hLXJlYWwtc2VjcmV0";

const SECRET: K8sObject = {
  kind: "Secret",
  apiVersion: "v1",
  metadata: { name: "s-1", namespace: "default" },
};

const DAY_MS = 24 * 60 * 60 * 1000;

/** An ISO timestamp a whole number of days old, so `ageFromTimestamp` reads it
 *  back as exactly that many days. Relative rather than a fixed date: the
 *  clock cannot be frozen here (`userEvent` needs real timers) and a literal
 *  stamp would rot into a bigger number every day. */
function daysAgo(days: number): string {
  return new Date(Date.now() - days * DAY_MS).toISOString();
}

/** Frame A of the mock: a Deployment short of its replicas. */
const DEGRADED_DEPLOYMENT: K8sObject = {
  kind: "Deployment",
  apiVersion: "apps/v1",
  metadata: { name: "checkout-api", namespace: "checkout", creationTimestamp: daysAgo(84) },
  spec: { replicas: 12 },
  status: { readyReplicas: 9 },
};

/** Frame B of the mock: a Pod doing exactly what it was asked to. */
const RUNNING_POD: K8sObject = {
  kind: "Pod",
  apiVersion: "v1",
  metadata: { name: "cart-session-store-0", namespace: "checkout", creationTimestamp: daysAgo(211) },
  status: { phase: "Running", containerStatuses: [{ name: "redis", ready: true, state: { running: {} } }] },
};

/** A kind `resourceStatusLine` has no verdict for, aged so an age fact would
 *  have something to draw if one were drawn at all. */
const AGED_CONFIGMAP: K8sObject = {
  kind: "ConfigMap",
  apiVersion: "v1",
  metadata: { name: "cm-1", namespace: "default", creationTimestamp: daysAgo(30) },
};

function baseDescriptor(overrides: Partial<KindDescriptor<ListRow>> = {}): KindDescriptor<ListRow> {
  return { k8sKind: "Pod", columns: [], source: "watch", scope: "namespaced", actions: {}, ...overrides };
}

describe("ResourceDetailView", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getManifest.mockResolvedValue({ yaml: "kind: Pod\n" });
    listEvents.mockResolvedValue({ events: [] });
    listCrds.mockResolvedValue({ crds: [] });
    descriptorFor.mockReturnValue(undefined);
    codeEditorProps.length = 0;
    asked.length = 0;
    // Which blocks are open is a module-level store that outlives a render,
    // exactly as it outlives a launch. Cleared, so no test inherits another's
    // choices — least of all one about a Secret.
    localStorage.clear();
    loadSectionFolds();
  });

  /**
   * Open a titled block, the way a reader does. Every one of them opens shut
   * on a first visit — "first open should keep everything collapsed" — so a
   * test that reads what is inside one asks for it first.
   */
  async function expand(name: string) {
    await userEvent.click(screen.getByRole("button", { name }));
  }

  it("shows a loading state while the object is in flight", () => {
    getObject.mockImplementation(() => new Promise(() => {}));
    const { getByText } = render(<ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />);
    expect(getByText(/loading/i)).toBeDefined();
  });

  it("renders Details, YAML and Events once ready, and no Containers or Metrics for a kind whose descriptor doesn't offer them", async () => {
    getObject.mockResolvedValue({ object: POD });
    const { getByRole, queryByRole } = render(
      <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "Details" })).toBeDefined());
    expect(getByRole("tab", { name: "YAML" })).toBeDefined();
    expect(getByRole("tab", { name: "Events" })).toBeDefined();
    expect(queryByRole("tab", { name: "Containers" })).toBeNull();
    expect(queryByRole("tab", { name: "Metrics" })).toBeNull();
  });

  it("names the object in the error state", async () => {
    getObject.mockResolvedValue({ error: "forbidden" });
    const { getByRole } = render(<ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />);
    await waitFor(() => expect(getByRole("alert")).toBeDefined());
    const text = getByRole("alert").textContent ?? "";
    expect(text).toContain("Pod");
    expect(text).toContain("web-1");
  });

  it("offers Containers only for a kind whose descriptor sets panes.containers", async () => {
    getObject.mockResolvedValue({ object: POD });
    descriptorFor.mockReturnValue(baseDescriptor({ panes: { containers: true } }));
    const { getByRole, queryByRole } = render(
      <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "Containers" })).toBeDefined());
    expect(queryByRole("tab", { name: "Metrics" })).toBeNull();
  });

  it("offers Metrics only for a kind whose descriptor sets panes.metrics", async () => {
    getObject.mockResolvedValue({ object: { kind: "Node", metadata: { name: "n1" } } });
    descriptorFor.mockReturnValue(baseDescriptor({ k8sKind: "Node", scope: "cluster", panes: { metrics: true } }));
    const { getByRole, queryByRole } = render(<ResourceDetailView context="ctx" kind="Node" namespace={null} name="n1" />);
    await waitFor(() => expect(getByRole("tab", { name: "Metrics" })).toBeDefined());
    expect(queryByRole("tab", { name: "Containers" })).toBeNull();
  });

  it("loads YAML and Events lazily, only once each pane is opened, and never refetches a pane already opened", async () => {
    getObject.mockResolvedValue({ object: POD });
    const { getByRole } = render(<ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />);
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());

    // A reader who never leaves Details pays for the object alone — a peek
    // fills on nearly every row click, and YAML/Events are usually never
    // looked at.
    expect(getObject).toHaveBeenCalledTimes(1);
    expect(getManifest).not.toHaveBeenCalled();
    expect(listEvents).not.toHaveBeenCalled();

    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(getManifest).toHaveBeenCalledTimes(1));
    expect(listEvents).not.toHaveBeenCalled();

    await userEvent.click(getByRole("tab", { name: "Events" }));
    await waitFor(() => expect(listEvents).toHaveBeenCalledTimes(1));

    // Switching back to Details, then to both already-opened panes again,
    // must not re-fire any of the three loads.
    await userEvent.click(getByRole("tab", { name: "Details" }));
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await userEvent.click(getByRole("tab", { name: "Events" }));

    expect(getObject).toHaveBeenCalledTimes(1);
    expect(getManifest).toHaveBeenCalledTimes(1);
    expect(listEvents).toHaveBeenCalledTimes(1);
  });

  it("shows a loading state for the manifest while it is in flight", async () => {
    getObject.mockResolvedValue({ object: POD });
    getManifest.mockImplementation(() => new Promise(() => {}));
    const { getByRole, getByText } = render(
      <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    expect(getByText(/loading.*manifest/i)).toBeDefined();
  });

  it("renders the fetched manifest once the YAML pane is opened", async () => {
    getObject.mockResolvedValue({ object: POD });
    getManifest.mockResolvedValue({ yaml: "kind: Pod\nspec:\n  nodeName: node-7\n" });
    const { getByRole, container } = render(
      <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(container.querySelector(".cm-content")?.textContent).toContain("node-7"));
  });

  it("keeps the YAML pane usable when the manifest fetch fails", async () => {
    getObject.mockResolvedValue({ object: POD });
    getManifest.mockResolvedValue({ error: "forbidden" });
    const { getByRole } = render(<ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />);
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(getByRole("alert")).toBeDefined());
    const text = getByRole("alert").textContent ?? "";
    // Names the object that failed, same convention as the object's own
    // error state — several panes can be open at once.
    expect(text).toContain("Pod");
    expect(text).toContain("web-1");

    // "Usable" means the rest of the shell still works after the failure —
    // other tabs remain clickable and render, rather than the whole
    // component wedging on the one failed pane.
    await userEvent.click(getByRole("tab", { name: "Details" }));
    await waitFor(() => expect(getByRole("tab", { name: "Details" }).getAttribute("aria-selected")).toBe("true"));
  });

  it("renders event rows once the Events pane is opened for a resource with events", async () => {
    getObject.mockResolvedValue({ object: POD });
    listEvents.mockResolvedValue({
      events: [
        { name: "web-1.abc", namespace: "default", type: "Warning", reason: "BackOff", object: "Pod/web-1", message: "container crashed", age: "5m", count: 1 },
        { name: "web-1.def", namespace: "default", type: "Normal", reason: "Scheduled", object: "Pod/web-1", message: "assigned to node-3", age: "10m", count: 1 },
      ],
    });
    const { getByRole, getByText } = render(
      <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "Events" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "Events" }));
    await waitFor(() => expect(getByText("BackOff")).toBeDefined());
    expect(getByText("container crashed")).toBeDefined();
    expect(getByText("Scheduled")).toBeDefined();
    expect(getByText("assigned to node-3")).toBeDefined();
    // The Type cell reads BOTH halves of `eventVerdict`, not just the dot's
    // tone: the word itself is only bold/coloured (`data-bad="true"`) for a
    // Warning, plain for a Normal — the design's rule for `bad`, and the
    // second half of the one-rule promise that a `Badge` (tone only, no
    // separate word/dot treatment) could not keep.
    const warningPill = getByText("Warning");
    expect(warningPill.getAttribute("data-kind")).toBe("danger");
    expect(warningPill.getAttribute("data-bad")).toBe("true");
    const normalPill = getByText("Normal");
    expect(normalPill.getAttribute("data-kind")).toBe("neutral");
    expect(normalPill.getAttribute("data-bad")).toBeNull();
  });

  it("does not query the cluster's CRDs to fetch a built-in kind's manifest", async () => {
    getObject.mockResolvedValue({ object: POD });
    const { getByRole } = render(<ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />);
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(getManifest).toHaveBeenCalledTimes(1));
    expect(listCrds).not.toHaveBeenCalled();
    expect(getManifest).toHaveBeenCalledWith("ctx", "Pod", "default", "web-1", undefined, undefined);
  });

  it("resolves a custom resource's group/version/plural from the cluster's CRDs and passes it to getManifest", async () => {
    getObject.mockResolvedValue({ object: { kind: "Certificate", metadata: { name: "cert-1", namespace: "default" } } });
    listCrds.mockResolvedValue({
      crds: [
        {
          name: "certificates.cert-manager.io",
          group: "cert-manager.io",
          version: "v1",
          kind: "Certificate",
          plural: "certificates",
          namespaced: true,
        },
      ],
    });
    getManifest.mockResolvedValue({ yaml: "kind: Certificate\n" });
    const { getByRole } = render(
      <ResourceDetailView context="ctx" kind="Certificate" namespace="default" name="cert-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(getManifest).toHaveBeenCalledTimes(1));
    expect(getManifest).toHaveBeenCalledWith("ctx", "Certificate", "default", "cert-1", undefined, {
      group: "cert-manager.io",
      version: "v1",
      plural: "certificates",
    });
  });

  it("shows a distinct, informative error when no CRD on the cluster matches the custom resource's kind", async () => {
    getObject.mockResolvedValue({ object: { kind: "Certificate", metadata: { name: "cert-1", namespace: "default" } } });
    listCrds.mockResolvedValue({ crds: [] });
    const { getByRole } = render(
      <ResourceDetailView context="ctx" kind="Certificate" namespace="default" name="cert-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(getByRole("alert")).toBeDefined());
    const text = getByRole("alert").textContent ?? "";
    expect(text).toContain("Certificate");
    // Distinct from a generic manifest-fetch failure: names the real reason
    // (no matching CustomResourceDefinition), not a bare "could not load".
    expect(text.toLowerCase()).toContain("customresourcedefinition");
    expect(getManifest).not.toHaveBeenCalled();
  });

  it("shows a distinct error when the cluster's CRDs themselves fail to load", async () => {
    getObject.mockResolvedValue({ object: { kind: "Certificate", metadata: { name: "cert-1", namespace: "default" } } });
    listCrds.mockResolvedValue({ error: "forbidden" });
    const { getByRole } = render(
      <ResourceDetailView context="ctx" kind="Certificate" namespace="default" name="cert-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(getByRole("alert")).toBeDefined());
    const text = getByRole("alert").textContent ?? "";
    expect(text).toContain("Certificate");
    expect(text).toContain("forbidden");
    expect(getManifest).not.toHaveBeenCalled();
  });

  it("shows a labelled empty state for a resource with no events, not a blank pane", async () => {
    getObject.mockResolvedValue({ object: POD });
    listEvents.mockResolvedValue({ events: [] });
    const { getByRole, getByText, queryByRole } = render(
      <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "Events" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "Events" }));
    await waitFor(() => expect(getByText("No events")).toBeDefined());
    // Distinguishable from the error state below: no alert renders alongside it.
    expect(queryByRole("alert")).toBeNull();
  });

  it("shows the error state for events that failed to load, distinct from the empty-events state", async () => {
    getObject.mockResolvedValue({ object: POD });
    listEvents.mockResolvedValue({ error: "forbidden" });
    const { getByRole, queryByText } = render(
      <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "Events" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "Events" }));
    await waitFor(() => expect(getByRole("alert")).toBeDefined());
    expect(getByRole("alert").textContent ?? "").toContain("web-1");
    // Distinguishable from the empty state above: no "No events" label renders.
    expect(queryByText("No events")).toBeNull();
  });

  it("does not reuse a previously-opened pane's data after the subject changes on an already-mounted shell", async () => {
    getObject.mockResolvedValueOnce({ object: POD });
    getManifest.mockResolvedValueOnce({ yaml: "kind: Pod\nmetadata:\n  name: web-1\n" });

    const { getByRole, container, rerender } = render(
      <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(container.querySelector(".cm-content")?.textContent).toContain("web-1"));

    // A different pod, same shell instance — the peek fills like this on
    // nearly every row click.
    getObject.mockResolvedValueOnce({ object: POD_2 });
    getManifest.mockResolvedValueOnce({ yaml: "kind: Pod\nmetadata:\n  name: web-2\n" });
    rerender(<ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-2" />);

    await waitFor(() => expect(container.querySelector(".cm-content")?.textContent).toContain("web-2"));
    expect(container.querySelector(".cm-content")?.textContent).not.toContain("web-1");
    expect(getManifest).toHaveBeenCalledTimes(2);
  });

  it("persists the selected pane across a subject change when the new subject's kind also offers it", async () => {
    getObject.mockResolvedValueOnce({ object: POD });
    const { getByRole, rerender } = render(
      <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(getByRole("tab", { name: "YAML" }).getAttribute("aria-selected")).toBe("true"));

    getObject.mockResolvedValueOnce({ object: POD_2 });
    rerender(<ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-2" />);

    await waitFor(() => expect(getByRole("heading").textContent).toBe("web-2"));
    // Still on YAML — comparing YAML (or scanning Events) across several rows
    // is a normal workflow, and every row click must not throw the reader
    // back to Details.
    expect(getByRole("tab", { name: "YAML" }).getAttribute("aria-selected")).toBe("true");
  });

  it("falls back to Details when the newly selected subject's kind doesn't offer the previously selected pane", async () => {
    descriptorFor.mockImplementation((slug: string) =>
      slug === "pods" ? baseDescriptor({ panes: { containers: true } }) : undefined,
    );
    getObject.mockResolvedValueOnce({ object: POD });
    const { getByRole, queryByRole, rerender } = render(
      <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="web-1" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "Containers" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "Containers" }));
    await waitFor(() =>
      expect(getByRole("tab", { name: "Containers" }).getAttribute("aria-selected")).toBe("true"),
    );

    // A ConfigMap's descriptor offers no Containers pane — the guard that
    // already exists for "this kind doesn't have the selected pane" is what
    // must catch this, not a reset that also clobbers the persist case above.
    getObject.mockResolvedValueOnce({ object: CONFIGMAP });
    rerender(<ResourceDetailView context="ctx" kind="ConfigMap" namespace="default" name="cm-1" />);

    await waitFor(() => expect(getByRole("heading").textContent).toBe("cm-1"));
    expect(queryByRole("tab", { name: "Containers" })).toBeNull();
    expect(getByRole("tab", { name: "Details" }).getAttribute("aria-selected")).toBe("true");
  });

  it("never commits a frame that pairs the new subject's heading with the previous subject's content", async () => {
    // Settled-state assertions (as in the test above) cannot see this: RTL's
    // act() flushes passive effects synchronously, so by the time `await
    // waitFor(...)` resolves, any transient bad frame already happened and
    // was overwritten. A real browser has no such luxury — it paints
    // whatever was committed. This test records every committed frame with
    // a `useLayoutEffect` probe (which, like a browser's paint, runs
    // synchronously after each commit, before the next one) and asserts
    // none of them pairs one subject's heading with the other's content.
    getObject.mockResolvedValueOnce({ object: POD });
    getManifest.mockResolvedValueOnce({ yaml: "kind: Pod\nmetadata:\n  name: web-1\n" });

    const frames: Array<{ heading: string | null; content: string | null }> = [];

    function FrameProbe() {
      useLayoutEffect(() => {
        frames.push({
          heading: document.querySelector("h2")?.textContent ?? null,
          content: document.querySelector(".cm-content")?.textContent ?? null,
        });
      });
      return null;
    }

    function Harness(props: { namespace: string | null; name: string }) {
      return (
        <>
          <ResourceDetailView context="ctx" kind="Pod" {...props} />
          <FrameProbe />
        </>
      );
    }

    const { rerender } = render(<Harness namespace="default" name="web-1" />);
    await waitFor(() => expect(screen.getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(screen.getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(document.querySelector(".cm-content")?.textContent).toContain("web-1"));

    // Only frames from the subject change itself are under test.
    frames.length = 0;

    getObject.mockResolvedValueOnce({ object: POD_2 });
    getManifest.mockResolvedValueOnce({ yaml: "kind: Pod\nmetadata:\n  name: web-2\n" });
    rerender(<Harness namespace="default" name="web-2" />);

    await waitFor(() => expect(document.querySelector(".cm-content")?.textContent).toContain("web-2"));

    const mismatched = frames.filter((f) => {
      if (!f.heading || !f.content) return false;
      const other = f.heading === "web-1" ? "web-2" : "web-1";
      return f.content.includes(`name: ${other}`) && !f.content.includes(`name: ${f.heading}`);
    });
    expect(mismatched).toEqual([]);
  });

  /**
   * R-5 IS RETIRED, and so is the last of the tests that held the two screens
   * in step by comparing their rendered trees.
   *
   * The rule said the peek and the full tab were one pane with one set of
   * props; the user's full-tab mock says otherwise, and the user's own ruling
   * is that they are two screens with two designs. What replaced the rule is
   * asserted here — the discipline underneath it: one read of the subject, one
   * lazy-load rule, one derivation of the facts. So the two can differ in how
   * a fact reads and cannot differ in what it says.
   *
   * WHERE EACH HALF IS ASSERTED, which is the whole point of the split:
   *
   * - that the two get the SAME facts is asserted at the DATA layer
   *   (`detailFacts`), not by rendering both screens and comparing labels. A
   *   comparison of two trees is a comparison of two layouts, and the two
   *   layouts are meant to differ now.
   * - that each screen draws its own mock's layout is asserted in that
   *   screen's own tests — "the facts, the peek's way" below, and
   *   `ResourceTabView.test`'s "lays the facts out as three columns".
   */
  describe("what the two hosts still share, now that they are two screens", () => {
    const props = { context: "ctx", kind: "Pod", namespace: "checkout", name: "cart-session-store-0" } as const;

    it("reads the subject once in either host, through the same call", async () => {
      getObject.mockResolvedValue({ object: RUNNING_POD });
      descriptorFor.mockReturnValue(baseDescriptor({ panes: { containers: true } }));

      const asPeek = render(<ResourceDetailView {...props} peek={{ onClose: vi.fn(), onOpenTab: vi.fn() }} />);
      await waitFor(() => expect(asPeek.getByRole("tab", { name: "Details" })).toBeDefined());
      expect(getObject).toHaveBeenCalledTimes(1);
      const fromPeek = getObject.mock.calls[0];
      asPeek.unmount();

      getObject.mockClear();
      const asTab = render(<ResourceTabView {...props} />);
      await waitFor(() => expect(asTab.getByRole("tab", { name: "Overview" })).toBeDefined());
      expect(getObject).toHaveBeenCalledTimes(1);
      expect(getObject.mock.calls[0]).toEqual(fromPeek);
    });

    it("derives one fact list for one subject, whichever screen asks for it", async () => {
      // THE SHARED LAYER ITSELF, not two rendered trees compared. `detailFacts`
      // is the only place a subject's facts are derived; both screens call it
      // and neither can reach the other's layout. A fact fixed in one is
      // therefore fixed in both by construction — the property the deleted
      // tree comparison was standing in for, asserted where it actually lives.
      const facts = detailFacts({ kind: "Pod", object: RUNNING_POD });
      expect(facts.map((f) => f.label)).toContain("Status");
      expect(facts.map((f) => f.label)).toContain("Containers ready");
      // Pure and stable: two readings of one subject are the same list, so
      // two screens rendering it cannot drift apart between them.
      expect(detailFacts({ kind: "Pod", object: RUNNING_POD }).map((f) => f.label)).toEqual(
        facts.map((f) => f.label),
      );
      // And it is a kind's OWN list, dispatched on the route's kind: a
      // ConfigMap gets the identity facts, not a pod's.
      expect(detailFacts({ kind: "ConfigMap", object: AGED_CONFIGMAP }).map((f) => f.label)).toEqual([
        "Namespace",
        "Created",
      ]);
    });

    it("draws the WHOLE list in each screen, in that screen's own layout and no other's", async () => {
      getObject.mockResolvedValue({ object: RUNNING_POD });
      descriptorFor.mockReturnValue(baseDescriptor({ panes: { containers: true } }));

      // EQUALS, not contains. A screen that quietly dropped three of the
      // facts it was handed would look right in every other assertion here,
      // and the drift between the two screens would be exactly what the
      // retired both-hosts comparison used to catch. Each screen is held to
      // the derived list instead — which catches the same drift without
      // either screen's markup standing in for the other's.
      const derived = detailFacts({ kind: "Pod", object: RUNNING_POD }).map((f) => f.label);
      expect(derived.length).toBeGreaterThan(3);

      const asPeek = render(<ResourceDetailView {...props} peek={{ onClose: vi.fn(), onOpenTab: vi.fn() }} />);
      await waitFor(() => expect(asPeek.getByRole("tab", { name: "Details" })).toBeDefined());
      // The peek's own form: a label column beside a value column, and no
      // grid — the mock's two-column list, in a 352px pane.
      expect(document.querySelectorAll(".kv[data-stacked]")).toHaveLength(0);
      expect(document.querySelector("[data-slot='fact-grid']")).toBeNull();
      expect(labelsIn(".fact-list")).toEqual(derived);
      asPeek.unmount();

      const asTab = render(<ResourceTabView {...props} />);
      await waitFor(() => expect(asTab.getByRole("tab", { name: "Overview" })).toBeDefined());
      // The tab's own: three columns of label-above-value, built by the tab.
      const grid = document.querySelector<HTMLElement>("[data-slot='fact-grid']");
      expect(grid).not.toBeNull();
      expect(grid!.querySelectorAll(".kv[data-stacked='true']")).toHaveLength(derived.length);
      expect(labelsIn("[data-slot='fact-grid']")).toEqual(derived);
    });

    it("neither refetches a pane it has already opened, and neither fetches one it has not", async () => {
      getObject.mockResolvedValue({ object: RUNNING_POD });
      descriptorFor.mockReturnValue(baseDescriptor({ panes: { containers: true } }));

      const asTab = render(<ResourceTabView {...props} />);
      await waitFor(() => expect(asTab.getByRole("tab", { name: "Overview" })).toBeDefined());
      // Lazy: nothing behind YAML or Events has been asked for yet.
      expect(getManifest).not.toHaveBeenCalled();
      expect(listEvents).not.toHaveBeenCalled();

      await userEvent.click(asTab.getByRole("tab", { name: "YAML" }));
      await waitFor(() => expect(getManifest).toHaveBeenCalledTimes(1));
      await userEvent.click(asTab.getByRole("tab", { name: "Overview" }));
      await userEvent.click(asTab.getByRole("tab", { name: "YAML" }));
      // Coming back is not a new subject.
      expect(getManifest).toHaveBeenCalledTimes(1);
    });

    it("is the peek alone that offers the peek's own two controls", async () => {
      getObject.mockResolvedValue({ object: RUNNING_POD });
      descriptorFor.mockReturnValue(baseDescriptor());

      const asPeek = render(<ResourceDetailView {...props} peek={{ onClose: vi.fn(), onOpenTab: vi.fn() }} />);
      await waitFor(() => expect(asPeek.getByRole("tab", { name: "Details" })).toBeDefined());
      expect(asPeek.getByRole("button", { name: "Close inspector" })).toBeDefined();
      expect(asPeek.getByRole("button", { name: "Open tab" })).toBeDefined();
      asPeek.unmount();

      // The tab IS the tab: closing it is the window strip's job, and an
      // "Open tab" there would open a second copy of what you are reading.
      const asTab = render(<ResourceTabView {...props} />);
      await waitFor(() => expect(asTab.getByRole("tab", { name: "Overview" })).toBeDefined());
      expect(asTab.queryByRole("button", { name: "Close inspector" })).toBeNull();
      expect(asTab.queryByRole("button", { name: "Open tab" })).toBeNull();
    });
  });

  /**
   * Labels and Annotations, and the gate that keeps a Secret's annotation out
   * of the document.
   *
   * These were asserted three times over — through `GenericBody`, `PodBody`
   * and `WorkloadBody` — because all three rendered their own copy of the
   * blocks. Two of those copies had no `Secret` branch at all, and were safe
   * only because the kinds they served happened not to include Secret: a
   * security gate resting on a membership list two files away. The blocks now
   * belong to the HOST, so they are asserted once, here, through the pane a
   * reader actually sees.
   */
  describe("Labels and Annotations, which the host places", () => {
    // Obviously-fake fixture text — never anything that reads as a real
    // manifest or credential.
    const FIXTURE_VALUE = "fixture-only-not-a-real-last-applied-manifest";
    const APPLIED = "kubectl.kubernetes.io/last-applied-configuration";

    const withMeta = (kind: string, meta: Record<string, unknown>): K8sObject => ({
      kind,
      apiVersion: "v1",
      metadata: { name: "subject-1", namespace: "default", ...meta },
    });

    async function open(kind: string, meta: Record<string, unknown>) {
      getObject.mockResolvedValue({ object: withMeta(kind, meta) });
      const view = render(<ResourceDetailView context="ctx" kind={kind} namespace="default" name="subject-1" />);
      await waitFor(() => expect(view.getByRole("tab", { name: "Details" })).toBeDefined());
      return view;
    }

    it("gives each its own headed block of full-width key=value lines", async () => {
      const { container } = await open("ConfigMap", {
        labels: { app: "checkout" },
        annotations: { "srelens.io/note": "hello" },
      });
      expect(screen.getByRole("heading", { level: 3, name: "Labels" })).toBeDefined();
      expect(screen.getByRole("heading", { level: 3, name: "Annotations" })).toBeDefined();
      await expand("Labels");
      expect(screen.getByText("app=")).toBeDefined();
      expect(screen.getByText("checkout")).toBeDefined();
      // Not truncated: `PairList` writes no `title`, so wrapping is the only
      // way a long value can be read at all.
      expect(container.querySelector("li.truncate")).toBeNull();
      expect(container.querySelector(".pairs .v.break-all")).not.toBeNull();
    });

    it("draws neither block for an object carrying neither", async () => {
      await open("ConfigMap", {});
      expect(screen.queryByRole("heading", { level: 3, name: "Labels" })).toBeNull();
      expect(screen.queryByRole("heading", { level: 3, name: "Annotations" })).toBeNull();
    });

    it("shows an ordinary kind's annotations outright once the block is opened, with no second toggle", async () => {
      // The design draws annotations open and ungated. The block itself folds
      // now, which is a preference about how much of a pane a reader wants at
      // once; what is NOT here is the `Show N annotations` gate, which is a
      // security control and belongs to Secret alone.
      await open("ConfigMap", { annotations: { "srelens.io/last-applied-by": "dana@acme.io" } });
      await expand("Annotations");
      expect(documentContains("dana@acme.io")).toBe(true);
      expect(screen.queryByRole("button", { name: /^Show / })).toBeNull();
    });

    it("keeps a Secret's annotation value out of the document until asked, then hides it again", async () => {
      // A `kubectl apply`-managed Secret: `last-applied-configuration` holds
      // the ENTIRE applied manifest, base64 `data` map and all, and
      // `k8s.getObject`'s Secret redaction never touches
      // `metadata.annotations`.
      await open("Secret", { annotations: { [APPLIED]: FIXTURE_VALUE } });

      // Not as text, not as a title/aria-label/data-*, not anywhere in the
      // markup — nothing under the toggle is mounted at all.
      expect(documentContains(FIXTURE_VALUE)).toBe(false);

      await expand("Annotations");
      // Opening the BLOCK reveals the gate, never what is behind it.
      expect(documentContains(FIXTURE_VALUE)).toBe(false);

      const toggle = screen.getByRole("button", { name: "Show 1 annotation" });
      expect(toggle.getAttribute("title")).toBeNull();
      expect(toggle.getAttribute("aria-label")).toBeNull();

      await userEvent.click(toggle);
      await waitFor(() => expect(documentContains(FIXTURE_VALUE)).toBe(true));

      await userEvent.click(screen.getByRole("button", { name: "Hide" }));
      expect(documentContains(FIXTURE_VALUE)).toBe(false);
    });

    it("gates Secret and nothing else", async () => {
      // The two rules must not be confused for one. `AnnotationLines` holds
      // the applied manifest back on EVERY kind, for legibility; the gate is a
      // different rule and applies to Secret alone. Asserted on an ordinary
      // annotation, so the legibility rule cannot stand in for the gate.
      await open("Pod", { annotations: { "srelens.io/last-applied-by": "dana@acme.io" } });
      await expand("Annotations");
      expect(documentContains("dana@acme.io")).toBe(true);
      expect(screen.queryByRole("button", { name: /^Show / })).toBeNull();
    });

    it("withholds an ordinary kind's applied manifest for length, and says where to read it", async () => {
      await open("ConfigMap", {
        annotations: { [APPLIED]: FIXTURE_VALUE, "srelens.io/last-applied-by": "dana@acme.io" },
      });
      await expand("Annotations");
      expect(documentContains(FIXTURE_VALUE)).toBe(false);
      expect(screen.getByText(new RegExp(APPLIED.replace(/\//g, "\\/"))).textContent).toMatch(/YAML/);
      expect(documentContains("dana@acme.io")).toBe(true);
    });
  });

  /**
   * "By default everything is uncollapsed, first open should keep everything
   * collapsed, and from for next one remember what all was uncollapsed" — the
   * reader's own words. The store is `lib/sectionFolds.ts` and its own tests
   * cover the document; these are about the pane a reader actually sees.
   */
  describe("the blocks a reader has opened", () => {
    const withMeta = (kind: string, meta: Record<string, unknown>): K8sObject => ({
      kind,
      apiVersion: "v1",
      metadata: { name: "subject-1", namespace: "default", ...meta },
    });

    async function open(kind: string, meta: Record<string, unknown> = {}) {
      getObject.mockResolvedValue({ object: withMeta(kind, meta) });
      const view = render(<ResourceDetailView context="ctx" kind={kind} namespace="default" name="subject-1" />);
      await waitFor(() => expect(view.getByRole("tab", { name: "Details" })).toBeDefined());
      return view;
    }

    /**
     * THE RULING, at the pane a reader actually sees: a titled section that is
     * the only content of its pane defaults to open. Everything else opens
     * shut — "first open should keep everything collapsed" — but a pane whose
     * whole substance is one block would otherwise open completely empty,
     * which is the very hostility the unheaded lead fact list was exempted
     * for.
     */
    describe("a pane whose whole content is one block", () => {
      it("opens the Containers pane showing the containers", async () => {
        descriptorFor.mockReturnValue(baseDescriptor({ panes: { containers: true } }));
        getObject.mockResolvedValue({
          object: {
            kind: "Pod",
            apiVersion: "v1",
            metadata: { name: "subject-1", namespace: "default" },
            spec: { containers: [{ name: "app", image: "redis:7.4-alpine" }] },
          } as K8sObject,
        });
        const view = render(
          <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="subject-1" />,
        );
        await waitFor(() => expect(view.getByRole("tab", { name: "Containers" })).toBeDefined());
        await userEvent.click(view.getByRole("tab", { name: "Containers" }));
        expect(screen.getByRole("button", { name: "Containers" }).getAttribute("aria-expanded")).toBe("true");
        expect(screen.getByText("redis:7.4-alpine")).toBeDefined();
      });

      it("opens it for a pod with init containers too, so one kind means one default", async () => {
        // The default may NOT vary with the subject. The memory is keyed per
        // KIND: a reader who opens Containers on a pod with init containers
        // stores a marker, shuts it again and the marker is dropped — and the
        // next pod without init containers would then show it open. What the
        // document means would depend on which pod happened to be on screen
        // when they clicked. The main group is the pane's subject either way,
        // so it always leads open and the init group keeps the shut rule.
        descriptorFor.mockReturnValue(baseDescriptor({ panes: { containers: true } }));
        getObject.mockResolvedValue({
          object: {
            kind: "Pod",
            apiVersion: "v1",
            metadata: { name: "subject-1", namespace: "default" },
            spec: {
              initContainers: [{ name: "migrate", image: "ghcr.io/example/migrate:2" }],
              containers: [{ name: "app", image: "redis:7.4-alpine" }],
            },
          } as K8sObject,
        });
        const view = render(
          <ResourceDetailView context="ctx" kind="Pod" namespace="default" name="subject-1" />,
        );
        await waitFor(() => expect(view.getByRole("tab", { name: "Containers" })).toBeDefined());
        await userEvent.click(view.getByRole("tab", { name: "Containers" }));
        expect(screen.getByRole("button", { name: "Containers" }).getAttribute("aria-expanded")).toBe("true");
        expect(screen.getByText("redis:7.4-alpine")).toBeDefined();
        // And nothing else opened with it.
        expect(screen.getByRole("button", { name: "Init containers" }).getAttribute("aria-expanded")).toBe(
          "false",
        );
        expect(screen.queryByText("ghcr.io/example/migrate:2")).toBeNull();
      });

      it("opens a ConfigMap's Data, the only titled block its pane offers", async () => {
        getObject.mockResolvedValue({
          object: {
            kind: "ConfigMap",
            apiVersion: "v1",
            metadata: { name: "subject-1", namespace: "default" },
            data: { "app.conf": "level=info" },
          } as K8sObject,
        });
        const view = render(
          <ResourceDetailView context="ctx" kind="ConfigMap" namespace="default" name="subject-1" />,
        );
        await waitFor(() => expect(view.getByRole("tab", { name: "Details" })).toBeDefined());
        expect(screen.getByText("level=info")).toBeDefined();
      });

      it("leaves a Secret's Data shut, whose pane says more than the values", async () => {
        // Not the same shape, and the safer answer where the difference is
        // arguable: a Secret's pane heads its data with a summary block, so
        // it never opens empty, and less disclosed by default is the right
        // way to be wrong about a Secret.
        getObject.mockResolvedValue({
          object: {
            kind: "Secret",
            apiVersion: "v1",
            metadata: { name: "subject-1", namespace: "default" },
            type: "Opaque",
            data: { token: FIXTURE_B64 },
          } as K8sObject,
        });
        const view = render(
          <ResourceDetailView context="ctx" kind="Secret" namespace="default" name="subject-1" />,
        );
        await waitFor(() => expect(view.getByRole("tab", { name: "Details" })).toBeDefined());
        expect(screen.getByRole("button", { name: /^Data/ }).getAttribute("aria-expanded")).toBe("false");
        expect(screen.queryByRole("button", { name: "Reveal" })).toBeNull();
      });
    });

    it("opens every titled block shut on a first visit, and the unheaded lead block open", async () => {
      await open("ConfigMap", {
        creationTimestamp: "2026-08-20T00:00:00Z",
        labels: { app: "checkout" },
        annotations: { "srelens.io/note": "hello" },
      });
      for (const name of ["Labels", "Annotations"]) {
        expect(screen.getByRole("button", { name }).getAttribute("aria-expanded")).toBe("false");
      }
      // The lead fact list has no heading, so there is nothing to hang a
      // control on — and a pane that opens showing nothing at all is hostile.
      expect(screen.getByText("Namespace")).toBeDefined();
      expect(screen.getByText("default")).toBeDefined();
    });

    it("remembers what the reader opened, for the next subject of that kind", async () => {
      const first = await open("ConfigMap", { labels: { app: "checkout" } });
      await expand("Labels");
      expect(screen.getByText("checkout")).toBeDefined();
      first.unmount();

      await open("ConfigMap", { labels: { app: "billing" } });
      expect(screen.getByRole("button", { name: "Labels" }).getAttribute("aria-expanded")).toBe("true");
      expect(screen.getByText("billing")).toBeDefined();
    });

    it("remembers per kind, so opening a block on one kind opens nothing on another", async () => {
      const first = await open("ConfigMap", { labels: { app: "checkout" } });
      await expand("Labels");
      first.unmount();

      await open("Pod", { labels: { app: "checkout" } });
      expect(screen.getByRole("button", { name: "Labels" }).getAttribute("aria-expanded")).toBe("false");
    });

    it("leaves a Secret's annotations shut whatever any other kind's memory says", async () => {
      // THE GATE, from the direction that would break it. `AnnotationsToggle`
      // is a security control — a `kubectl apply`-managed Secret carries its
      // whole base64 `data` map inside the applied-configuration annotation,
      // and `k8s.getObject`'s redaction never touches annotations. A memory
      // recorded on a Deployment is a different key and cannot reach it.
      setSectionOpen("Deployment", "Annotations", true);
      await open("Secret", { annotations: { "srelens.io/note": "fixture-only" } });
      expect(screen.getByRole("button", { name: "Annotations" }).getAttribute("aria-expanded")).toBe("false");
      expect(documentContains("fixture-only")).toBe(false);
    });

    it("still gates a Secret's values behind the reader's own reveal when the block itself is remembered open", async () => {
      // The memory can disclose the GATE and never what is behind it. The
      // toggle keeps its own state, starts shut on every mount, and reads
      // nothing from this store — so the only thing a remembered "open" can
      // show is the words on its button.
      setSectionOpen("Secret", "Annotations", true);
      await open("Secret", { annotations: { "srelens.io/note": "fixture-only" } });
      expect(screen.getByRole("button", { name: "Annotations" }).getAttribute("aria-expanded")).toBe("true");
      expect(documentContains("fixture-only")).toBe(false);
      await userEvent.click(screen.getByRole("button", { name: "Show 1 annotation" }));
      await waitFor(() => expect(documentContains("fixture-only")).toBe(true));
    });

    it("keeps the run of blocks unbroken, so a shut block still draws its rule", async () => {
      // `.section + .section` is what divides a detail. A shut block is still
      // a section, and the memory adds no element between any two of them.
      const { container } = await open("ConfigMap", {
        creationTimestamp: "2026-08-20T00:00:00Z",
        labels: { app: "checkout" },
        annotations: { "srelens.io/note": "hello" },
      });
      const body = container.querySelector(".pane-body")!;
      expect(body.children.length).toBeGreaterThan(1);
      expect([...body.children].every((el) => el.matches("section.section"))).toBe(true);
    });
  });

  describe("the Secret YAML pane's redaction", () => {
    // The Details pane gates a Secret's values behind an explicit reveal.
    // The YAML pane sits one tab over and, left alone, hands the very same
    // values over with no gate at all — `k8s.getManifest` does not redact
    // (only `k8s.getObject` does). This is a deliberate divergence from
    // classic, which shows the manifest unredacted.
    it("keeps a Secret's values out of the document entirely", async () => {
      getObject.mockResolvedValue({ object: SECRET });
      getManifest.mockResolvedValue({
        yaml: `apiVersion: v1\nkind: Secret\nmetadata:\n  name: s-1\ndata:\n  token: ${FIXTURE_B64}\n`,
      });
      const { getByRole, container } = render(
        <ResourceDetailView context="ctx" kind="Secret" namespace="default" name="s-1" />,
      );
      await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
      await userEvent.click(getByRole("tab", { name: "YAML" }));

      // Positive control first, so the absence assertion below cannot pass
      // vacuously on an editor that simply rendered nothing.
      await waitFor(() => expect(container.querySelector(".cm-content")?.textContent).toContain("REDACTED"));
      expect(container.querySelector(".cm-content")?.textContent).toContain("token:");
      expect(documentContains(FIXTURE_B64)).toBe(false);
    });

    it("tells the reader the values are redacted and where to reveal them", async () => {
      getObject.mockResolvedValue({ object: SECRET });
      getManifest.mockResolvedValue({ yaml: `kind: Secret\ndata:\n  token: ${FIXTURE_B64}\n` });
      const { getByRole, container } = render(
        <ResourceDetailView context="ctx" kind="Secret" namespace="default" name="s-1" />,
      );
      await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
      await userEvent.click(getByRole("tab", { name: "YAML" }));
      await waitFor(() => expect(container.querySelector(".cm-content")?.textContent).toContain("REDACTED"));

      // Shown less, and TOLD so — a silently shortened manifest reads as the
      // real one. `Alert` tone "info" is a `status` region, not an `alert`,
      // so it never collides with the pane's own error state.
      const notice = getByRole("status").textContent ?? "";
      expect(notice.toLowerCase()).toContain("redacted");
      expect(notice).toContain("Details");
    });

    it("shows an error, and never the raw manifest, when a Secret's manifest cannot be redacted", async () => {
      getObject.mockResolvedValue({ object: SECRET });
      // Tabs are not legal YAML indentation — the redactor cannot parse this,
      // and must fail closed rather than pass the input through.
      getManifest.mockResolvedValue({ yaml: `kind: Secret\ndata:\n\ttoken: ${FIXTURE_B64}\n` });
      const { getByRole, container } = render(
        <ResourceDetailView context="ctx" kind="Secret" namespace="default" name="s-1" />,
      );
      await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
      await userEvent.click(getByRole("tab", { name: "YAML" }));
      await waitFor(() => expect(getByRole("alert")).toBeDefined());
      expect(documentContains(FIXTURE_B64)).toBe(false);
      expect(container.querySelector(".cm-content")).toBeNull();

      // The redactor's refusals are this package's own careful sentences, and
      // they are careful precisely because no error message here may quote the
      // source it failed on. They are not cluster errors and match no
      // classification, so routing this pane through `describeError` must
      // leave them exactly as written — and, because the pane keeps its own
      // title, the reader is never told "Something went wrong" about them.
      const alert = getByRole("alert");
      expect(alert.textContent).toContain("Could not load Secret default/s-1's manifest");
      expect(alert.textContent).toContain("it could not be parsed");
      expect(alert.textContent).not.toContain("Something went wrong");
      // Nothing was reformatted into a second copy of anything, either.
      expect(alert.querySelector('[data-slot="raw"]')).toBeNull();
    });

    it("leaves a non-Secret kind's manifest untouched, with no redaction notice", async () => {
      getObject.mockResolvedValue({ object: CONFIGMAP });
      getManifest.mockResolvedValue({ yaml: "kind: ConfigMap\ndata:\n  greeting: hello-world\n" });
      const { getByRole, queryByRole, container } = render(
        <ResourceDetailView context="ctx" kind="ConfigMap" namespace="default" name="cm-1" />,
      );
      await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
      await userEvent.click(getByRole("tab", { name: "YAML" }));
      await waitFor(() => expect(container.querySelector(".cm-content")?.textContent).toContain("hello-world"));
      expect(container.querySelector(".cm-content")?.textContent).not.toContain("REDACTED");
      expect(queryByRole("status")).toBeNull();
    });
  });

  it("reports the ambiguity when two CustomResourceDefinitions claim the same kind", async () => {
    // Two groups can legitimately define the same `.kind`. Taking the first
    // match would fetch a manifest from possibly the wrong group and show it
    // as if it were right — a possibly-wrong success, which is worse than a
    // failure.
    getObject.mockResolvedValue({ object: { kind: "Widget", metadata: { name: "w-1", namespace: "default" } } });
    listCrds.mockResolvedValue({
      crds: [
        { name: "widgets.example.com", group: "example.com", version: "v1", kind: "Widget", plural: "widgets", namespaced: true },
        { name: "widgets.other.io", group: "other.io", version: "v1", kind: "Widget", plural: "widgets", namespaced: true },
      ],
    });
    const { getByRole } = render(<ResourceDetailView context="ctx" kind="Widget" namespace="default" name="w-1" />);
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(getByRole("alert")).toBeDefined());
    const text = getByRole("alert").textContent ?? "";
    expect(text).toContain("Widget");
    expect(text).toContain("example.com");
    expect(text).toContain("other.io");
    // Never guessed at: no manifest is fetched from either group.
    expect(getManifest).not.toHaveBeenCalled();
  });

  it("still resolves a kind claimed by exactly one CustomResourceDefinition among several", async () => {
    getObject.mockResolvedValue({ object: { kind: "Widget", metadata: { name: "w-1", namespace: "default" } } });
    listCrds.mockResolvedValue({
      crds: [
        { name: "gadgets.example.com", group: "example.com", version: "v1", kind: "Gadget", plural: "gadgets", namespaced: true },
        { name: "widgets.other.io", group: "other.io", version: "v2", kind: "Widget", plural: "widgets", namespaced: true },
      ],
    });
    const { getByRole } = render(<ResourceDetailView context="ctx" kind="Widget" namespace="default" name="w-1" />);
    await waitFor(() => expect(getByRole("tab", { name: "YAML" })).toBeDefined());
    await userEvent.click(getByRole("tab", { name: "YAML" }));
    await waitFor(() => expect(getManifest).toHaveBeenCalledTimes(1));
    expect(getManifest).toHaveBeenCalledWith("ctx", "Widget", "default", "w-1", undefined, {
      group: "other.io",
      version: "v2",
      plural: "widgets",
    });
  });

  /**
   * The mock's third header line — a toned dot, the state, the ready ratio and
   * the age — and the two affordances at its top right.
   */
  describe("the header the design draws", () => {
    it("reads the state, the ready ratio and the age across one line", async () => {
      getObject.mockResolvedValue({ object: DEGRADED_DEPLOYMENT });
      const { getByText, container } = render(
        <ResourceDetailView context="ctx" kind="Deployment" namespace="checkout" name="checkout-api" />,
      );
      await waitFor(() => expect(getByText("Degraded")).toBeDefined());
      // Bare figures, each carrying its own noun — the user's call, taken over
      // the kit's own objection (see `Inspector`'s doc comment).
      expect(getByText("9/12 ready")).toBeDefined();
      expect(getByText("84d")).toBeDefined();
      expect(container.querySelector("header")?.textContent).not.toContain("Ready 9/12");
    });

    it("names every bare figure for a reader who cannot see it", async () => {
      getObject.mockResolvedValue({ object: DEGRADED_DEPLOYMENT });
      const { container, getByText } = render(
        <ResourceDetailView context="ctx" kind="Deployment" namespace="checkout" name="checkout-api" />,
      );
      await waitFor(() => expect(getByText("Degraded")).toBeDefined());
      // `InspectorFact.label` is never drawn — it is an `sr-only` `dt`. A fact
      // handed a label that merely repeats what the value already says on
      // screen leaves a screen reader with nothing, which is the whole reason
      // the user's bare-figure ruling was survivable.
      const terms = Array.from(container.querySelectorAll("header dt"));
      expect(terms.map((t) => t.textContent)).toEqual(["Progress", "Age"]);
      terms.forEach((t) => expect(t.className).toContain("sr-only"));
    });

    it("draws the age quietly and the ready ratio in normal ink", async () => {
      getObject.mockResolvedValue({ object: DEGRADED_DEPLOYMENT });
      const { getByText } = render(
        <ResourceDetailView context="ctx" kind="Deployment" namespace="checkout" name="checkout-api" />,
      );
      await waitFor(() => expect(getByText("Degraded")).toBeDefined());
      expect(getByText("84d").style.color).toBe(toneColor("muted"));
      // A fact defaults to normal ink; only the age is quiet in the mock.
      expect(getByText("9/12 ready").style.color).toBe("");
    });

    it("colours the state and marks the name only when the subject is unhealthy", async () => {
      getObject.mockResolvedValue({ object: DEGRADED_DEPLOYMENT });
      const bad = render(
        <ResourceDetailView context="ctx" kind="Deployment" namespace="checkout" name="checkout-api" />,
      );
      await waitFor(() => expect(bad.getByText("Degraded")).toBeDefined());
      expect(bad.container.querySelector("header .status")?.getAttribute("data-bad")).toBe("true");
      // The mock's dot before the NAME. Colour alone says nothing to a
      // colour-blind reader and nothing at all to a screen reader, so the kit
      // pairs it with a word only the latter hears.
      expect(bad.getByText("Needs attention")).toBeDefined();
      bad.unmount();

      getObject.mockResolvedValue({ object: RUNNING_POD });
      const good = render(
        <ResourceDetailView context="ctx" kind="Pod" namespace="checkout" name="cart-session-store-0" />,
      );
      // Read off the HEADER's own pill: the Details body below it states the
      // pod's phase as well, so a bare text query finds two "Running"s.
      const headerStatus = () => good.container.querySelector("header .status");
      await waitFor(() => expect(headerStatus()?.textContent).toBe("Running"));
      expect(headerStatus()?.getAttribute("data-bad")).toBeNull();
      expect(good.queryByText("Needs attention")).toBeNull();
      expect(good.getByText("1/1 ready")).toBeDefined();
      expect(good.getByText("211d")).toBeDefined();
    });

    it("draws no status line at all for a kind that has no health of its own", async () => {
      getObject.mockResolvedValue({ object: AGED_CONFIGMAP });
      const { container, getByRole } = render(
        <ResourceDetailView context="ctx" kind="ConfigMap" namespace="default" name="cm-1" />,
      );
      await waitFor(() => expect(getByRole("heading", { name: "cm-1" })).toBeDefined());
      // `resourceStatusLine` returning null is an answer, not a gap: a
      // ConfigMap has no health, and half a line — an age with nothing to
      // qualify it — would read as the rest having gone missing.
      expect(container.querySelector("header .status")).toBeNull();
      expect(container.querySelector("header dl")).toBeNull();
    });
  });

  it("orders the panes the way the design does", async () => {
    getObject.mockResolvedValue({ object: RUNNING_POD });
    descriptorFor.mockReturnValue(baseDescriptor({ panes: { containers: true, metrics: true } }));
    const { getAllByRole, getByRole } = render(
      <ResourceDetailView context="ctx" kind="Pod" namespace="checkout" name="cart-session-store-0" />,
    );
    await waitFor(() => expect(getByRole("tab", { name: "Metrics" })).toBeDefined());
    // `Details Containers YAML Events Metrics`. Metrics is deferred and no
    // kind's descriptor asks for it yet, so this order only bites the day one
    // does — which is exactly when nobody would think to check it.
    expect(getAllByRole("tab").map((t) => t.textContent)).toEqual([
      "Details",
      "Containers",
      "YAML",
      "Events",
      "Metrics",
    ]);
  });

  it("offers Open tab in the peek host only, and leaves the promotion to the host", async () => {
    getObject.mockResolvedValue({ object: POD });
    const onOpenTab = vi.fn();
    const onClose = vi.fn();
    const props = { context: "ctx", kind: "Pod", namespace: "default", name: "web-1" } as const;

    const asPeek = render(<ResourceDetailView {...props} peek={{ onClose, onOpenTab }} />);
    await waitFor(() => expect(asPeek.getByRole("tab", { name: "Details" })).toBeDefined());
    await userEvent.click(asPeek.getByRole("button", { name: "Open tab" }));
    expect(onOpenTab).toHaveBeenCalledTimes(1);
    // Promoting is not dismissing: what the peek does with itself afterwards
    // is the host's business, not the pane's.
    expect(onClose).not.toHaveBeenCalled();
    asPeek.unmount();

    // The tab host IS the tab. An Open tab there would open itself.
    const asTab = render(<ResourceDetailView {...props} />);
    await waitFor(() => expect(asTab.getByRole("tab", { name: "Details" })).toBeDefined());
    expect(asTab.queryByRole("button", { name: "Open tab" })).toBeNull();
  });

  /**
   * The user's report: the YAML editor took the top third of the pane and the
   * manifest stopped around line 28, with a blank white field beneath it.
   *
   * The cause was the kit's `CodeEditor` default. Left to itself it grows with
   * its content up to `maxHeight` (520px), and 520px of 12px type at a 1.55
   * line height is a little under 28 lines — the very place the manifest was
   * cut. Its wrapper's `h-full` did not save it: `height` and `max-height` are
   * different properties, and the cap wins on the used height.
   */
  describe("the YAML pane's height", () => {
    async function openYaml(kind: string, name: string) {
      const view = render(<ResourceDetailView context="ctx" kind={kind} namespace="default" name={name} />);
      await waitFor(() => expect(view.getByRole("tab", { name: "YAML" })).toBeDefined());
      await userEvent.click(view.getByRole("tab", { name: "YAML" }));
      return view;
    }

    it("asks the editor to fill the pane and scroll inside it", async () => {
      getObject.mockResolvedValue({ object: POD });
      const { container } = await openYaml("Pod", "web-1");
      await waitFor(() => expect(container.querySelector(".cm-content")).not.toBeNull());

      expect(codeEditorProps.at(-1)?.fill).toBe(true);
    });

    it("gives that editor a parent with a height to fill", async () => {
      getObject.mockResolvedValue({ object: POD });
      const { container } = await openYaml("Pod", "web-1");
      await waitFor(() => expect(container.querySelector(".cm-content")).not.toBeNull());

      // `fill` resolves to `height: 100%`, which is nothing at all against a
      // parent whose own height is auto. The pane's body is a definite height;
      // this is the chain that carries it down to the editor.
      const host = container.querySelector('[data-slot="yaml-editor"]') as HTMLElement | null;
      expect(host?.className).toContain("h-full");
      const seat = host?.querySelector(".cm-editor")?.parentElement?.parentElement;
      expect(seat?.className).toContain("flex-1");
      expect(seat?.className).toContain("min-h-0");
    });

    it("keeps the Secret redaction notice from taking that height away", async () => {
      getObject.mockResolvedValue({ object: SECRET });
      getManifest.mockResolvedValue({ yaml: `kind: Secret\ndata:\n  token: ${FIXTURE_B64}\n` });
      const { container, getByRole } = await openYaml("Secret", "s-1");
      await waitFor(() => expect(container.querySelector(".cm-content")?.textContent).toContain("REDACTED"));

      // The notice is a sibling above the editor, not a block the editor has
      // to grow under: same slot, same seat, one more row in it.
      expect(getByRole("status")).toBeDefined();
      expect(codeEditorProps.at(-1)?.fill).toBe(true);
      const host = container.querySelector('[data-slot="yaml-editor"]') as HTMLElement | null;
      expect(host?.className).toContain("h-full");
      const seat = host?.querySelector(".cm-editor")?.parentElement?.parentElement;
      expect(seat?.className).toContain("flex-1");
      expect(seat?.className).toContain("min-h-0");
    });
  });

  /**
   * The design's footer: a wide `Ask`, the kind's own two actions, and an
   * overflow. The middle pair varies by kind and comes off `KindActions` — no
   * branch on a kind's name lives here — while `Ask` and the overflow are the
   * pane's own shape.
   */
  describe("the footer action bar", () => {
    const podDescriptor = () => baseDescriptor({ actions: { logs: true, shell: true, forward: true, evict: true } });
    const deploymentDescriptor = () =>
      baseDescriptor({ k8sKind: "Deployment", actions: { logs: true, scale: true, restart: true } });

    /** `Inspector` puts it last inside the pane, and nowhere else. */
    const footer = () => document.querySelector("section.pane > footer") as HTMLElement | null;
    const barWords = () => Array.from(footer()?.querySelectorAll("button") ?? []).map((b) => b.textContent);

    it("puts Logs and Edit on a Deployment's bar, behind Ask and before the overflow", async () => {
      getObject.mockResolvedValue({ object: DEGRADED_DEPLOYMENT });
      descriptorFor.mockReturnValue(deploymentDescriptor());
      const { getByRole } = render(
        <ResourceDetailView context="ctx" kind="Deployment" namespace="checkout" name="checkout-api" />,
      );
      await waitFor(() => expect(getByRole("tab", { name: "Details" })).toBeDefined());

      expect(barWords()).toEqual(["Ask", "Logs", "Edit", "More actions"]);
    });

    it("swaps that Edit for Shell on a Pod, off the descriptor rather than the kind's name", async () => {
      getObject.mockResolvedValue({ object: RUNNING_POD });
      descriptorFor.mockReturnValue(podDescriptor());
      const { getByRole } = render(
        <ResourceDetailView context="ctx" kind="Pod" namespace="checkout" name="cart-session-store-0" />,
      );
      await waitFor(() => expect(getByRole("tab", { name: "Details" })).toBeDefined());

      expect(barWords()).toEqual(["Ask", "Logs", "Shell", "More actions"]);
    });

    it("asks the console about this very subject, and asks WHY when it is unhealthy", async () => {
      getObject.mockResolvedValue({ object: DEGRADED_DEPLOYMENT });
      descriptorFor.mockReturnValue(deploymentDescriptor());
      const { getByRole } = render(
        <ResourceDetailView context="ctx" kind="Deployment" namespace="checkout" name="checkout-api" />,
      );
      await waitFor(() => expect(getByRole("tab", { name: "Details" })).toBeDefined());
      await userEvent.click(within(footer()!).getByRole("button", { name: /^Ask/ }));

      // The same phrasing a list row's chip sends — one question per gesture,
      // not two spellings of it.
      expect(asked).toEqual(["Why is checkout-api unhealthy?"]);
    });

    it("asks the other question of a healthy subject", async () => {
      getObject.mockResolvedValue({ object: RUNNING_POD });
      descriptorFor.mockReturnValue(podDescriptor());
      const { getByRole } = render(
        <ResourceDetailView context="ctx" kind="Pod" namespace="checkout" name="cart-session-store-0" />,
      );
      await waitFor(() => expect(getByRole("tab", { name: "Details" })).toBeDefined());
      await userEvent.click(within(footer()!).getByRole("button", { name: /^Ask/ }));

      expect(asked).toEqual(["What is cart-session-store-0 using right now?"]);
    });

    it("folds the rest behind the overflow, and confirms a destructive one before it runs", async () => {
      getObject.mockResolvedValue({ object: DEGRADED_DEPLOYMENT });
      descriptorFor.mockReturnValue(deploymentDescriptor());
      const { getByRole } = render(
        <ResourceDetailView context="ctx" kind="Deployment" namespace="checkout" name="checkout-api" />,
      );
      await waitFor(() => expect(getByRole("tab", { name: "Details" })).toBeDefined());
      await userEvent.click(getByRole("button", { name: "More actions" }));

      const menu = await screen.findByRole("dialog");
      expect(Array.from(menu.querySelectorAll("button")).map((b) => b.textContent)).toEqual([
        "Copy as kubectl",
        "Scale",
        "Restart rollout",
        "Delete",
      ]);
      // Marked destructive, not merely present: the same tone the row menu
      // gives the same entries.
      const del = within(menu).getByRole("button", { name: "Delete" });
      expect((del as HTMLElement).style.color).toBe(toneColor("sev"));

      // And it takes a confirm. The pane has to RENDER `useRowMenu`'s dialog,
      // not just its items — a footer wired to the items alone offers Delete
      // and then does nothing at all.
      await userEvent.click(del);
      expect(await screen.findByRole("heading", { name: "Delete Deployment?" })).toBeDefined();
      expect(deleteResource).not.toHaveBeenCalled();
    });

    it("withholds Delete from a custom resource, whose GVK the backend cannot resolve", async () => {
      getObject.mockResolvedValue({ object: { kind: "Widget", metadata: { name: "w-1", namespace: "default" } } });
      // No descriptor is exactly what a kind outside `K8S_KIND` gets, and it is
      // the same verdict `customDescriptor` reaches: Delete would always fail,
      // and an action that cannot work is worse than an absent one.
      descriptorFor.mockReturnValue(undefined);
      const { getByRole, queryByRole } = render(
        <ResourceDetailView context="ctx" kind="Widget" namespace="default" name="w-1" />,
      );
      await waitFor(() => expect(getByRole("tab", { name: "Details" })).toBeDefined());

      expect(barWords()).toEqual(["Ask", "Edit", "Copy as kubectl"]);
      // Nothing was left over to fold, so there is no overflow to open onto an
      // empty menu.
      expect(queryByRole("button", { name: "More actions" })).toBeNull();
    });

    it("offers the same actions in the tab's header row, only more of them on the bar", async () => {
      getObject.mockResolvedValue({ object: RUNNING_POD });
      descriptorFor.mockReturnValue(podDescriptor());
      const props = { context: "ctx", kind: "Pod", namespace: "checkout", name: "cart-session-store-0" } as const;

      const asPeek = render(<ResourceDetailView {...props} peek={{ onClose: vi.fn(), onOpenTab: vi.fn() }} />);
      await waitFor(() => expect(asPeek.getByRole("tab", { name: "Details" })).toBeDefined());
      expect(barWords()).toEqual(["Ask", "Logs", "Shell", "More actions"]);
      asPeek.unmount();

      // The design puts this row in a footer in the peek and in the header in
      // the tab, and gives the wider surface four of the kind's own actions
      // instead of two. The ACTIONS are the row menu's in both — the placement
      // and the count are the only things the host decides.
      const asTab = render(<ResourceTabView {...props} />);
      await waitFor(() => expect(asTab.getByRole("tab", { name: "Overview" })).toBeDefined());
      const header = document.querySelector("[data-slot='tab-actions']") as HTMLElement;
      expect(Array.from(header.querySelectorAll("button")).map((b) => b.textContent)).toEqual([
        "Ask",
        "Logs",
        "Shell",
        "Forward",
        "Edit",
        "More actions",
      ]);
      expect(document.querySelector("section.pane > footer")).toBeNull();
    });

    /**
     * `DetailFooter` reconstitutes a `ListRow` for `useRowMenu`, and
     * `suspended: object.spec?.suspend === true` is the one field on it that
     * is not identity. It decides whether a CronJob's action reads Suspend or
     * Resume — and dropping it read "Suspend" on an already-suspended CronJob,
     * which is an action that does nothing, offered as though it did.
     *
     * The whole-branch review mutated it to `suspended={false}` and all 750
     * tests stayed green: the adaptation was correct and entirely unpinned.
     * Both directions are asserted here, because a single unsuspended fixture
     * cannot catch the field being dropped — `isSuspended` reads a missing
     * field exactly like `false`. (#331)
     */
    const cronJobDescriptor = () =>
      baseDescriptor({ k8sKind: "CronJob", actions: { suspend: true, trigger: true } });

    const cronJob = (suspend: boolean): K8sObject => ({
      kind: "CronJob",
      apiVersion: "batch/v1",
      metadata: { name: "nightly-backup", namespace: "batch", creationTimestamp: daysAgo(120) },
      spec: { schedule: "0 2 * * *", suspend },
      status: {},
    });

    /** Every action the footer offers: the two on the bar plus whatever the
     *  overflow holds. Suspend/Resume is the third entry for a CronJob, so a
     *  reader has to open the overflow to reach it. */
    async function allFooterActions(): Promise<(string | null)[]> {
      const bar = barWords();
      await userEvent.click(within(footer()!).getByRole("button", { name: "More actions" }));
      const menu = await screen.findByRole("dialog");
      const folded = Array.from(menu.querySelectorAll("button")).map((b) => b.textContent);
      await userEvent.keyboard("{Escape}");
      return [...bar, ...folded];
    }

    async function footerActionsFor(suspend: boolean): Promise<(string | null)[]> {
      getObject.mockResolvedValue({ object: cronJob(suspend) });
      descriptorFor.mockReturnValue(cronJobDescriptor());
      const view = render(<ResourceDetailView context="ctx" kind="CronJob" namespace="batch" name="nightly-backup" />);
      await waitFor(() => expect(view.getByRole("tab", { name: "Details" })).toBeDefined());
      const words = await allFooterActions();
      view.unmount();
      return words;
    }

    it("offers Suspend on a running CronJob and Resume on a suspended one — the pane's own spec, not a default", async () => {
      const running = await footerActionsFor(false);
      expect(running).toContain("Suspend");
      expect(running).not.toContain("Resume");

      const suspended = await footerActionsFor(true);
      expect(suspended).toContain("Resume");
      expect(suspended).not.toContain("Suspend");
    });
  });
});
