import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext } from "@srelens/core";
import type { Probe } from "../../lib/probe";
import type { ClusterRow } from "./ClusterTable";
import { SourcesRail } from "./SourcesRail";

const ctx = (over: Partial<ClusterContext> = {}): ClusterContext => ({
  name: "prod-eu",
  // Unique per name unless a test says otherwise, so a list of rows has a list
  // of keys.
  stableId: over.name ?? "prod-eu",
  cluster: "prod",
  server: "https://prod:6443",
  namespace: "",
  isCurrent: false,
  isLocal: false,
  sourceFile: "/home/dana/.kube/config",
  authKind: "exec plugin · gcloud",
  ...over,
});

const row = (over: Partial<ClusterContext> = {}, probe: Probe = { state: "unread" }): ClusterRow => ({
  context: ctx(over),
  probe,
});

/**
 * The one sentence §6's third section renders, asserted as EXACT TEXT.
 *
 * A regexp over a phrase would pass against a section that also drew a badge
 * per cluster beside it, which is the thing this suite exists to forbid.
 */
const GATE =
  "Every change the agent makes to any cluster on this list stops at a confirmation prompt first — the same gate whether the cluster runs on this laptop or across the internet.";

/** §6's footnote under the kubeconfig section, verbatim. */
const FOOTNOTE =
  "srelens reads these files in place and connects to the API server directly. Nothing is copied anywhere.";

describe("SourcesRail", () => {
  it("is a named region 292px wide", () => {
    const { container } = render(<SourcesRail files={[]} rows={[]} />);
    const aside = container.querySelector("aside");
    expect(aside?.style.width).toBe("292px");
    expect(screen.getByRole("complementary", { name: "Sources" })).toBeTruthy();
  });

  it("counts a file's contexts, and how many are in use", () => {
    render(
      <SourcesRail
        files={["/k/config"]}
        rows={[
          row({ sourceFile: "/k/config", isCurrent: true }),
          row({ sourceFile: "/k/config", stableId: "b", name: "b" }),
        ]}
      />,
    );
    expect(screen.getByText(/2 contexts/)).toBeTruthy();
    // Exact, not a pattern: the whole line is the claim, and `2 contexts · 2 in
    // use` would satisfy the regexp above.
    expect(screen.getByText("2 contexts · 1 in use")).toBeTruthy();
  });

  it("counts each file's own contexts and nobody else's", () => {
    render(
      <SourcesRail
        files={["/k/one", "/k/two"]}
        rows={[
          row({ name: "a", sourceFile: "/k/one", isCurrent: true }),
          row({ name: "b", sourceFile: "/k/one" }),
          row({ name: "c", sourceFile: "/k/two" }),
        ]}
      />,
    );
    expect(screen.getByText("2 contexts · 1 in use")).toBeTruthy();
    expect(screen.getByText("1 context · 0 in use")).toBeTruthy();
  });

  it("keeps a stored file that no longer yields any context", () => {
    render(<SourcesRail files={["/k/gone.yaml"]} rows={[]} />);
    expect(screen.getByText("/k/gone.yaml")).toBeTruthy();
    expect(screen.getByText("0 contexts")).toBeTruthy();
    expect(
      screen.getByText(
        "No contexts came from this file. It may have been moved, deleted or emptied since it was added.",
      ),
    ).toBeTruthy();
  });

  it("says nothing about a missing file for one that still yields contexts", () => {
    render(<SourcesRail files={["/k/config"]} rows={[row({ sourceFile: "/k/config" })]} />);
    expect(screen.queryByText(/no contexts came from this file/i)).toBeNull();
  });

  it("names a file that supplies contexts even when it was never stored", () => {
    // Web mode: `loadKubeconfigFiles` is not read, so `files` is empty while
    // the contexts the server merged still came from somewhere.
    render(<SourcesRail files={[]} rows={[row({ sourceFile: "/srv/kubeconfig" })]} />);
    expect(screen.getByText("/srv/kubeconfig")).toBeTruthy();
    expect(screen.getByText("1 context · 0 in use")).toBeTruthy();
  });

  it("lists a stored path once when it is stored twice", () => {
    render(<SourcesRail files={["/k/config", "/k/config"]} rows={[]} />);
    expect(screen.getAllByText("/k/config")).toHaveLength(1);
    expect(screen.getAllByTestId("source-file")).toHaveLength(1);
  });

  it("draws no file row for a context that names no file", () => {
    render(<SourcesRail files={[]} rows={[row({ sourceFile: "" })]} />);
    expect(screen.queryAllByTestId("source-file")).toHaveLength(0);
  });

  it("keeps §6's footnote about reading the files in place", () => {
    render(<SourcesRail files={["/k/config"]} rows={[row({ sourceFile: "/k/config" })]} />);
    expect(screen.getByText(FOOTNOTE)).toBeTruthy();
  });

  it("offers no way to add a file when there is no filesystem to browse", () => {
    render(<SourcesRail files={["/k/config"]} rows={[row()]} />); // no onAddFile
    expect(screen.queryByRole("button", { name: /add/i })).toBeNull();
    expect(screen.getByText(/on the desktop/i)).toBeTruthy(); // the reason, said once
  });

  it("offers Add, and no explanation, when there is a filesystem to browse", async () => {
    const onAddFile = vi.fn();
    render(<SourcesRail files={["/k/config"]} rows={[row()]} onAddFile={onAddFile} />);
    expect(screen.queryByText(/on the desktop/i)).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: /add/i }));
    expect(onAddFile).toHaveBeenCalledTimes(1);
  });

  it("draws no local section when no cluster runs on this laptop", () => {
    render(<SourcesRail files={["/k/config"]} rows={[row({ sourceFile: "/k/config" })]} />);
    expect(screen.queryByTestId("sources-local")).toBeNull();
  });

  it("says how a local cluster is reached and what it measured", () => {
    const { container } = render(
      <SourcesRail
        files={[]}
        rows={[
          row(
            { name: "kind-dev", isLocal: true, provider: "kind", server: "https://127.0.0.1:6443" },
            { state: "reachable", latencyMs: 12 },
          ),
        ]}
      />,
    );
    const local = screen.getByTestId("sources-local");
    expect(local.textContent).toContain("kind-dev");
    expect(screen.getByText("kind · 127.0.0.1:6443")).toBeTruthy();
    expect(screen.getByText("12 ms")).toBeTruthy();
    // The other half of {@link readingPill}'s pin: one pill, and the reading
    // is inside it rather than merely somewhere on the rail.
    const pill = readingPill(container);
    expect(pill?.textContent).toBe("12 ms");
  });

  /**
   * **The badge is asserted as an ELEMENT, not as absent digits.**
   *
   * These two used to assert `queryByText(/\d+\s*ms/)` alone, and replacing
   * the rail's `{reading !== null && …}` gate with `{true && …}` — an EMPTY
   * muted pill beside a local cluster nothing has read — passed all 47 tests.
   * An empty pill is exactly "a placeholder that implies a reading was taken",
   * which is the shape this project's absent-not-zero rule exists to prevent,
   * and no assertion about digits can see it. So the pin is on the pill's own
   * node: for a cluster with no reading there is no pill at all.
   */
  function readingPill(container: HTMLElement): Element | null {
    return container.querySelector('[data-slot="local-reading"]');
  }

  it("shows no latency for a local cluster it has not read", () => {
    const { container } = render(
      <SourcesRail files={[]} rows={[row({ isLocal: true, provider: "kind" }, { state: "unread" })]} />,
    );
    expect(screen.queryByText(/0\s*ms/)).toBeNull();
    expect(screen.queryByText(/\d+\s*ms/)).toBeNull();
    expect(readingPill(container)).toBeNull();
  });

  it("shows no latency for a local cluster that did not answer", () => {
    const { container } = render(
      <SourcesRail
        files={[]}
        rows={[
          row(
            { isLocal: true, provider: "kind" },
            { state: "unreachable", latencyMs: 0, error: "connection refused" },
          ),
        ]}
      />,
    );
    expect(screen.queryByText(/0\s*ms/)).toBeNull();
    expect(screen.queryByText(/\d+\s*ms/)).toBeNull();
    expect(readingPill(container)).toBeNull();
  });

  it("draws a sub-millisecond reading as <1 ms, never as 0 ms", () => {
    render(
      <SourcesRail
        files={[]}
        rows={[row({ isLocal: true, provider: "kind" }, { state: "reachable", latencyMs: 0.2 })]}
      />,
    );
    expect(screen.getByText("<1 ms")).toBeTruthy();
    expect(screen.queryByText(/0\s*ms/)).toBeNull();
  });

  it("names a local cluster's endpoint with no leading separator when no provider was detected", () => {
    render(
      <SourcesRail
        files={[]}
        rows={[row({ name: "kind-dev", isLocal: true, server: "https://127.0.0.1:6443" })]}
      />,
    );
    // Exact text: a `parts.join(" · ")` over an absent provider produces
    // ` · 127.0.0.1:6443`, which no "contains" assertion would catch.
    expect(screen.getByText("127.0.0.1:6443")).toBeTruthy();
  });

  it("says the confirmation gate once, however many clusters there are", () => {
    render(
      <SourcesRail
        files={["/k/config"]}
        rows={[
          row({ name: "prod-eu", sourceFile: "/k/config" }),
          row({ name: "staging", sourceFile: "/k/config" }),
          row({ name: "kind-dev", isLocal: true, provider: "kind" }),
        ]}
      />,
    );
    expect(screen.getAllByText(GATE)).toHaveLength(1);
  });

  /**
   * §6's mock draws `read + write` / `read only` per cluster. Task 3
   * established that no such distinction exists anywhere in srelens: the write
   * gate is `crates/mcp/src/stdio.rs`'s alone, `isLocal` never enters it, and
   * no cluster is read-only. Two badges that differ by nothing imply a setting
   * the reader could change, so the section says one sentence and draws no
   * badge at all. Restoring the mock's shape has to fail this.
   */
  it("draws no per-cluster badge in what the agent may reach", () => {
    render(
      <SourcesRail
        files={["/k/config"]}
        rows={[
          row({ name: "prod-eu", sourceFile: "/k/config" }),
          row({ name: "kind-dev", isLocal: true, provider: "kind" }),
        ]}
      />,
    );
    const agent = screen.getByTestId("sources-agent");
    // Element absence, not a text pattern: `Badge` is the only thing in the kit
    // that renders `.badge`, and any wording of a per-cluster verdict would
    // have to name the cluster it is about.
    expect(agent.querySelectorAll(".badge")).toHaveLength(0);
    expect(agent.querySelectorAll("[data-tone]")).toHaveLength(0);
    expect(agent.textContent).not.toContain("prod-eu");
    expect(agent.textContent).not.toContain("kind-dev");
    expect(screen.queryByText(/read \+ write/i)).toBeNull();
    expect(screen.queryByText(/read only/i)).toBeNull();
  });

  /**
   * `min-width: auto` — eight defects on this migration, none of them visible
   * in jsdom, which is why the classes themselves are the assertion.
   */
  it("keeps the scroll on the body and lets a long path shrink", () => {
    const long = "/Users/dana/Library/Application Support/srelens/kubeconfigs/acme-prod.yaml";
    const { container } = render(<SourcesRail files={[long]} rows={[]} />);

    const aside = container.querySelector("aside");
    expect(aside?.className).toContain("side-rail");
    // The aside is the fixed frame; the body is what scrolls.
    expect(aside?.className).not.toContain("scroll");
    expect(aside?.className).not.toContain("overflow");

    const body = container.querySelector('[data-slot="rail-body"]');
    expect(body?.className).toContain("side-rail-body");
    expect(body?.className).toContain("min-w-0");

    const path = screen.getByText(long);
    expect(path.className).toContain("block");
    expect(path.className).toContain("truncate");
    expect(path.className).toMatch(/max-w-\[/);
    expect(path.title).toBe(long);
    expect(path.parentElement?.className).toContain("min-w-0");
  });

  it("lets a local cluster's name and endpoint shrink", () => {
    render(
      <SourcesRail
        files={[]}
        rows={[
          row(
            {
              name: "kind-a-rather-long-local-cluster-name",
              isLocal: true,
              provider: "kind",
              server: "https://127.0.0.1:6443",
            },
            { state: "reachable", latencyMs: 3 },
          ),
        ]}
      />,
    );
    const name = screen.getByText("kind-a-rather-long-local-cluster-name");
    expect(name.className).toContain("block");
    expect(name.className).toContain("truncate");
    expect(name.parentElement?.className).toContain("min-w-0");

    const via = screen.getByText("kind · 127.0.0.1:6443");
    expect(via.className).toContain("truncate");

    // The reading refuses to shrink; the name and the path absorb the width.
    expect(screen.getByText("3 ms").closest("[data-slot='local-reading']")?.className).toContain(
      "shrink-0",
    );
  });

  it("puts the caller's className on the frame", () => {
    const { container } = render(<SourcesRail files={[]} rows={[]} className="border-l-0" />);
    expect(container.querySelector("aside")?.className).toContain("border-l-0");
  });
});
