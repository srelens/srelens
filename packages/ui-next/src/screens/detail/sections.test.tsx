import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { Condition } from "@srelens/core";
import {
  AnnotationsSection,
  AnnotationLines,
  ConditionsSection,
  LabelsSection,
  StringList,
  partitionAnnotations,
} from "./sections";

const DEPLOYMENT_CONDITIONS: Condition[] = [
  { type: "Available", status: "False", reason: "MinimumReplicasUnavailable" },
  { type: "Progressing", status: "True", reason: "ReplicaSetUpdated" },
  { type: "ReplicaFailure", status: "False" },
];

/** The colour `StatusPill` painted a condition's name with, empty when it
 *  left the name plain. Read off the inline style the component documents as
 *  its own mechanism rather than off a `data-*` attribute, which is the kit's
 *  to add or drop. */
function tone(container: HTMLElement, name: string): string {
  const pill = [...container.querySelectorAll(".status")].find((el) => el.textContent === name);
  return (pill as HTMLElement | undefined)?.style.color ?? "no such condition";
}

describe("ConditionsSection", () => {
  it("names the block and gives every condition a row", () => {
    const { container } = render(<ConditionsSection conditions={DEPLOYMENT_CONDITIONS} />);
    expect(screen.getByRole("heading", { level: 3, name: "Conditions" })).toBeDefined();
    expect(container.querySelectorAll(".kv")).toHaveLength(3);
  });

  it("reads the condition's name on the left and its status and reason on the right", () => {
    render(<ConditionsSection conditions={DEPLOYMENT_CONDITIONS} />);
    expect(screen.getByText("Available")).toBeDefined();
    expect(screen.getByText("False · MinimumReplicasUnavailable")).toBeDefined();
    expect(screen.getByText("True · ReplicaSetUpdated")).toBeDefined();
  });

  it("stands an em dash in for a condition that reports no reason", () => {
    render(<ConditionsSection conditions={DEPLOYMENT_CONDITIONS} />);
    expect(screen.getByText("False · —")).toBeDefined();
  });

  it("drops the last-transition column the design does not have", () => {
    render(
      <ConditionsSection
        conditions={[{ type: "Available", status: "True", lastTransitionTime: "2026-08-20T00:00:00Z" }]}
      />,
    );
    expect(screen.queryByText("Last transition")).toBeNull();
    expect(screen.queryByText(/ago/)).toBeNull();
  });

  it("colours the name of a bad condition and leaves a good one plain", () => {
    // The design's asymmetric rule: red `Available`, plain `ReplicaFailure`
    // beside its own ok dot. `StatusPill` owns which kinds count as bad and
    // paints the word with an inline tone; this asserts the flag reached it,
    // not a colour computed here. `Progressing` is the third case and is
    // pinned with the whole row below.
    const { container } = render(<ConditionsSection conditions={DEPLOYMENT_CONDITIONS} />);
    expect(tone(container, "Available")).toBe("var(--sev)");
    expect(tone(container, "ReplicaFailure")).toBe("");
  });

  it("draws the design frame's three-tone Conditions row entire: danger, warning, ok", () => {
    // THE pin for frame A of the user's mock, asserted as one thing because
    // it is one thing: three conditions of a mid-rollout Deployment, each
    // with its own dot tone AND its own name colour, in the frame's order.
    //
    //   (danger)  Available       False · MinimumReplicasUnavailable   red name
    //   (warn)    Progressing     True  · ReplicaSetUpdated            amber name
    //   (ok)      ReplicaFailure  False · —                            plain name
    //
    // Every tone is core's `conditionKindWithReason`; this section keeps no
    // second heuristic, which is why the two bugs the frame exposed could be
    // fixed where the list column and every other reader of a condition's
    // tone got them too. `Failed` did not match `ReplicaFailure`, so a
    // Deployment's healthy state read as a failure — a genuine fix, and so
    // one BOTH designs take, in plain `conditionKind`. The amber
    // `Progressing` is the other half and is this design's alone: a rollout
    // still in flight was drawn the same green as one that had landed, and
    // the mock's two frames prove the difference tone by reason with the same
    // type and status in both. Classic never asked for that reading, so it
    // lives in the `WithReason` variant this section calls and classic does
    // not.
    //
    // The name colour is the asymmetric half of the rule: a bad state is
    // worth the ink, a good one is not, so the ok row alone reads plain.
    const { container } = render(<ConditionsSection conditions={DEPLOYMENT_CONDITIONS} />);
    const kinds = [...container.querySelectorAll(".status")].map((el) => el.getAttribute("data-kind"));
    expect(kinds).toEqual(["danger", "warning", "success"]);
    expect([
      tone(container, "Available"),
      tone(container, "Progressing"),
      tone(container, "ReplicaFailure"),
    ]).toEqual(["var(--sev)", "var(--warn)", ""]);
  });

  it("says the state in words, never in colour alone", () => {
    render(<ConditionsSection conditions={[{ type: "Available", status: "False" }]} />);
    // "False" is in the row's own text, so the dot is a second channel.
    expect(screen.getByText(/False/)).toBeDefined();
  });

  it("renders nothing at all for an object that reports no conditions", () => {
    const { container } = render(<ConditionsSection conditions={[]} />);
    expect(container.querySelector("section")).toBeNull();
    expect(container.innerHTML).toBe("");
  });

  it("is a flat section, not a card", () => {
    const { container } = render(<ConditionsSection conditions={DEPLOYMENT_CONDITIONS} />);
    expect(container.querySelector("section.section")).not.toBeNull();
    expect(container.querySelector(".card")).toBeNull();
  });

  it("takes conditions as data, so any kind's list renders the same way", () => {
    // A Pod's lifecycle conditions, which carry no reason at all — the module
    // must not reach for a workload's status or a Pod's phase to render them.
    render(
      <ConditionsSection
        conditions={[
          { type: "Initialized", status: "True" },
          { type: "Ready", status: "True" },
        ]}
      />,
    );
    expect(screen.getByText("Initialized")).toBeDefined();
    expect(screen.getByText("Ready")).toBeDefined();
    expect(screen.getAllByText("True · —")).toHaveLength(2);
  });
});

describe("AnnotationLines", () => {
  const APPLIED = "kubectl.kubernetes.io/last-applied-configuration";
  const MANIFEST = `{"apiVersion":"apps/v1","kind":"Deployment","metadata":{"name":"checkout-api"},"spec":{"replicas":12}}`;

  it("prints every annotation as a full-width key=value line", () => {
    render(<AnnotationLines annotations={{ "checksum/config": "8f41c2a9", "srelens.io/last-applied-by": "dana@acme.io" }} />);
    expect(screen.getByText("checksum/config=")).toBeDefined();
    expect(screen.getByText("8f41c2a9")).toBeDefined();
    expect(screen.getByText("dana@acme.io")).toBeDefined();
  });

  it("wraps long values instead of truncating them, since nothing else can read them now", () => {
    // `PairList` no longer writes the value into a `title`, so a truncated
    // row is a value nobody can read at all.
    const { container } = render(<AnnotationLines annotations={{ note: "a".repeat(400) }} />);
    expect(container.querySelector("li.truncate")).toBeNull();
    expect(container.querySelector(".v.break-all")).not.toBeNull();
  });

  it("withholds the applied-manifest annotation and says where to read it", () => {
    render(<AnnotationLines annotations={{ [APPLIED]: MANIFEST, "checksum/config": "8f41c2a9" }} />);
    expect(screen.queryByText(MANIFEST)).toBeNull();
    expect(screen.queryByText(`${APPLIED}=`)).toBeNull();
    const note = screen.getByText(new RegExp(APPLIED));
    expect(note.textContent).toMatch(/YAML/);
    // The other annotations are untouched.
    expect(screen.getByText("8f41c2a9")).toBeDefined();
  });

  it("keeps the withheld value out of the document entirely, not merely out of sight", () => {
    const { container } = render(<AnnotationLines annotations={{ [APPLIED]: MANIFEST }} />);
    expect(container.innerHTML).not.toContain("replicas");
    expect(container.querySelector("[title]")).toBeNull();
  });

  it("says nothing about withholding when there is nothing to withhold", () => {
    render(<AnnotationLines annotations={{ "checksum/config": "8f41c2a9" }} />);
    expect(screen.queryByText(/not printed/)).toBeNull();
  });

  it("renders nothing at all for an object with no annotations", () => {
    const { container } = render(<AnnotationLines annotations={{}} />);
    expect(container.innerHTML).toBe("");
  });

  it("names what it withheld, so a caller can render its own note", () => {
    expect(partitionAnnotations({ [APPLIED]: MANIFEST, app: "web" })).toEqual({
      shown: [["app", "web"]],
      withheld: [APPLIED],
    });
    expect(partitionAnnotations({ app: "web" })).toEqual({ shown: [["app", "web"]], withheld: [] });
  });
});

/** Whether a string appears anywhere in the rendered markup — text, `title`,
 *  `aria-label`, `data-*`, everything a DOM inspector or a screen reader would
 *  see. A boolean rather than an element query, so a failure never prints the
 *  sensitive value into the test output. */
function documentContains(value: string): boolean {
  return document.body.innerHTML.includes(value);
}

describe("StringList", () => {
  it("gives every item its own line", () => {
    const { container } = render(<StringList items={["10.1.2.3", "fd00::1"]} />);
    expect([...container.querySelectorAll("li")].map((li) => li.textContent)).toEqual(["10.1.2.3", "fd00::1"]);
  });

  it("renders an empty list as an empty list, not as an absent block", () => {
    const { container } = render(<StringList items={[]} />);
    expect(container.querySelectorAll("li")).toHaveLength(0);
    expect(container.querySelector("ul")).not.toBeNull();
  });
});

describe("LabelsSection", () => {
  it("heads the block and prints every pair", () => {
    render(<LabelsSection labels={{ app: "web", tier: "front" }} />);
    expect(screen.getByRole("heading", { level: 3, name: "Labels" })).toBeDefined();
    expect(screen.getByText("web")).toBeDefined();
    expect(screen.getByText("front")).toBeDefined();
  });

  it("renders nothing at all when the object has none — an empty block still draws its own rule", () => {
    const { container } = render(<LabelsSection labels={{}} />);
    expect(container.innerHTML).toBe("");
  });
});

/**
 * The gate, pinned on the component that now holds it rather than on one of
 * the three bodies that used to hold three copies of it.
 *
 * Two of those copies — `PodBody`'s and `WorkloadBody`'s — had NO `Secret`
 * branch. That was safe only because the four kinds those bodies serve are
 * `SELF_DESCRIBING_KINDS`, none of which can be a Secret: a security gate
 * resting on a membership list in a third file. These tests fail for the right
 * reason whichever body renders the section, because there is only one
 * section. (#331)
 */
describe("AnnotationsSection — the secrecy gate, wherever it is rendered from", () => {
  // Obviously-fake fixture text — never anything that reads as a real
  // manifest or credential.
  const FIXTURE_VALUE = "fixture-only-not-a-real-last-applied-manifest";
  const APPLIED_KEY = "kubectl.kubernetes.io/last-applied-configuration";

  it("keeps a Secret's annotation value out of the document until a reader asks", async () => {
    render(<AnnotationsSection kind="Secret" annotations={{ "srelens.io/rotated-from": FIXTURE_VALUE }} />);
    // An ORDINARY annotation key, deliberately: the shared legibility rule
    // drops `last-applied-configuration` on every kind, so using that key
    // would prove nothing about the gate.
    expect(documentContains(FIXTURE_VALUE)).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Show 1 annotation" }));
    expect(documentContains(FIXTURE_VALUE)).toBe(true);
  });

  it("still gates a Secret whose applied manifest the shared rule would have withheld anyway", () => {
    // The two rules must not be confused for one. `AnnotationLines` drops
    // `last-applied-configuration` for legibility and would happen to drop
    // this value too — but a Secret never reaches it. The gate is what keeps
    // the value out, and it is still the gate doing it.
    render(<AnnotationsSection kind="Secret" annotations={{ [APPLIED_KEY]: FIXTURE_VALUE }} />);
    expect(documentContains(FIXTURE_VALUE)).toBe(false);
    expect(screen.getByRole("button", { name: "Show 1 annotation" })).toBeDefined();
  });

  it("gates Secret and nothing else — every other kind's annotations are open, as the design draws them", () => {
    for (const kind of ["Pod", "Deployment", "StatefulSet", "ReplicaSet", "ConfigMap", "Lease"]) {
      const { unmount } = render(
        <AnnotationsSection kind={kind} annotations={{ "srelens.io/last-applied-by": "dana@acme.io" }} />,
      );
      expect({ kind, open: documentContains("dana@acme.io") }).toEqual({ kind, open: true });
      expect({ kind, toggle: screen.queryByRole("button", { name: /^Show / }) }).toEqual({ kind, toggle: null });
      unmount();
    }
  });

  it("renders nothing at all for an object with no annotations", () => {
    const { container } = render(<AnnotationsSection kind="Secret" annotations={{}} />);
    expect(container.innerHTML).toBe("");
  });
});

/**
 * The copies cannot come back silently.
 *
 * `StringList` had SIX byte-identical definitions across this directory,
 * `LabelsSection` three and `AnnotationsSection` three — and two of the
 * `AnnotationsSection` copies were missing the `Secret` branch above. Every one
 * of them was justified in a comment saying it was too small to share. Read
 * off the source rather than inferred from behaviour, because a re-added copy
 * would behave identically on the day it was written; that is exactly why the
 * first six were never noticed. (#331)
 */
describe("one definition each, across every detail body", () => {
  const here = dirname(fileURLToPath(import.meta.url));
  const sources = readdirSync(here)
    .filter((f) => f.endsWith(".tsx") && !f.endsWith(".test.tsx"))
    .map((f) => ({ file: f, text: readFileSync(join(here, f), "utf8") }));

  const SHARED = [
    "StringList",
    "LabelsSection",
    "AnnotationsSection",
    "AnnotationsToggle",
    "AnnotationLines",
    "RelatedPodsSection",
    "ConditionsSection",
  ];

  /**
   * Every form a component in this directory is actually written in:
   * `function X(`, and `const X = (`/`const X: T = (` for the arrow form.
   * The first version of this sweep matched only the `function` form, which
   * left a re-added copy free to escape it by being written the other way —
   * and both forms are already in use here (`AnnotationLines` is a function,
   * `RELATED_POD_COLUMNS` a const), so that was not a hypothetical.
   */
  const definesComponent = (text: string, name: string) =>
    new RegExp(`(?:function\\s+${name}\\s*[(<]|const\\s+${name}\\s*(?::[^=\\n]*)?=)`).test(text);

  it.each(SHARED)("defines %s exactly once, in sections.tsx", (name) => {
    const definedIn = sources.filter(({ text }) => definesComponent(text, name)).map(({ file }) => file);
    expect(definedIn).toEqual(["sections.tsx"]);
  });

  it("catches a copy written either way round, so the form is not the escape hatch", () => {
    // The predicate itself, checked against both spellings and against a
    // near-miss that must NOT match — a sweep that matched everything would
    // pass this file's real sources by accident.
    for (const name of SHARED) {
      expect(definesComponent(`function ${name}({ items }: { items: string[] }) {`, name)).toBe(true);
      expect(definesComponent(`const ${name} = ({ items }: { items: string[] }) => (`, name)).toBe(true);
      expect(definesComponent(`const ${name}: FC<Props> = (props) => (`, name)).toBe(true);
      // A call site and an import are not a definition.
      expect(definesComponent(`      <${name} items={ports} />`, name)).toBe(false);
      expect(definesComponent(`import { ${name} } from "./sections";`, name)).toBe(false);
    }
  });

  it("reads a directory with every body in it, so the sweep above is not vacuous", () => {
    const files = sources.map((s) => s.file);
    expect(files).toContain("sections.tsx");
    for (const body of ["CronJobBody", "GenericBody", "PodBody", "SecretBody", "ServiceBody", "WorkloadBody"]) {
      expect(files).toContain(`${body}.tsx`);
    }
  });
});
