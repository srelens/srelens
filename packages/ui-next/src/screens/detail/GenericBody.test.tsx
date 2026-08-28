import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import type { K8sObject, PodSummary, PodMetric } from "@srelens/core";

// `GenericBody`'s "Pods" section reads live pods/metrics for a kind's
// related-pod selector via core's `podsForSelector`/`podMetrics` — mocked
// here so a test controls what "the cluster said" without one.
// `importOriginal` keeps every formatter (`relatedPodSelector`, `str`,
// `conditionKind`, ...) intact.
const { podsForSelector, podMetrics, listReplicaSets, listEndpointSlices, listJobs, getSecret } = vi.hoisted(
  () => ({
    podsForSelector: vi.fn(async (): Promise<{ pods?: PodSummary[]; error?: string }> => ({ pods: [] })),
    podMetrics: vi.fn(async (): Promise<{ metrics?: PodMetric[]; error?: string }> => ({ metrics: [] })),
    // The four other live reads the real `DETAILS_BODY` entries make, stubbed
    // so the sweep at the bottom of this file can mount every one of them.
    listReplicaSets: vi.fn(async () => ({ replicasets: [] })),
    listEndpointSlices: vi.fn(async () => ({ endpointslices: [] })),
    listJobs: vi.fn(async () => ({ jobs: [] })),
    getSecret: vi.fn(async () => ({ data: {} })),
  }),
);

vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  podsForSelector,
  podMetrics,
  listReplicaSets,
  listEndpointSlices,
  listJobs,
  getSecret,
}));

import userEvent from "@testing-library/user-event";
import { KV, Section } from "@srelens/ui-kit";
import { Section as DetailSection } from "./Section";
import { GenericBody, SELF_DESCRIBING_KINDS } from "./GenericBody";
import { DETAILS_BODY, detailFacts } from "./detailData";

function object(
  kind: string,
  spec: Record<string, unknown> = {},
  status: Record<string, unknown> = {},
  metadata: NonNullable<K8sObject["metadata"]> = { name: "obj-1", namespace: "default" },
): K8sObject {
  return { kind, apiVersion: "v1", metadata, spec, status } as K8sObject;
}

/** Scans the whole rendered document for a substring — text content, `title`,
 *  `aria-label`, `data-*`, everything a DOM inspector or a screen reader
 *  would see, not only what a text query happens to match. A boolean
 *  assertion rather than an element query, so a failure here never prints
 *  the sensitive value into the test output — matches `SecretBody.test.tsx`'s
 *  own `documentContains` helper. */
function documentContains(value: string): boolean {
  return document.body.innerHTML.includes(value);
}

/**
 * A nested `DETAILS_BODY` in the shape every real one has: its blocks returned
 * as siblings of the wrapper's own, wrapped in nothing.
 * `ServiceDetailsBody`, `NodeDetailsBody`, `SecretDetailsBody` and the rest
 * all return a fragment for exactly this reason, and each pins it in its own
 * file. Used here so the wrapper's tests model a real body rather than the
 * mistake its doc comment warns about.
 */
function NestedBody({ title }: { title: string }) {
  return (
    <Section title={title} className="nested-body">
      {`${title} rows`}
    </Section>
  );
}

/**
 * The shape the wrapper's doc comment forbids: a nested body that returns its
 * blocks inside an element of its own.
 */
function WrappedBody() {
  return (
    <div>
      <Section title="Wrapped body">rows</Section>
    </div>
  );
}

/**
 * Whether the hairline chain is unbroken: `.section + .section` is the rule
 * that draws it, so every block of the run has to be a direct sibling of every
 * other. One element wrapped around one block costs the rule on both sides of
 * it — above and below — and nothing about the rendering looks wrong.
 */
function runIsUnbroken(container: HTMLElement): boolean {
  return [...container.children].every((el) => el.matches("section.section"));
}

/**
 * A kind's lead fact list, drawn.
 *
 * The facts are DATA (`detailFacts`, and `identityFacts` for every kind
 * without a list of its own) because the peek and the full tab lay one list
 * out two different ways. Neither layout is this file's business, so the cases
 * below render the plainest possible rows and assert what was DERIVED; each
 * screen's own test pins its own layout. (#331)
 */
function Lead({ kind, object }: { kind: string; object: K8sObject }) {
  const facts = detailFacts({ kind, object });
  if (facts.length === 0) return null;
  return (
    <DetailSection>
      {facts.map((fact) => (
        <KV key={fact.label} k={fact.label} v={fact.value} mono={fact.mono} />
      ))}
    </DetailSection>
  );
}

/** What a screen composes: the lead facts, then the wrapper's blocks. */
function renderFacts(kind: string, object: K8sObject) {
  return render(<Lead kind={kind} object={object} />);
}

describe("GenericBody", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    podsForSelector.mockResolvedValue({ pods: [] });
    podMetrics.mockResolvedValue({ metrics: [] });
  });

  describe("a kind with no specific body", () => {
    // Lease has no `DETAILS_BODY` entry in `ResourceDetailView` and is not one of
    // `SELF_DESCRIBING_KINDS` — exactly the ~23-kind case this task exists
    // to fix: no `children` at all, the wrapper alone must be a complete,
    // correct detail.
    const LEASE = object(
      "Lease",
      {},
      {},
      {
        name: "lease-1",
        namespace: "kube-node-lease",
        creationTimestamp: "2026-08-20T00:00:00Z",
        labels: { app: "controller" },
        annotations: { "kubectl.kubernetes.io/note": "renewed automatically" },
        ownerReferences: [{ kind: "Node", name: "node-a" }],
      },
    );

    it("derives every identity fact, with cross-resource references as plain text", () => {
      renderFacts("Lease", LEASE);
      expect(screen.getByText("kube-node-lease")).toBeDefined();
      expect(screen.getByText("Node/node-a")).toBeDefined();

      // Namespace and Controlled by are `ResourceLink`/`LinkedResources` in
      // classic that navigate; nothing here can (`PaneBody` has no
      // navigation contract — see the task report), so neither renders as a
      // navigation control.
      expect(screen.queryByRole("button", { name: /^Open / })).toBeNull();
    });

    it("heads the first fact list with nothing, the way the design does", () => {
      // The pane's own header has already named the subject; a "Metadata"
      // bar above the first list is a second name for it. There is no title
      // to give it either: the list is data, and neither screen heads it.
      renderFacts("Lease", LEASE);
      expect(screen.queryByText("Metadata")).toBeNull();
      expect(screen.queryByRole("heading", { name: "Metadata" })).toBeNull();
    });

    it("drops Name, which repeats the header verbatim", () => {
      renderFacts("Lease", LEASE);
      expect(screen.queryByText("Name")).toBeNull();
      expect(screen.queryByText("lease-1")).toBeNull();
    });

    it("dates the object by age alone, not age plus an absolute stamp", () => {
      renderFacts("Lease", LEASE);
      expect(screen.getByText(/^\d+[smhd] ago$/)).toBeDefined();
      expect(screen.queryByText(/ago \(/)).toBeNull();
    });

    it("omits absent identity facts rather than showing them empty", () => {
      const bare = object("Lease", {}, {}, { name: "bare-lease" });
      const { container } = renderFacts("Lease", bare);
      expect(detailFacts({ kind: "Lease", object: bare })).toEqual([]);
      // And no block at all rather than an empty one, which would still have
      // its padding and still rule against whatever followed it.
      expect(container.querySelector("section.section")).toBeNull();
    });
  });

  describe("the run of sections", () => {
    it("is flat blocks divided by rules, not a stack of cards", () => {
      const { container } = render(
        <GenericBody
          kind="Lease"
          object={object("Lease", {}, { conditions: [{ type: "Ready", status: "True" }] }, { name: "l", namespace: "default" })}
          context="ctx"
        />,
      );
      expect(container.querySelector("section.section")).not.toBeNull();
      expect(container.querySelector(".card")).toBeNull();
    });

    it("lands every block as a sibling, the wrapper's own and the nested body's alike", () => {
      const { container } = render(
        <GenericBody
          kind="Lease"
          object={object(
            "Lease",
            {},
            { conditions: [{ type: "Ready", status: "True" }] },
            { name: "l", namespace: "default", labels: { app: "controller" } },
          )}
          context="ctx"
        >
          <NestedBody title="Nested kind body" />
        </GenericBody>,
      );
      expect(container.children.length).toBeGreaterThan(1);
      expect(runIsUnbroken(container)).toBe(true);
    });

    it("loses the rule on both sides of a nested body that wraps its own blocks", () => {
      // The invariant the wrapper's doc comment states, made checkable — and
      // the reason the check above is worth anything. A body returning its
      // blocks inside a div is a sibling of neither the block before it nor
      // the one after, so `.section + .section` matches at neither join and
      // two hairlines vanish with nothing else looking wrong. Asserted from
      // the violating side so the guard is known to discriminate rather than
      // to pass on any shape at all.
      const { container } = render(
        <GenericBody
          kind="Lease"
          object={object("Lease", {}, { conditions: [{ type: "Ready", status: "True" }] }, {
            name: "l",
            namespace: "default",
          })}
          context="ctx"
        >
          <WrappedBody />
        </GenericBody>,
      );
      expect(runIsUnbroken(container)).toBe(false);
      const wrapped = [...container.children].filter((el) => !el.matches("section.section"));
      expect(wrapped.map((el) => el.tagName)).toEqual(["DIV"]);
    });

    it("renders no block at all when a block has nothing to say", () => {
      // An empty section still has padding and still draws a rule against the
      // next one, so a missing middle block must be absent, not blank.
      const { container } = render(
        <GenericBody kind="Lease" object={object("Lease", {}, {}, { name: "bare-lease" })} context="ctx" />,
      );
      expect(container.querySelectorAll("section.section")).toHaveLength(0);
    });

    it("opens the run with the nested body, since the lead facts are the screen's to place above it", () => {
      const { container } = render(
        <GenericBody kind="ConfigMap" object={object("ConfigMap", {}, {}, { name: "cm-1" })} context="ctx">
          <NestedBody title="Nested kind body" />
        </GenericBody>,
      );
      expect(container.children[0]).toBe(container.querySelector(".nested-body"));
      expect(runIsUnbroken(container)).toBe(true);
    });
  });

  /**
   * Labels, Annotations and the Secret annotation gate all moved off this
   * wrapper and onto the pane that draws it — see `ResourceDetailView.test`'s
   * "Labels and Annotations, which the host places", where every one of the
   * properties that used to be asserted here is asserted through the real
   * render path instead.
   *
   * They moved because the two hosts lay them out differently: the peek stacks
   * them under the rest of the body and the full tab reads them side by side.
   * A body rendering its own could only ever produce one of those, and the
   * three copies that used to exist (here, `PodBody`, `WorkloadBody`) were
   * exactly the arrangement that let a security gate rest on a membership list
   * two files away.
   */
  describe("Labels and Annotations", () => {
    it("renders neither, so the host can place them", () => {
      render(
        <GenericBody
          kind="ConfigMap"
          object={object("ConfigMap", {}, {}, {
            name: "cm-1",
            labels: { app: "checkout" },
            annotations: { "srelens.io/note": "hello" },
          })}
          context="ctx"
        />,
      );
      expect(screen.queryByRole("heading", { level: 3, name: "Labels" })).toBeNull();
      expect(screen.queryByRole("heading", { level: 3, name: "Annotations" })).toBeNull();
    });
  });

  describe("a kind with a DETAILS_BODY entry", () => {
    it("reads lead facts, then the nested body, in classic's order", () => {
      // Classic's `GenericDetail` nests `KindBody` after its own metadata
      // section, and both screens keep that order — they place the lead facts
      // above the wrapper. Composed here the way a screen composes it, since
      // the two halves now live on either side of the data/design line.
      const CM = object("ConfigMap");
      const { container } = render(
        <>
          <Lead kind="ConfigMap" object={CM} />
          <GenericBody kind="ConfigMap" object={CM} context="ctx">
            <NestedBody title="Nested kind body" />
          </GenericBody>
        </>,
      );
      const namespaceKey = screen.getByText("Namespace");
      const nested = screen.getByRole("heading", { level: 3, name: "Nested kind body" });
      // eslint-disable-next-line no-bitwise
      expect(namespaceKey.compareDocumentPosition(nested) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
      // And still one unbroken run of siblings across the join.
      expect(runIsUnbroken(container)).toBe(true);
    });
  });

  describe("related pods", () => {
    it("renders related pods for a kind whose relatedPodSelector finds one", async () => {
      podsForSelector.mockResolvedValue({
        pods: [
          { name: "svc-pod-1", namespace: "default", phase: "Running", ready: "1/1", restarts: 0, node: "node-a", age: "2d", image: "app:1.0" },
        ],
      });
      render(
        <GenericBody
          kind="Service"
          object={object("Service", { selector: { app: "web" } }, {}, { name: "web", namespace: "default" })}
          context="ctx"
        />,
      );
      await waitFor(() => expect(screen.getByText("Pods")).toBeDefined());
      await waitFor(() => expect(screen.getByText("svc-pod-1")).toBeDefined());
      expect(podsForSelector).toHaveBeenCalledWith("ctx", "default", { app: "web" }, []);
    });

    it("does not render related pods for a kind relatedPodSelector finds none for", () => {
      render(
        <GenericBody
          kind="ConfigMap"
          object={object("ConfigMap", {}, {}, { name: "cm-1", namespace: "default" })}
          context="ctx"
        />,
      );
      expect(screen.queryByText("Pods")).toBeNull();
      expect(podsForSelector).not.toHaveBeenCalled();
    });
  });

  describe("conditions", () => {
    it("renders conditions as the shared rows, not a sortable table", () => {
      render(
        <GenericBody
          kind="Lease"
          object={object(
            "Lease",
            {},
            { conditions: [{ type: "Ready", status: "True", reason: "AsExpected", lastTransitionTime: "2026-08-20T00:00:00Z" }] },
          )}
          context="ctx"
        />,
      );
      expect(screen.getByRole("heading", { level: 3, name: "Conditions" })).toBeDefined();
      expect(screen.getByText("Ready")).toBeDefined();
      expect(screen.getByText("True · AsExpected")).toBeDefined();
      expect(screen.queryByText("Last transition")).toBeNull();
      expect(screen.queryByRole("columnheader")).toBeNull();
    });

    it("reads as empty rather than broken when the object reports no conditions", () => {
      render(<GenericBody kind="Lease" object={object("Lease", {}, {})} context="ctx" />);
      expect(screen.queryByText("Conditions")).toBeNull();
    });
  });

  describe("the four self-describing kinds", () => {
    it("lists exactly Pod, Deployment, StatefulSet and ReplicaSet", () => {
      expect([...SELF_DESCRIBING_KINDS].sort()).toEqual(
        ["Deployment", "Pod", "ReplicaSet", "StatefulSet"].sort(),
      );
    });

    it.each([...SELF_DESCRIBING_KINDS])("passes %s's children through untouched", (kind) => {
      render(
        <GenericBody kind={kind} object={object(kind)} context="ctx">
          <NestedBody title="Own properties" />
        </GenericBody>,
      );
      expect(screen.getByRole("heading", { level: 3, name: "Own properties" })).toBeDefined();
      // No Conditions and no Pods either: these four state their own.
      expect(screen.queryByRole("heading", { level: 3, name: "Conditions" })).toBeNull();
    });

    it.each([...SELF_DESCRIBING_KINDS])("gives %s a fact list of its own rather than the identity one", (kind) => {
      // The other half of the same rule, and the half that used to be a
      // `SELF_DESCRIBING_KINDS` check inside the wrapper: these four already
      // state Namespace, Created and their owner, so they must not be handed
      // the generic list on top of their own.
      const facts = detailFacts({ kind, object: object(kind) }).map((f) => f.label);
      expect(facts).not.toEqual(["Namespace"]);
    });

    // DaemonSet is deliberately NOT in `SELF_DESCRIBING_KINDS` — classic's
    // `ObjectDetail` does not special-case it either, so it still gets the
    // wrapper (and its own DaemonSetBody nests inside it, per classic's
    // `GenericDetail` + `KindBody`).
    it("still wraps DaemonSet, which classic does not special-case", () => {
      const DS = object("DaemonSet");
      render(
        <>
          <Lead kind="DaemonSet" object={DS} />
          <GenericBody kind="DaemonSet" object={DS} context="ctx">
            <NestedBody title="Scheduling" />
          </GenericBody>
        </>,
      );
      // It leads with the identity facts every wrapped kind gets — a
      // DaemonSet's own numbers are per-node and state themselves in a titled
      // Scheduling block instead.
      expect(screen.getByText("Namespace")).toBeDefined();
      expect(screen.getByRole("heading", { level: 3, name: "Scheduling" })).toBeDefined();
    });
  });
});

/**
 * The hairline guard, run against the REAL bodies rather than the synthetic
 * `NestedBody` above.
 *
 * `runIsUnbroken` proved it discriminates — `WrappedBody` fails it — but until
 * now nothing asserted that any body actually shipped satisfies it, which is
 * the claim the rule exists to make. It caught one: `WorkloadDetailsBody`
 * returned a bare `EmptyState` (not a `.section`) from an `if (!kind)` guard,
 * and a run containing it loses the rule on both sides. That guard is gone —
 * the kind is now the route's, and `DETAILS_BODY[""]` is undefined. (#331)
 */
describe("every real DETAILS_BODY entry lands in an unbroken run", () => {
  /** Enough of each kind for its body to draw something. Deliberately plain:
   *  the sweep is about the SHAPE of what each body returns, not its facts. */
  const FIXTURES: Record<string, K8sObject> = {
    Pod: object("Pod", { containers: [{ name: "app", image: "app:1" }], nodeName: "node-a" }, {
      phase: "Running",
      conditions: [{ type: "Ready", status: "True" }],
      containerStatuses: [{ name: "app", ready: true, restartCount: 0, state: { running: { startedAt: "2026-08-20T00:00:00Z" } } }],
    }),
    Deployment: object("Deployment", { replicas: 1, selector: { matchLabels: { app: "web" } } }, { readyReplicas: 1 }),
    StatefulSet: object("StatefulSet", { replicas: 1, selector: { matchLabels: { app: "db" } } }, { readyReplicas: 1 }),
    DaemonSet: object("DaemonSet", { selector: { matchLabels: { app: "agent" } } }, { desiredNumberScheduled: 1, numberReady: 1 }),
    ReplicaSet: object("ReplicaSet", { replicas: 1, selector: { matchLabels: { app: "web" } } }, { readyReplicas: 1 }),
    Service: object("Service", { type: "ClusterIP", clusterIP: "10.0.0.1", ports: [{ port: 80 }] }, {}),
    Node: object("Node", {}, { nodeInfo: { kubeletVersion: "v1.30.0" }, capacity: { cpu: "4" }, allocatable: { cpu: "3800m" } }, { name: "node-a" }),
    Job: object("Job", { completions: 1 }, { succeeded: 1, startTime: "2026-08-20T00:00:00Z" }),
    CronJob: object("CronJob", { schedule: "0 2 * * *" }, {}),
    ConfigMap: { ...object("ConfigMap", {}, {}), data: { "app.conf": "k=v" } } as K8sObject,
    Secret: { ...object("Secret", {}, {}), type: "Opaque", data: { token: "cmVkYWN0ZWQ=" } } as K8sObject,
  };

  it("covers every entry in the table, so the sweep cannot silently shrink", () => {
    expect(Object.keys(FIXTURES).sort()).toEqual(Object.keys(DETAILS_BODY).sort());
  });

  it.each(Object.keys(DETAILS_BODY))("keeps the run unbroken for %s, wrapper and body together", async (kind) => {
    const Body = DETAILS_BODY[kind];
    const fixture = FIXTURES[kind];
    const { container } = render(
      <>
        {/* Composed the way a screen composes it: the lead facts, then the
            wrapper and the kind's own blocks. A body alone is not the run a
            reader sees, and the join between the two is exactly where a
            hairline would be lost. */}
        <Lead kind={kind} object={fixture} />
        <GenericBody kind={kind} object={fixture} context="ctx">
          <Body kind={kind} object={fixture} context="ctx" revisions={{ status: "idle" }} />
        </GenericBody>
      </>,
    );
    // Every block a sibling of every other: `.section + .section` is what
    // draws the hairline, so one element wrapped around one block costs the
    // rule above AND below it, with nothing about the rendering looking wrong.
    expect({ kind, unbroken: runIsUnbroken(container) }).toEqual({ kind, unbroken: true });
    expect(container.children.length).toBeGreaterThan(0);
    // Settles the live reads inside act(), so no state update lands after the
    // assertion.
    await waitFor(() => expect(container.children.length).toBeGreaterThan(0));
  });
});
