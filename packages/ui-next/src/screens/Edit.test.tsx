import { createElement } from "react";
import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, within, act } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

/**
 * The manifest, the apply, the diff and the delete are all supplied at core's
 * boundary — the only thing this screen reads or writes. Everything in the
 * kit stays real except that `CodeEditor` is wrapped to record what it was
 * handed: CodeMirror compiles its state into a generated stylesheet jsdom
 * cannot see, so what the editor was ASKED to show, and what it says back
 * through `onChange`, are the two observable things.
 */
const core = vi.hoisted(() => ({
  getManifest: vi.fn(),
  applyManifest: vi.fn(),
  diffManifest: vi.fn(),
  validateManifest: vi.fn(),
  deleteResource: vi.fn(),
  getSecret: vi.fn(),
  listCrds: vi.fn(),
  openApiSchema: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));
vi.mock("@srelens/core/react", async (orig) => ({
  ...(await orig<typeof import("@srelens/core/react")>()),
  useNamespaceOptions: () => ({ namespaces: ["default", "payments"], scope: "", error: "" }),
}));
// The rail's cluster, mutable so a test can move it out from under an open
// editor the way a reader would.
const { active } = vi.hoisted(() => ({ active: { name: "prod-eu", stableId: "prod-eu" } }));
// The console the Review button hands the draft to.
const { ask } = vi.hoisted(() => ({ ask: vi.fn() }));
vi.mock("../console", () => ({ useConsole: () => ({ ask }) }));
vi.mock("../lib/clusters", async (orig) => ({
  ...(await orig<typeof import("../lib/clusters")>()),
  useActiveContext: () => ({ ...active }),
  getKubeconfigFiles: () => [],
}));

const { editors } = vi.hoisted(() => ({ editors: [] as Record<string, unknown>[] }));
vi.mock("@srelens/ui-kit", async (importOriginal) => {
  const actual = await importOriginal<typeof import("@srelens/ui-kit")>();
  return {
    ...actual,
    CodeEditor: (props: Record<string, unknown>) => {
      editors.push({ ...props });
      return createElement(actual.CodeEditor, props as never);
    },
  };
});

import { EditResource, TEMPLATES } from "./Edit";

// The same jsdom gaps every screen with a Radix dialog or picker stubs.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
const proto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
proto.scrollIntoView ??= () => {};
proto.hasPointerCapture ??= () => false;
proto.setPointerCapture ??= () => {};
proto.releasePointerCapture ??= () => {};

const LIVE = `apiVersion: v1
kind: ConfigMap
metadata:
  name: web
  namespace: checkout
  resourceVersion: "100"
data:
  key: value
`;
const EDITED = LIVE.replace("key: value", "key: changed");
const ROUTE = "/edit/prod-eu/ConfigMap/checkout/web";

/** The editor's last mount, and a way to type into it. */
function latestEditor() {
  const props = editors.at(-1) as {
    value: string;
    onChange: (v: string) => void;
    ariaLabel: string;
    readOnly?: boolean;
    completions?: unknown;
    onCursorChange?: (pos: number) => void;
    onDiagnostics?: (diagnostics: unknown[]) => void;
  };
  return props;
}

/** A schema bundle for the fixture's ConfigMap: enough to say what goes where. */
const SCHEMA = {
  key: "io.k8s.api.core.v1.ConfigMap",
  schemas: {
    "io.k8s.api.core.v1.ConfigMap": {
      type: "object",
      properties: {
        apiVersion: { type: "string", description: "APIVersion defines the versioned schema." },
        kind: { type: "string" },
        metadata: { allOf: [{ $ref: "#/components/schemas/ObjectMeta" }], description: "Standard object's metadata." },
        data: { type: "object", description: "Data contains the configuration data." },
        immutable: { type: "boolean", description: "Immutable, if set to true, ensures that data cannot be updated." },
      },
    },
    ObjectMeta: {
      type: "object",
      properties: {
        name: { type: "string", description: "Name must be unique within a namespace." },
        namespace: { type: "string", description: "Namespace defines the space within which each name must be unique." },
      },
    },
  },
};

beforeEach(() => {
  editors.length = 0;
  active.name = "prod-eu";
  active.stableId = "prod-eu";
  core.getManifest.mockReset().mockResolvedValue({ yaml: LIVE });
  core.applyManifest.mockReset().mockResolvedValue({
    documents: [{ kind: "ConfigMap", name: "web", applied: true }],
    applied: true,
  });
  core.diffManifest.mockReset().mockResolvedValue({ documents: [] });
  core.validateManifest.mockReset().mockResolvedValue({ valid: true, errors: [] });
  core.deleteResource.mockReset().mockResolvedValue({ ok: true });
  core.getSecret.mockReset().mockResolvedValue({ data: { password: "s3cret" } });
  core.listCrds.mockReset().mockResolvedValue({ crds: [] });
  core.openApiSchema.mockReset().mockResolvedValue(SCHEMA);
  ask.mockReset();
});

const SECRET = `apiVersion: v1
kind: Secret
metadata:
  name: db
  namespace: checkout
type: Opaque
data:
  password: czNjcmV0
`;

describe("EditResource", () => {
  it("opens on the live manifest, read from the cluster the tab is on", async () => {
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    // A built-in kind needs no CRD lookup, so none is passed.
    expect(core.getManifest).toHaveBeenCalledWith("prod-eu", "ConfigMap", "checkout", "web", undefined, undefined);
    expect(core.listCrds).not.toHaveBeenCalled();
    expect(latestEditor().ariaLabel).toBe("web manifest");
    // Nothing typed yet, so there is nothing to apply.
    expect(screen.getByRole("button", { name: "Apply" })).toHaveProperty("disabled", true);
  });

  it("applies an edited draft only after the reader confirms, and reloads on success", async () => {
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));

    act(() => latestEditor().onChange(EDITED));
    const apply = screen.getByRole("button", { name: "Apply" });
    expect(apply).toHaveProperty("disabled", false);
    await userEvent.click(apply);
    // Nothing has been written yet — the dialog names what will be.
    expect(core.applyManifest).not.toHaveBeenCalled();
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("ConfigMap");
    expect(dialog.textContent).toContain("prod-eu");

    core.getManifest.mockResolvedValue({ yaml: EDITED });
    await userEvent.click(within(dialog).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(core.applyManifest).toHaveBeenCalledWith("prod-eu", EDITED, false));
    // Applied, and the manifest re-read so the editor shows what is live now.
    expect((await screen.findByRole("status")).textContent).toContain("Applied ConfigMap web");
    await waitFor(() => expect(core.getManifest).toHaveBeenCalledTimes(2));
  });

  it("offers to force an apply another manager's field blocks", async () => {
    // Server-side apply refuses a field someone else owns rather than
    // overwriting it. The screen says whose, and the one way past it.
    core.applyManifest.mockResolvedValueOnce({
      documents: [
        {
          kind: "ConfigMap",
          name: "web",
          applied: false,
          conflict: { managers: ["helm"], fields: [".data.key"], message: "conflict" },
        },
      ],
      applied: false,
    });
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    act(() => latestEditor().onChange(EDITED));
    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Apply" }));

    const banner = await screen.findByRole("alert");
    expect(banner.textContent).toContain("helm");
    expect(banner.textContent).toContain(".data.key");
    await userEvent.click(within(banner).getByRole("button", { name: "Force apply" }));
    await waitFor(() => expect(core.applyManifest).toHaveBeenLastCalledWith("prod-eu", EDITED, true));
  });

  it("deletes only after a confirmation that names the resource and the cluster", async () => {
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    await userEvent.click(screen.getByRole("button", { name: "Delete" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("web");
    expect(dialog.textContent).toContain("prod-eu");
    expect(core.deleteResource).not.toHaveBeenCalled();

    await userEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() =>
      expect(core.deleteResource).toHaveBeenCalledWith("prod-eu", "ConfigMap", "checkout", "web"),
    );
    // The tab says what happened rather than showing an editor over nothing.
    expect(await screen.findByText(/ConfigMap web was deleted/)).toBeDefined();
  });

  it("does not call an emptied manifest applied", async () => {
    // Blank YAML applies nothing; the API answers with no documents, which is
    // neither a conflict nor a failure and used to be toasted as a success.
    core.applyManifest.mockResolvedValueOnce({ documents: [], applied: false });
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    act(() => latestEditor().onChange("   \n"));
    // Whitespace alone is not a draft worth applying.
    expect(screen.getByRole("button", { name: "Apply" })).toHaveProperty("disabled", true);
    // A comment-only document reaches the server and comes back empty.
    act(() => latestEditor().onChange("# nothing here\n"));
    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Apply" }));
    expect(await screen.findByText(/Nothing to apply/)).toBeDefined();
    expect(screen.queryByText(/^Applied /)).toBeNull();
    // The draft is kept, not cleared as an applied one would be.
    expect(core.getManifest).toHaveBeenCalledTimes(1);
  });

  it("keeps a Secret's values out of the editor until they are revealed through the gated read", async () => {
    // `k8s.getManifest` returns a Secret in the clear; the detail pane
    // redacts it and the editor has to, or one tab over undoes the gate.
    core.getManifest.mockResolvedValue({ yaml: SECRET });
    render(<EditResource route="/edit/prod-eu/Secret/checkout/db" />);
    await waitFor(() => expect(latestEditor()?.value).toBeDefined());
    const shown = latestEditor().value as string;
    expect(shown).not.toContain("czNjcmV0");
    expect(shown).toContain("REDACTED");
    expect(latestEditor().readOnly).toBe(true);
    expect(screen.getByTestId("secret-redacted")).toBeDefined();
    expect(core.getSecret).not.toHaveBeenCalled();

    await userEvent.click(screen.getByRole("button", { name: "Reveal values" }));
    await waitFor(() => expect(core.getSecret).toHaveBeenCalledWith("prod-eu", "checkout", "db"));
    await waitFor(() => expect(latestEditor().value).toBe(SECRET));
    expect(latestEditor().readOnly).toBe(false);
    expect(screen.queryByTestId("secret-redacted")).toBeNull();
  });

  it("resolves a custom kind's group before reading its manifest", async () => {
    // `k8s.getManifest` knows the built-in kinds and has to be told about any
    // other; every custom resource opened to an error until the editor asked.
    core.listCrds.mockResolvedValue({
      crds: [{ name: "widgets.acme.io", group: "acme.io", version: "v1", kind: "Widget", plural: "widgets", namespaced: true }],
    });
    core.getManifest.mockResolvedValue({ yaml: "apiVersion: acme.io/v1\nkind: Widget\nmetadata:\n  name: w1\n" });
    render(<EditResource route="/edit/prod-eu/Widget/checkout/w1" />);
    await waitFor(() => expect(latestEditor()?.value).toContain("kind: Widget"));
    expect(core.getManifest).toHaveBeenCalledWith("prod-eu", "Widget", "checkout", "w1", undefined, {
      group: "acme.io",
      version: "v1",
      plural: "widgets",
    });
    // `k8s.deleteResource` has no CRD path, so Delete on a custom resource
    // would fail every time; it is withheld, as the list menu withholds it.
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
  });

  it("keeps the cluster it opened on when the rail moves, and asks before writing there", async () => {
    const { rerender } = render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    act(() => latestEditor().onChange(EDITED));

    // The rail moves on to another cluster. The tab is not re-pointed: the
    // draft stays, nothing is re-read, and the write is still aimed at the
    // cluster the tab opened on.
    active.name = "staging";
    active.stableId = "staging";
    rerender(<EditResource route={ROUTE} />);
    expect(latestEditor().value).toBe(EDITED);
    expect(core.getManifest).toHaveBeenCalledTimes(1);

    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("still runs against prod-eu, not staging");
    // Unacknowledged, the apply is refused and the dialog stays on the question.
    await userEvent.click(within(dialog).getByRole("button", { name: "Apply" }));
    expect(core.applyManifest).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog").textContent).toContain("This runs on prod-eu, not staging");

    await userEvent.click(within(dialog).getByRole("checkbox"));
    await userEvent.click(within(dialog).getByRole("button", { name: "Apply" }));
    await waitFor(() => expect(core.applyManifest).toHaveBeenCalledWith("prod-eu", EDITED, false));
  });

  it("edits the cluster named in the route, not the one the rail is on", async () => {
    // Edit picked on staging while the rail — and perhaps another editor tab
    // for the same-named resource — is on prod-eu. The route carries the
    // cluster, so this is its own tab, pinned to staging from the first read.
    render(<EditResource route="/edit/staging/ConfigMap/checkout/web" />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    expect(core.getManifest).toHaveBeenCalledWith("staging", "ConfigMap", "checkout", "web", undefined, undefined);
    act(() => latestEditor().onChange(EDITED));
    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    const dialog = await screen.findByRole("dialog");
    expect(dialog.textContent).toContain("still runs against staging, not prod-eu");
  });

  it("says when the comparison failed, rather than showing no changes", async () => {
    // A dry run that is forbidden, or times out, used to come back as an
    // empty diff — "No changes." one click before Apply, and no "changed
    // elsewhere" check either.
    core.diffManifest.mockResolvedValue({ error: 'configmaps "web" is forbidden: cannot dry-run' });
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    act(() => latestEditor().onChange(EDITED));
    await userEvent.click(screen.getByRole("button", { name: "Diff" }));
    expect(await screen.findByText("Could not compare with the cluster", {}, { timeout: 3000 })).toBeDefined();
    expect(screen.queryByText("No changes.")).toBeNull();
    // And the header says so, whether or not the panel is open.
    expect(screen.getByText("Not compared")).toBeDefined();
  });

  it("lists the lint pass's findings beside the editor, and calls the draft valid when there are none", async () => {
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    const sidebar = screen.getByRole("complementary", { name: "Analysis" });
    // What the editor's linter found — syntax, then the server-side dry run
    // — arrives through one callback and is listed with its line.
    act(() =>
      latestEditor().onDiagnostics!([
        { from: 60, to: 70, line: 7, severity: "error", message: 'unknown field "immutble"' },
      ]),
    );
    expect(within(sidebar).getByText('unknown field "immutble"')).toBeDefined();
    expect(within(sidebar).getByText("L7")).toBeDefined();
    expect(screen.getByTestId("manifest-status").textContent).toContain("1 problem");

    act(() => latestEditor().onDiagnostics!([]));
    expect(screen.getByTestId("manifest-status").textContent).toContain("valid");
    expect(within(sidebar).getByText(/No problems/).textContent).toContain("dry run");
  });

  it("says what the schema allows where the cursor is, and completes from the same schema", async () => {
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    expect(core.openApiSchema).toHaveBeenCalledWith("prod-eu", "v1", "ConfigMap");
    const sidebar = screen.getByRole("complementary", { name: "Analysis" });
    // Cursor at the top: the top-level keys, with type and description.
    expect(await within(sidebar).findByText("immutable")).toBeDefined();
    expect(within(sidebar).getByText("Data contains the configuration data")).toBeDefined();

    // Cursor inside metadata: its keys, and the path in the heading.
    act(() => latestEditor().onCursorChange!(LIVE.indexOf("  name: web") + 4));
    expect(await within(sidebar).findByText("namespace")).toBeDefined();
    expect(within(sidebar).getByText("metadata")).toBeDefined();
    expect(within(sidebar).queryByText("immutable")).toBeNull();

    // The editor's completion popup is fed from the same schema.
    expect(typeof latestEditor().completions).toBe("function");
  });

  it("runs a dry run on demand, shows the verdict, and retires it when the draft changes", async () => {
    core.validateManifest.mockResolvedValue({
      valid: false,
      errors: [{ docIndex: 0, message: "metadata.name: Invalid value: too long" }],
    });
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    await userEvent.click(screen.getByRole("button", { name: "Dry run" }));
    const verdict = await screen.findByTestId("dry-run-verdict");
    expect(verdict.textContent).toContain("Dry run failed");
    expect(verdict.textContent).toContain("Invalid value: too long");
    expect(core.validateManifest).toHaveBeenCalledWith("prod-eu", LIVE);

    // The verdict was about that text; typing makes it stale, so it goes.
    act(() => latestEditor().onChange(EDITED));
    expect(screen.queryByTestId("dry-run-verdict")).toBeNull();

    core.validateManifest.mockResolvedValue({ valid: true, errors: [] });
    await userEvent.click(screen.getByRole("button", { name: "Dry run" }));
    expect((await screen.findByTestId("dry-run-verdict")).textContent).toContain("Dry run passed");
  });

  it("hands the draft to the assistant for review, naming the cluster it is bound for", async () => {
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    act(() => latestEditor().onChange(EDITED));
    await userEvent.click(screen.getByRole("button", { name: "Review" }));
    expect(ask).toHaveBeenCalledTimes(1);
    const question = ask.mock.calls[0][0] as string;
    expect(question).toContain("ConfigMap web");
    expect(question).toContain("prod-eu");
    expect(question).toContain("key: changed");
  });

  it("reverts the draft to the live manifest", async () => {
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    act(() => latestEditor().onChange(EDITED));
    expect(latestEditor().value).toBe(EDITED);
    await userEvent.click(screen.getByRole("button", { name: "Revert" }));
    await waitFor(() => expect(latestEditor().value).toBe(LIVE));
    expect(core.getManifest).toHaveBeenCalledTimes(2);
  });

  it("takes a conflict down once the draft that caused it changes", async () => {
    core.applyManifest.mockResolvedValueOnce({
      documents: [
        {
          kind: "ConfigMap",
          name: "web",
          applied: false,
          conflict: { managers: ["helm"], fields: [".data.key"], message: "conflict" },
        },
      ],
      applied: false,
    });
    render(<EditResource route={ROUTE} />);
    await waitFor(() => expect(latestEditor()?.value).toBe(LIVE));
    act(() => latestEditor().onChange(EDITED));
    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Apply" }));
    await screen.findByRole("alert");

    // Force applies what is in the editor, so a banner left over an edited
    // draft would force text no plain apply had described. Typing takes it
    // down, Force with it.
    act(() => latestEditor().onChange(EDITED.replace("changed", "changed-again")));
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("button", { name: "Force apply" })).toBeNull();
    // Back to the conflicted text, and the banner is back: it is that text's.
    act(() => latestEditor().onChange(EDITED));
    expect(screen.getByRole("alert").textContent).toContain("helm");
  });

  it("opens a custom kind that shares a built-in's name under its own group, not as the built-in", async () => {
    core.listCrds.mockResolvedValue({
      crds: [
        { name: "deployments.acme.io", group: "acme.io", version: "v1", kind: "Deployment", plural: "deployments", namespaced: true },
      ],
    });
    core.getManifest.mockResolvedValue({ yaml: "apiVersion: acme.io/v1\nkind: Deployment\nmetadata:\n  name: api\n" });
    render(<EditResource route="/edit/prod-eu/acme.io%2FDeployment/checkout/api" />);
    await waitFor(() => expect(latestEditor()?.value).toContain("acme.io/v1"));
    expect(core.getManifest).toHaveBeenCalledWith("prod-eu", "Deployment", "checkout", "api", undefined, {
      group: "acme.io",
      version: "v1",
      plural: "deployments",
    });
    // And no Delete: `k8s.deleteResource` resolves by kind alone, which here
    // would be the built-in Deployment of the same name.
    expect(screen.queryByRole("button", { name: "Delete" })).toBeNull();
  });

  it("says when the manifest could not be read, instead of an empty editor", async () => {
    core.getManifest.mockResolvedValue({ error: 'configmaps "web" is forbidden' });
    render(<EditResource route={ROUTE} />);
    expect(await screen.findByText(/Could not load ConfigMap web/)).toBeDefined();
    expect(editors).toHaveLength(0);
  });

  it("refuses a route that does not name a resource", () => {
    render(<EditResource route="/edit/web" />);
    expect(screen.getByText(/does not name a resource/)).toBeDefined();
  });
});

describe("EditResource on /new", () => {
  it("creates on the cluster named in the route, and asks first when the rail is elsewhere", async () => {
    // New pressed on staging's list opens /new/staging; the rail is on
    // prod-eu. The draft is staging's, and the gate says so before Create.
    render(<EditResource route="/new/staging" />);
    await waitFor(() => expect(latestEditor()?.value).toBe(TEMPLATES.Deployment("default")));
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    expect(core.applyManifest).not.toHaveBeenCalled();
    expect(screen.getByText(/still runs against staging, not prod-eu/)).toBeDefined();
    await userEvent.click(screen.getByRole("checkbox"));
    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() =>
      expect(core.applyManifest).toHaveBeenCalledWith("staging", TEMPLATES.Deployment("default"), false),
    );
  });

  it("starts from a template in the picked namespace, and creates with one click", async () => {
    render(<EditResource route="/new" />);
    await waitFor(() => expect(latestEditor()?.value).toBe(TEMPLATES.Deployment("default")));
    expect(screen.getByRole("heading", { name: "New resource" })).toBeDefined();

    await userEvent.click(screen.getByRole("button", { name: "Create" }));
    await waitFor(() =>
      expect(core.applyManifest).toHaveBeenCalledWith("prod-eu", TEMPLATES.Deployment("default"), false),
    );
    expect((await screen.findByRole("status")).textContent).toContain("Created ConfigMap web");
  });

  it("swaps the draft when a different template is picked", async () => {
    render(<EditResource route="/new" />);
    await waitFor(() => expect(latestEditor()?.value).toBe(TEMPLATES.Deployment("default")));
    // A native select, so picked with `selectOptions` rather than two clicks.
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Template" }), "ConfigMap");
    await waitFor(() => expect(latestEditor()?.value).toBe(TEMPLATES.ConfigMap("default")));
    // And the namespace follows the picker while the draft is still the
    // template's.
    await userEvent.selectOptions(screen.getByRole("combobox", { name: "Namespace" }), "payments");
    await waitFor(() => expect(latestEditor()?.value).toBe(TEMPLATES.ConfigMap("payments")));
  });
});
