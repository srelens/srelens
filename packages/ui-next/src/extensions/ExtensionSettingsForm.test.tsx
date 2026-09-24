import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  listContexts: vi.fn(),
  listNamespaces: vi.fn(),
}));
import { listContexts, listNamespaces, type ExtensionSetting, type InstalledExtension } from "@srelens/core";
import { ExtensionSettingsForm } from "./ExtensionSettingsForm";

// The pickers' popover watches its trigger, and cmdk scrolls the highlighted row into view.
if (!("ResizeObserver" in globalThis)) {
  (globalThis as unknown as { ResizeObserver: unknown }).ResizeObserver = class {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}
const elementProto = window.HTMLElement.prototype as unknown as Record<string, unknown>;
elementProto.scrollIntoView ??= () => {};
elementProto.hasPointerCapture ??= () => false;
elementProto.setPointerCapture ??= () => {};
elementProto.releasePointerCapture ??= () => {};

const every: ExtensionSetting[] = [
  { id: "note", type: "string", title: "Note", maxLength: 40, description: "Shown on the dashboard." },
  { id: "expiryWindowDays", type: "number", title: "Warn before expiry (days)", default: 14, minimum: 1, integer: true },
  { id: "verbose", type: "boolean", title: "Verbose", default: false },
  { id: "mode", type: "select", title: "Refresh", default: "normal",
    options: [{ value: "normal", label: "Normal" }, { value: "hard", label: "Hard" }] },
  { id: "kinds", type: "multi-select", title: "Kinds", options: [{ value: "a", label: "Alpha" }, { value: "b", label: "Beta" }] },
  { id: "prometheusUrl", type: "url", title: "Prometheus URL", required: true },
  { id: "namespace", type: "namespace-selector", title: "Default namespace" },
  { id: "cluster", type: "cluster-selector", title: "Home cluster" },
  { id: "token", type: "secret-reference", title: "API token" },
];

function app(settings: ExtensionSetting[] = every, saved: Record<string, unknown> = {}): InstalledExtension {
  return {
    manifest: {
      id: "org.example.certs", name: "Certificates", version: "1.0.0", srelensApiVersion: "^0.3", kind: "declarative",
      permissions: [], capabilities: [], settings, contributions: { pages: [], detailTabs: [], detailLinks: [] },
    },
    enabled: true, revision: 3, grants: [], settings: saved, source: "local", installedAt: 0, history: [],
  };
}

const contexts = [
  { name: "prod", stableId: "/kube/config#prod", key: "/kube/config#prod" },
  { name: "staging", stableId: "/kube/config#staging", key: "/kube/config#staging" },
];

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(listContexts).mockResolvedValue({ contexts } as any);
  vi.mocked(listNamespaces).mockResolvedValue({ namespaces: ["cert-manager", "team-a"], summaries: [] } as any);
  // Opening a picker makes Radix measure ranges, and jsdom has no layout.
  Range.prototype.getClientRects = () => [] as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () =>
    ({ x: 0, y: 0, width: 0, height: 0, top: 0, right: 0, bottom: 0, left: 0, toJSON: () => ({}) }) as DOMRect;
});

function form(plugin = app(), onSave = vi.fn().mockResolvedValue(undefined)) {
  render(<ExtensionSettingsForm plugin={plugin} onSave={onSave} onClose={() => {}} />);
  return { onSave, form: screen.getByRole("form", { name: "Certificates settings" }) };
}

it("draws a field for every declared setting type", async () => {
  const { form: region } = form();
  expect(within(region).getByRole("textbox", { name: "Note" })).toBeTruthy();
  expect(within(region).getByRole("spinbutton", { name: "Warn before expiry (days)" })).toBeTruthy();
  expect(within(region).getByRole("checkbox", { name: "Verbose" })).toBeTruthy();
  expect(within(region).getByRole("combobox", { name: "Refresh" })).toBeTruthy();
  const kinds = within(region).getByRole("group", { name: "Kinds" });
  expect(within(kinds).getByRole("checkbox", { name: "Alpha" })).toBeTruthy();
  expect(within(kinds).getByRole("checkbox", { name: "Beta" })).toBeTruthy();
  const url = within(region).getByRole("textbox", { name: /Prometheus URL/ });
  expect(url.getAttribute("aria-required")).toBe("true");
  expect(within(region).getByRole("combobox", { name: "Home cluster" })).toBeTruthy();
  expect(within(region).getByRole("group", { name: "Default namespace" })).toBeTruthy();
  // A secret has no input: its value never goes through settings.
  const secret = within(region).getByRole("group", { name: "API token" });
  expect(within(secret).queryByRole("textbox")).toBeNull();
  expect(secret.textContent).toContain("Not set");
  // Help text and defaults are shown, and a default is not pretended to be saved.
  expect(region.textContent).toContain("Shown on the dashboard.");
  expect(region.textContent).toContain("Default: 14");
  expect((within(region).getByRole("spinbutton", { name: "Warn before expiry (days)" }) as HTMLInputElement).value).toBe("");
});

it("draws app text as plain text, never markup or reordering characters", () => {
  const hostile = "Token ‮gnp.exe <b>bold</b>";
  const { form: region } = form(app([{ id: "note", type: "select", title: hostile, description: "<img src=x onerror=alert(1)>",
    options: [{ value: "a", label: "<i>label</i>​" }] }]));
  expect(region.querySelector("b, img, i")).toBeNull();
  expect(region.textContent).not.toContain("‮");
  expect(region.textContent).toContain("<b>bold</b>");
  expect(region.textContent).toContain("\\u202e");
  expect(region.textContent).toContain("<img src=x onerror=alert(1)>");
});

it("saves typed values, and leaves out what was not set", async () => {
  const { form: region, onSave } = form();
  fireEvent.change(within(region).getByRole("textbox", { name: "Note" }), { target: { value: "hello" } });
  fireEvent.change(within(region).getByRole("spinbutton", { name: "Warn before expiry (days)" }), { target: { value: "30" } });
  fireEvent.click(within(region).getByRole("checkbox", { name: "Verbose" }));
  fireEvent.click(within(within(region).getByRole("group", { name: "Kinds" })).getByRole("checkbox", { name: "Beta" }));
  fireEvent.change(within(region).getByRole("textbox", { name: /Prometheus URL/ }), { target: { value: "https://prom:9090" } });
  fireEvent.click(within(region).getByRole("combobox", { name: "Refresh" }));
  fireEvent.click(await screen.findByRole("option", { name: "Hard" }));
  fireEvent.click(within(region).getByRole("combobox", { name: "Home cluster" }));
  fireEvent.click(await screen.findByRole("option", { name: "staging" }));
  fireEvent.click(within(region).getByRole("button", { name: "Save settings" }));
  await waitFor(() => expect(onSave).toHaveBeenCalledTimes(1));
  expect(onSave).toHaveBeenCalledWith({
    note: "hello",
    expiryWindowDays: 30,
    verbose: true,
    mode: "hard",
    kinds: ["b"],
    prometheusUrl: "https://prom:9090",
    // The context's key, never its display name (#265).
    cluster: "/kube/config#staging",
  });
});

it("picks a namespace from a chosen cluster's namespaces", async () => {
  const { form: region, onSave } = form(app(every, { prometheusUrl: "https://prom" }));
  const field = within(region).getByRole("group", { name: "Default namespace" });
  fireEvent.click(within(field).getByRole("combobox", { name: "Cluster to list namespaces from" }));
  fireEvent.click(await screen.findByRole("option", { name: "prod" }));
  await waitFor(() => expect(listNamespaces).toHaveBeenCalledWith("/kube/config#prod"));
  fireEvent.click(within(field).getByRole("combobox", { name: "Default namespace" }));
  fireEvent.click(await screen.findByRole("option", { name: "team-a" }));
  fireEvent.click(within(region).getByRole("button", { name: "Save settings" }));
  await waitFor(() =>
    expect(onSave).toHaveBeenCalledWith({ prometheusUrl: "https://prom", namespace: "team-a" }),
  );
});

it("says why namespaces could not be listed, with a retry, rather than offering none", async () => {
  vi.mocked(listNamespaces).mockResolvedValue({ error: "forbidden: cannot list namespaces" } as any);
  const { form: region } = form();
  const field = within(region).getByRole("group", { name: "Default namespace" });
  fireEvent.click(within(field).getByRole("combobox", { name: "Cluster to list namespaces from" }));
  fireEvent.click(await screen.findByRole("option", { name: "prod" }));
  expect(await within(field).findByText(/forbidden: cannot list namespaces/)).toBeTruthy();
  vi.mocked(listNamespaces).mockResolvedValue({ namespaces: ["team-a"], summaries: [] } as any);
  fireEvent.click(within(field).getByRole("button", { name: "Retry" }));
  await waitFor(() => expect(listNamespaces).toHaveBeenCalledTimes(2));
});

it("shows saved values and keeps a secret's reference out of the payload", async () => {
  const saved = { note: "kept", expiryWindowDays: 7, prometheusUrl: "https://prom", token: { secretRef: "org.example.certs/token" } };
  const { form: region, onSave } = form(app(every, saved));
  expect((within(region).getByRole("textbox", { name: "Note" }) as HTMLInputElement).value).toBe("kept");
  expect((within(region).getByRole("spinbutton", { name: "Warn before expiry (days)" }) as HTMLInputElement).value).toBe("7");
  const secret = within(region).getByRole("group", { name: "API token" }).textContent!;
  expect(secret).toContain("Set");
  expect(secret).not.toContain("Not set");
  // A secret that is set is not also described as one this version cannot store.
  expect(secret).not.toContain("cannot store");
  fireEvent.click(within(region).getByRole("button", { name: "Save settings" }));
  await waitFor(() =>
    expect(onSave).toHaveBeenCalledWith({ note: "kept", expiryWindowDays: 7, prometheusUrl: "https://prom" }),
  );
});

it("asks for a required value and a number before saving", async () => {
  const { form: region, onSave } = form();
  fireEvent.change(within(region).getByRole("spinbutton", { name: "Warn before expiry (days)" }), { target: { value: "1.5" } });
  fireEvent.click(within(region).getByRole("button", { name: "Save settings" }));
  const url = within(region).getByRole("textbox", { name: /Prometheus URL/ });
  expect(url.getAttribute("aria-invalid")).toBe("true");
  expect(document.getElementById(url.getAttribute("aria-describedby")!.split(" ").pop()!)?.textContent).toContain("Required");
  const days = within(region).getByRole("spinbutton", { name: "Warn before expiry (days)" });
  expect(days.getAttribute("aria-invalid")).toBe("true");
  expect(onSave).not.toHaveBeenCalled();
});

it("puts each host refusal beside its field, and the rest above the form", async () => {
  const onSave = vi.fn().mockRejectedValue(
    new Error(
      "settings.prometheusUrl: Must be an http or https URL with a host (EXTENSION_INVALID_VALUE)\n" +
        "actions[0].arguments: `$bogus` is not a value this host substitutes (EXTENSION_INVALID_BINDING)",
    ),
  );
  const { form: region } = form(app(every, { prometheusUrl: "https://prom" }), onSave);
  fireEvent.click(within(region).getByRole("button", { name: "Save settings" }));
  const url = within(region).getByRole("textbox", { name: /Prometheus URL/ });
  await waitFor(() => expect(url.getAttribute("aria-invalid")).toBe("true"));
  const described = url.getAttribute("aria-describedby")!.split(" ").map((id) => document.getElementById(id)?.textContent).join(" ");
  expect(described).toContain("Must be an http or https URL with a host");
  const alert = within(region).getByRole("alert");
  expect(alert.textContent).toContain("is not a value this host substitutes");
  expect(alert.textContent).not.toContain("Must be an http");
});

it("says so when an app declares no settings", () => {
  form(app([]));
  expect(screen.getByText("This app declares no settings.")).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Save settings" })).toBeNull();
});
