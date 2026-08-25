import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * Only the capability wrappers and the platform check are replaced.
 * `describeError` stays real, so the failure assertion below is against core's
 * own classification rather than a copy of it, and `plural` stays real so the
 * healthy line is counted by core's arithmetic.
 */
const core = vi.hoisted(() => ({
  diagnoseContext: vi.fn(),
  installPlugin: vi.fn(),
  startToolInstall: vi.fn(),
  toolboxStatus: vi.fn(),
  isTauri: vi.fn(() => true),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

import type { ClusterContext } from "@srelens/core";
import type { DiagnosisReport, RequirementResult } from "@srelens/core/lib/toolbox";
import { ExecAuthRail } from "./ExecAuthRail";
import { Toolbox } from "../Toolbox";
import { resetContexts, setContexts } from "../../lib/clusters";
import { resetProbes } from "../../lib/probe";
import { defaultState } from "../../lib/tabs";
import * as store from "../../lib/tabsStore";
import { resetView } from "../../lib/workspace";

/**
 * The three resolutions the backend models, as `RequirementStatusDto` sends
 * them (`crates/kube/src/toolbox.rs`).
 *
 * `OFF_PATH` is deliberately an **installable** tool that is present but off
 * the app's PATH: that is the pair the rail exists to tell apart, and a rail
 * that keyed its button off `installable` alone would draw an Install here.
 */
const MISSING: RequirementResult = {
  binary: "kubectl-oidc_login",
  kind: "krew-plugin",
  plugin: "oidc-login",
  installable: true,
  status: "missing",
  path: null,
  version: null,
};
const OFF_PATH: RequirementResult = {
  binary: "kubectl",
  kind: "kubectl",
  plugin: null,
  installable: true,
  status: "not-on-app-path",
  path: "/opt/homebrew/bin/kubectl",
  version: null,
};
const EXTERNAL: RequirementResult = {
  binary: "gke-gcloud-auth-plugin",
  kind: "external",
  plugin: null,
  installable: false,
  status: "missing",
  path: null,
  version: null,
};
/** Resolved, and carrying the one version the backend ever fills in. */
const FOUND: RequirementResult = {
  binary: "kubectl",
  kind: "kubectl",
  plugin: null,
  installable: true,
  status: "found",
  path: "/Users/ada/.srelens/bin/kubectl",
  version: "v1.31.4",
};

const report = (context: string, items: RequirementResult[]): { data: DiagnosisReport } => ({
  data: { context, items },
});

/** Answer each context with its own report, by name. */
function reports(byContext: Record<string, RequirementResult[]>) {
  core.diagnoseContext.mockImplementation((name: string) =>
    Promise.resolve(report(name, byContext[name] ?? [])),
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  core.isTauri.mockReturnValue(true);
  core.installPlugin.mockResolvedValue({ data: { plugin: "oidc-login", output: "installed" } });
  core.startToolInstall.mockResolvedValue({
    data: { tool: "kubectl", version: "v1.31.4", path: "/Users/ada/.srelens/bin/kubectl" },
  });
  core.toolboxStatus.mockResolvedValue({ data: [] });
  reports({});
  resetContexts();
  resetProbes();
  resetView();
});

const rail = (contexts: string[]) => render(<ExecAuthRail contexts={contexts} />);

/** The block a context's own heading owns. */
const sectionFor = (context: string) =>
  screen.getByRole("heading", { name: context }).closest("section") as HTMLElement;

const headings = () =>
  screen.queryAllByRole("heading").map((h) => h.textContent?.trim() ?? "");

/** The resolution word and the severity the pill drew it at. */
const verdict = (scope: HTMLElement) => {
  const pill = scope.querySelector(".status");
  return { word: pill?.textContent?.trim() ?? "", kind: pill?.getAttribute("data-kind") ?? "" };
};

describe("ExecAuthRail — a tool that is not there", () => {
  it("offers to install a binary srelens cannot find and can install", async () => {
    reports({ "edge-apac": [MISSING] });
    rail(["edge-apac"]);

    const section = await waitFor(() => sectionFor("edge-apac"));
    const button = within(section).getByRole("button", { name: /install kubectl-oidc_login/i });
    await userEvent.click(button);

    // The krew plugin's own name, not the binary the plugin installs as.
    expect(core.installPlugin).toHaveBeenCalledWith("oidc-login");
  });

  it("installs kubectl itself through the managed installer", async () => {
    reports({ "edge-apac": [{ ...OFF_PATH, status: "missing", path: null }] });
    rail(["edge-apac"]);

    const section = await waitFor(() => sectionFor("edge-apac"));
    await userEvent.click(within(section).getByRole("button", { name: /install kubectl/i }));

    expect(core.startToolInstall).toHaveBeenCalledWith("kubectl", expect.anything());
    expect(core.installPlugin).not.toHaveBeenCalled();
  });

  it("says so, rather than offering a button, for a tool srelens does not install", async () => {
    reports({ "edge-apac": [EXTERNAL] });
    rail(["edge-apac"]);

    const section = await waitFor(() => sectionFor("edge-apac"));
    expect(section.textContent).toContain("gke-gcloud-auth-plugin");
    expect(within(section).queryByRole("button", { name: /install/i })).toBeNull();
    expect(section.textContent).toMatch(/does not install this one/i);
  });
});

describe("ExecAuthRail — a tool that is there but not visible", () => {
  it("explains an off-PATH binary instead of offering an install that changes nothing", async () => {
    // Both cases in one render, so a rail that collapsed them has to disagree
    // with one of these two assertions.
    reports({ "edge-apac": [OFF_PATH], "prod-eu": [MISSING] });
    rail(["edge-apac", "prod-eu"]);

    const off = await waitFor(() => sectionFor("edge-apac"));
    const gone = sectionFor("prod-eu");

    // `OFF_PATH.installable` is true. Installing it again would put a second
    // copy somewhere the app already searches and leave this one where it is,
    // so the button is not drawn — the remedy is the PATH.
    expect(within(off).queryByRole("button", { name: /install/i })).toBeNull();
    expect(within(gone).getByRole("button", { name: /install/i })).toBeTruthy();

    expect(off.textContent).toMatch(/not on the PATH srelens/i);
    // The location is the actionable half, and it is on screen rather than in
    // an attribute nobody can see.
    expect(off.textContent).toContain("/opt/homebrew/bin/kubectl");

    // Two resolutions, two words. A rail that mapped both to "Missing" fails
    // here even though every sentence above it still rendered.
    expect(verdict(off).word).not.toBe(verdict(gone).word);
    expect(verdict(gone).word).toMatch(/missing/i);
  });
});

describe("ExecAuthRail — a context with nothing to report", () => {
  it("reports a context that needs no external tool as healthy, not absent", async () => {
    reports({ "prod-eu": [] });
    const { container } = rail(["prod-eu"]);

    // `ContextRequirements` calls this a healthy state in as many words: the
    // user has no exec block, so nothing external is needed.
    expect(await screen.findByText(/every context's auth resolves/i)).toBeTruthy();
    // Counted by core's `plural`, so the sentence says a check happened.
    await waitFor(() => expect(container.textContent).toContain("1 context"));
    // And no blank card for the context that had nothing to say.
    expect(headings()).not.toContain("prod-eu");
  });

  it("counts a context whose every requirement resolved as healthy too", async () => {
    reports({ "prod-eu": [FOUND] });
    rail(["prod-eu"]);

    expect(await screen.findByText(/every context's auth resolves/i)).toBeTruthy();
    expect(headings()).not.toContain("prod-eu");
  });

  it("does not claim health when there is no context to check", async () => {
    const { container } = rail([]);

    await waitFor(() => expect(container.textContent).toMatch(/no context/i));
    expect(container.textContent).not.toMatch(/every context's auth resolves/i);
    expect(core.diagnoseContext).not.toHaveBeenCalled();
  });
});

describe("ExecAuthRail — several contexts", () => {
  it("gives every context that has something to report its own section, and no others", async () => {
    reports({ "edge-apac": [MISSING], "prod-eu": [OFF_PATH], dev: [] });
    rail(["edge-apac", "prod-eu", "dev"]);

    await waitFor(() => expect(headings()).toEqual(["edge-apac", "prod-eu"]));
    expect(core.diagnoseContext).toHaveBeenCalledTimes(3);
    // Each section speaks for its own context, not for the first one answered.
    expect(sectionFor("edge-apac").textContent).toContain("kubectl-oidc_login");
    expect(sectionFor("prod-eu").textContent).not.toContain("kubectl-oidc_login");
  });

  it("counts one context's unresolved requirements, not its requirements", async () => {
    // `edge-apac` has THREE requirements and two problems. A count taken over
    // the whole list would read "3 issues" while listing two, and a count
    // taken over the rail would read "3 issues" on both sections.
    reports({ "edge-apac": [MISSING, EXTERNAL, FOUND], "prod-eu": [OFF_PATH] });
    rail(["edge-apac", "prod-eu"]);

    await waitFor(() => expect(sectionFor("edge-apac").textContent).toContain("2 issues"));
    expect(sectionFor("edge-apac").textContent).not.toContain("3 issues");
    expect(sectionFor("prod-eu").textContent).toContain("1 issue");

    // And the resolved requirement is not one of the rows being counted.
    expect(within(sectionFor("edge-apac")).queryByText("Installed")).toBeNull();
  });
});

describe("ExecAuthRail — what it does not claim", () => {
  it("states the resolution and says nothing about versions", async () => {
    // Every requirement here carries a version, INCLUDING the two the rail
    // draws. `status_fields` sends `None` for an unresolved requirement today,
    // so a fixture of realistic data would pass this test by accident — the
    // only versioned row would be the resolved one the rail filters out, and a
    // rail that printed `{binary} {version}` on every row would survive. The
    // property under test is that this rail never renders a version, whatever
    // the payload carries, because §17's sentence is what appears the moment
    // it does.
    reports({
      "edge-apac": [
        { ...MISSING, version: "v1.32.0" },
        { ...OFF_PATH, version: "v1.31.0" },
        FOUND,
      ],
    });
    const { container } = rail(["edge-apac"]);

    await waitFor(() => expect(headings()).toContain("edge-apac"));
    const text = container.textContent ?? "";

    // A kubeconfig records WHICH binary a context execs and never which
    // version, so §17's "needs kubelogin v1.32.0 · the installed v1.31.0
    // cannot refresh its token" is unbackable — every part of it.
    expect(text).not.toMatch(/v\d+\.\d+/);
    expect(text).not.toMatch(/\bversion\b/i);
    expect(text).not.toMatch(/\bupdate\b/i);

    // The resolution itself IS stated, so the assertions above are not passing
    // on an empty rail.
    expect(text).toMatch(/not on the PATH srelens/i);
    expect(verdict(sectionFor("edge-apac")).word).toBeTruthy();
  });

  it("puts no path or error in a title attribute", async () => {
    reports({ "edge-apac": [OFF_PATH] });
    const { container } = rail(["edge-apac"]);

    await waitFor(() => expect(headings()).toContain("edge-apac"));
    // The rule `PairList` and `KV` were stripped for. A resolution carries a
    // filesystem path, which is exactly the kind of value that must not hide
    // in an attribute nothing on screen announces.
    expect(container.querySelectorAll("[title]")).toHaveLength(0);
  });
});

describe("ExecAuthRail — the check itself", () => {
  it("says it is checking while the contexts are being diagnosed", async () => {
    let release: (v: { data: DiagnosisReport }) => void = () => {};
    core.diagnoseContext.mockReturnValue(
      new Promise<{ data: DiagnosisReport }>((resolve) => {
        release = resolve;
      }),
    );
    rail(["edge-apac"]);

    expect(screen.getByRole("status", { name: /checking exec auth/i })).toBeTruthy();

    release(report("edge-apac", [MISSING]));
    expect(await screen.findByRole("heading", { name: "edge-apac" })).toBeTruthy();
  });

  it("reports a context whose check failed in words, with the original folded away", async () => {
    core.diagnoseContext.mockResolvedValue({
      error: "handler error: ApiError: Unauthorized (401): Status { code: 401 }",
    });
    rail(["edge-apac"]);

    // A context that could not be checked goes to the collapsed section rather
    // than getting one of its own — but it is still NAMED there, so a reader
    // can tell whether the context they came for is among them.
    const section = await waitFor(() => sectionFor("1 context not checked"));
    expect(section.textContent).toContain("edge-apac");
    // `describeError`'s own sentence, not the struct.
    expect(section.textContent).toMatch(/rejected your credentials/i);
    // The struct is still reachable — behind a disclosure, which is the whole
    // reason it is not a `title`.
    const raw = section.querySelector('[data-slot="raw"]');
    expect(raw?.textContent).toContain("Status { code: 401 }");
  });

  it("says the same refusal once, however many contexts it arrives for", async () => {
    // Eleven contexts refusing identically is one fact, not eleven. Drawn a
    // card each they fill a 288px column and push the actionable finding off
    // the bottom — the shape that made the cluster overview unreadable when a
    // raw API error took four rows per cluster.
    core.diagnoseContext.mockResolvedValue({ error: "handler error: unknown context: x" });
    rail(["a", "b", "c"]);

    await waitFor(() => expect(headings()).toEqual(["3 contexts not checked"]));
    const section = sectionFor("3 contexts not checked");
    expect(section.textContent).toContain("a, b, c");
  });

  it("does not certify health when the check itself blew up", async () => {
    core.diagnoseContext.mockRejectedValue(new Error("boom"));
    const { container } = rail(["edge-apac"]);

    // The dangerous failure mode: with no data in hand, "every context's auth
    // resolves" is the sentence that would otherwise render.
    await waitFor(() => expect(container.textContent).toMatch(/could not check exec auth/i));
    expect(container.textContent).not.toMatch(/every context's auth resolves/i);
  });

  it("keeps one context's failure from hiding another context's answer", async () => {
    core.diagnoseContext.mockImplementation((name: string) =>
      name === "edge-apac"
        ? Promise.resolve({ error: "handler error: unknown context: edge-apac" })
        : Promise.resolve(report(name, [MISSING])),
    );
    rail(["edge-apac", "prod-eu"]);

    // The actionable context keeps its own section and its remedy; the one
    // that could not be checked is reported after it, not instead of it. That
    // ordering is the property: a failure must never cost the reader the
    // finding this rail exists to show.
    await waitFor(() => expect(headings()).toEqual(["prod-eu", "1 context not checked"]));
    expect(within(sectionFor("prod-eu")).getByRole("button", { name: /install/i })).toBeTruthy();
    expect(sectionFor("1 context not checked").textContent).toContain("edge-apac");
  });
});

describe("ExecAuthRail — where installs can run", () => {
  it("offers no install in the browser, where the capability is denied", async () => {
    core.isTauri.mockReturnValue(false);
    reports({ "edge-apac": [MISSING] });
    rail(["edge-apac"]);

    const section = await waitFor(() => sectionFor("edge-apac"));
    expect(within(section).queryByRole("button", { name: /install/i })).toBeNull();
    expect(section.textContent).toMatch(/srelens desktop app/i);
  });

  it("re-checks the context once an install has run", async () => {
    // Without this the rail keeps saying `Missing` about a binary that is now
    // on the PATH, for as long as the screen stays open — and the reader's
    // only way to find out it worked is to close the screen and come back.
    core.diagnoseContext
      .mockResolvedValueOnce(report("edge-apac", [MISSING]))
      .mockResolvedValue(
        report("edge-apac", [
          { ...MISSING, status: "found", path: "/Users/ada/.krew/bin/kubectl-oidc_login" },
        ]),
      );
    rail(["edge-apac"]);

    const section = await waitFor(() => sectionFor("edge-apac"));
    await userEvent.click(within(section).getByRole("button", { name: /install/i }));

    expect(await screen.findByText(/every context's auth resolves/i)).toBeTruthy();
    expect(core.diagnoseContext).toHaveBeenCalledTimes(2);
  });

  it("says what went wrong when an install is refused", async () => {
    core.installPlugin.mockResolvedValue({ error: "handler error: krew is not installed" });
    reports({ "edge-apac": [MISSING] });
    rail(["edge-apac"]);

    const section = await waitFor(() => sectionFor("edge-apac"));
    await userEvent.click(within(section).getByRole("button", { name: /install/i }));

    await waitFor(() =>
      expect(sectionFor("edge-apac").textContent).toMatch(/could not install kubectl-oidc_login/i),
    );
  });
});

describe("Toolbox mounts the rail", () => {
  const CTX: ClusterContext = {
    name: "prod-eu",
    stableId: "prod",
    cluster: "prod",
    server: "https://prod",
    isCurrent: true,
  };

  it("puts it beside the inventory without losing the pane head", async () => {
    setContexts([CTX]);
    store.setState(defaultState([CTX]));
    reports({ "prod-eu": [MISSING] });
    store.openTab("/toolbox");
    const { container } = render(<Toolbox route="/toolbox" />);

    // The rail is a landmark named by its own head.
    const aside = await waitFor(() =>
      screen.getByRole("complementary", { name: "Exec auth check" }),
    );
    expect((aside as HTMLElement).style.width).toBe("288px");
    // §17's pane head survives the move into `SideRail.mainHead`.
    expect(screen.getByText("Managed tools · installed under ~/.srelens/bin")).toBeTruthy();
    // And it diagnoses the contexts this window knows about.
    await waitFor(() => expect(within(aside as HTMLElement).getByRole("heading", { name: "prod-eu" })).toBeTruthy());
    expect(container.querySelectorAll("[title]")).toHaveLength(0);
  });
});
