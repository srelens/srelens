import { render, screen, fireEvent, waitFor, within } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
vi.mock("@srelens/core", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@srelens/core")>()),
  isTauri: () => true,
  listExtensionCatalog: vi.fn(),
  reviewCatalogExtension: vi.fn(),
  listExtensions: vi.fn(),
  configureExtensions: vi.fn(),
  validateExtension: vi.fn(),
  listContexts: vi.fn(),
}));
import {
  configureExtensions,
  listContexts,
  listExtensionCatalog,
  listExtensions,
  reviewCatalogExtension,
  validateExtension,
} from "@srelens/core";
import { ExtensionManager } from "./Extensions";
import { plainText } from "./displayText";

// Written by code point, so the source itself holds no invisible character.
const RLO = String.fromCodePoint(0x202e);
const TAG_A = String.fromCodePoint(0xe0041);
/** What the review draws for these UTF-16 units: one JSON escape each. */
const escapes = (...units: number[]) =>
  units.map((unit) => `\\u${unit.toString(16).padStart(4, "0")}`).join("");

/** Two custom-resource readers and an event reader its dashboard filters by API group. */
const manifest = () => ({
  id: "org.test.flux",
  name: "Flux test",
  version: "0.1.0",
  srelensApiVersion: "^0.1",
  kind: "declarative",
  permissions: ["k8s.listCustomResource", "k8s.listEvents"],
  capabilities: [
    {
      name: "kustomizations",
      title: "List Kustomizations",
      target: "k8s.listCustomResource",
      arguments: {
        group: "kustomize.toolkit.fluxcd.io",
        version: "v1",
        plural: "kustomizations",
        kind: "Kustomization",
        namespaced: true,
        printerColumns: [
          { name: "Ready", jsonPath: '.status.conditions[?(@.type=="Ready")].status', type: "string" },
          { name: "Revision", jsonPath: ".status.lastAppliedRevision", type: "string" },
        ],
      },
      inputs: ["context", "namespace"],
    },
    {
      name: "providers",
      title: "List providers",
      target: "k8s.listCustomResource",
      arguments: {
        group: "infra.example.io",
        version: "v1beta2",
        plural: "providers",
        kind: "Provider",
        namespaced: false,
        printerColumns: [{ name: "Region", jsonPath: ".spec.region", type: "string" }],
      },
      inputs: ["context"],
    },
    { name: "events", title: "List events", target: "k8s.listEvents", arguments: {}, inputs: ["context", "namespace"] },
  ],
  contributions: {
    pages: [
      {
        id: "overview",
        title: "Overview",
        capability: "kustomizations",
        dashboard: {
          pages: ["kustomizations"],
          events: { capability: "events", apiGroups: ["kustomize.toolkit.fluxcd.io", "infra.example.io"] },
        },
      },
      { id: "kustomizations", title: "Kustomizations", capability: "kustomizations", statusColumns: { ready: 0 } },
    ],
    detailTabs: [],
    detailLinks: [],
  },
});

const catalog = {
  catalog: {
    extensions: [
      {
        id: "org.test.flux",
        name: "Catalog Flux",
        description: "Flux resources",
        repository: "https://github.com/example/flux",
        license: "MIT",
        release: { version: "0.1.0", sha256: "digest", srelensApiVersion: "^0.1", prerelease: false },
      },
    ],
  },
  fetchedAt: 1,
  stale: false,
  error: null,
  hostApiVersions: ["0.1.0"],
  incompatible: [],
};

beforeEach(() => {
  vi.resetAllMocks();
  vi.mocked(listExtensions).mockResolvedValue({ schemaVersion: 1, nextRevision: 1, plugins: [] } as any);
  vi.mocked(configureExtensions).mockResolvedValue({} as any);
  vi.mocked(validateExtension).mockResolvedValue({ errors: [], permissionDiff: { previousRevision: null, added: ["Grant k8s.listCustomResource", "Grant k8s.listEvents"], removed: [], unchanged: [] } });
  vi.mocked(listContexts).mockResolvedValue({ contexts: [] });
  vi.mocked(listExtensionCatalog).mockResolvedValue(catalog as any);
});

async function reviewFromCatalog(source: string) {
  vi.mocked(reviewCatalogExtension).mockResolvedValue({ manifest: source, signature: [1, 2, 3] });
  render(<ExtensionManager />);
  fireEvent.click(await screen.findByRole("tab", { name: "Catalog" }));
  fireEvent.click(await screen.findByText("Review installation"));
  await screen.findByText("Install and grant permissions");
  return screen.getByRole("region", { name: "Review app permissions" });
}

async function reviewPasted(source: string) {
  render(<ExtensionManager />);
  fireEvent.change(await screen.findByLabelText("Local app manifest (JSON)"), { target: { value: source } });
  fireEvent.click(screen.getByText("Review manifest"));
  await screen.findByText("Install and grant permissions");
  return screen.getByRole("region", { name: "Review app permissions" });
}

/** The data cells of a row, as text. */
const cells = (row: HTMLElement) => within(row).getAllByRole("cell").map((cell) => cell.textContent);

/** A reader's row: its title, then group, version, kind, plural and scope. */
const reader = (review: HTMLElement, name: string) =>
  within(within(review).getByRole("table", { name: "Custom resources read" })).getByRole("row", {
    name: `Binding ${name}`,
  });

/** Opens a reader's printer columns and returns them as [column, JSON path] rows. */
function columns(row: HTMLElement, name: string) {
  const details = row.querySelector("details")!;
  expect(details.open).toBe(false);
  fireEvent.click(details.querySelector("summary")!);
  expect(details.open).toBe(true);
  const table = within(row).getByRole("table", { name: `${name} printer columns` });
  return within(table)
    .getAllByRole("row")
    .slice(1)
    .map((line) => within(line).getAllByRole("cell").map((cell) => cell.textContent));
}

/** The summary both install flows show for `manifest()`. */
function expectBindings(review: HTMLElement) {
  within(review).getByRole("list", { name: "Permission bindings" });

  const kustomizations = reader(review, "kustomizations");
  expect(within(kustomizations).getByRole("rowheader").textContent).toBe("List Kustomizations");
  expect(cells(kustomizations).slice(0, 5)).toEqual([
    "kustomize.toolkit.fluxcd.io",
    "v1",
    "Kustomization",
    "kustomizations",
    "Namespaced",
  ]);
  // The column names are on the row; their JSON paths open from it.
  expect(kustomizations.querySelector("summary")!.textContent).toBe("Ready, Revision");
  expect(columns(kustomizations, "kustomizations")).toEqual([
    ["Ready", '.status.conditions[?(@.type=="Ready")].status'],
    ["Revision", ".status.lastAppliedRevision"],
  ]);

  const providers = reader(review, "providers");
  expect(cells(providers).slice(0, 5)).toEqual(["infra.example.io", "v1beta2", "Provider", "providers", "Cluster-wide"]);
  expect(columns(providers, "providers")).toEqual([["Region", ".spec.region"]]);

  // The event reader says what it reads and which API groups the dashboard shows.
  const events = within(review).getByRole("listitem", { name: "Binding events" });
  expect(events.textContent).toContain("List events: reads the events of the namespace in view");
  expect(events.textContent).toContain(
    "Overview shows only those for API groups kustomize.toolkit.fluxcd.io, infra.example.io.",
  );
}

it("summarizes a catalog app's bindings and opens its manifest before Install is pressed", async () => {
  const source = JSON.stringify(manifest(), null, 2);
  const review = await reviewFromCatalog(source);
  expectBindings(review);

  expect(within(review).queryByRole("textbox", { name: "Manifest under review" })).toBeNull();
  fireEvent.click(within(review).getByRole("button", { name: "View manifest" }));
  const text = within(review).getByRole("textbox", { name: "Manifest under review" });
  await waitFor(() => expect(text.textContent).toContain('"plural": "kustomizations"'));
  expect(text.textContent).toContain('"group": "kustomize.toolkit.fluxcd.io"');
  // Something a reader may well want out of the app before deciding, and the
  // select-all chord alone is an affordance with nothing to see. (#656 review)
  expect(within(review).getByRole("button", { name: "Copy" })).toBeTruthy();
  expect(configureExtensions).not.toHaveBeenCalled();

  fireEvent.click(screen.getByText("Install and grant permissions"));
  await waitFor(() =>
    expect(configureExtensions).toHaveBeenCalledWith({
      action: "install",
      manifest: source,
      signature: [1, 2, 3],
      grants: ["k8s.listCustomResource", "k8s.listEvents"],
    }),
  );
});

it("summarizes a pasted manifest's bindings the same way and shows it indented", async () => {
  // Pasted on one line; the review shows it the way Details will once installed.
  const review = await reviewPasted(JSON.stringify(manifest()));
  expectBindings(review);
  fireEvent.click(within(review).getByRole("button", { name: "View manifest" }));
  const text = within(review).getByRole("textbox", { name: "Manifest under review" });
  await waitFor(() => expect(text.textContent).toContain('"kind": "Kustomization"'));
  expect(configureExtensions).not.toHaveBeenCalled();
});

it("escapes format characters in a binding's column name, path and in the manifest", async () => {
  const spoofing = manifest();
  // A right-to-left override would reorder the review around the column's name, and a
  // tag character (above U+FFFF) is invisible.
  (spoofing.capabilities[0].arguments as any).printerColumns[0] = {
    name: `Rea${RLO}dy`,
    jsonPath: `.status.${TAG_A}phase`,
    type: "string",
  };
  const review = await reviewFromCatalog(JSON.stringify(spoofing));
  const row = reader(review, "kustomizations");
  expect(row.querySelector("summary")!.textContent).toBe(`Rea${escapes(0x202e)}dy, Revision`);
  expect(columns(row, "kustomizations")[0]).toEqual([
    `Rea${escapes(0x202e)}dy`,
    `.status.${escapes(0xdb40, 0xdc41)}phase`,
  ]);
  expect(review.textContent).not.toMatch(/\p{Cf}/u);

  fireEvent.click(within(review).getByRole("button", { name: "View manifest" }));
  const text = within(review).getByRole("textbox", { name: "Manifest under review" });
  await waitFor(() => expect(text.textContent).toContain(`"name": "Rea${escapes(0x202e)}dy"`));
  expect(text.textContent).toContain(escapes(0xdb40, 0xdc41));
  expect(document.body.textContent).not.toMatch(/\p{Cf}/u);
});

it("summarizes no bindings until the host accepts the manifest, but shows its text", async () => {
  vi.mocked(validateExtension).mockResolvedValue({
    errors: [{ code: "EXTENSION_INVALID_BINDING", path: "capabilities[0].arguments.group", message: "Bind a custom-resource group" }],
  });
  render(<ExtensionManager />);
  fireEvent.change(await screen.findByLabelText("Local app manifest (JSON)"), {
    target: { value: JSON.stringify(manifest()) },
  });
  fireEvent.click(screen.getByText("Review manifest"));
  await screen.findByRole("list", { name: "Manifest problems" });
  const review = screen.getByRole("region", { name: "Review app permissions" });
  expect(within(review).queryByRole("list", { name: "Permission bindings" })).toBeNull();
  fireEvent.click(within(review).getByRole("button", { name: "View manifest" }));
  expect(within(review).getByRole("textbox", { name: "Manifest under review" })).toBeTruthy();
});

it("lists another capability's fixed arguments and inputs generically", async () => {
  const other = manifest();
  other.permissions = ["k8s.listWidgets"];
  other.capabilities = [
    { name: "widgets", title: "List widgets", target: "k8s.listWidgets", arguments: { limit: 5, selector: "tier=web" }, inputs: ["context"] } as any,
  ];
  const review = await reviewPasted(JSON.stringify(other));
  const widgets = within(review).getByRole("listitem", { name: "Binding widgets" });
  expect(widgets.textContent).toBe(
    "List widgets: fixed arguments limit 5, selector tier=web; takes context from the view.",
  );
});

it("reviews the secret store as a permission: what it keeps, and the host's own words for it (#543)", async () => {
  const keeping = manifest() as ReturnType<typeof manifest> & { settings?: unknown };
  keeping.permissions = [...keeping.permissions, "extension.secretStore"];
  keeping.settings = [
    { id: "token", type: "secret-reference", title: "API token" },
    { id: "hook", type: "secret-reference", title: `Webhook ${RLO}terces` },
    { id: "team", type: "string", title: "Team" },
  ];
  const review = await reviewPasted(JSON.stringify(keeping));
  const store = within(review).getByRole("listitem", { name: "extension.secretStore bindings" });
  expect(store.textContent).not.toContain("No binding uses this permission");
  expect(store.textContent).toContain("Keeps these secret settings in srelens's encrypted secrets vault: API token, Webhook");
  expect(store.textContent).not.toContain(RLO);
  expect(store.textContent).not.toContain("Team");
  expect(store.textContent).toContain("The app never reads them");
  // #548's host metadata, from the catalog, never from the manifest.
  expect(store.textContent).toContain("Sensitive");
  expect(store.textContent).toContain("medium impact");
  expect(store.textContent).toContain("Change a secret an app keeps in srelens's secrets vault");
});
it("draws inline manifest text with invisible and control characters escaped", () => {
  const zeroWidth = String.fromCodePoint(0x200b);
  const tag = String.fromCodePoint(0xe0001);
  expect(plainText(`a${RLO}b${zeroWidth}c\nd${tag}`)).toBe(
    `a${escapes(0x202e)}b${escapes(0x200b)}c${escapes(0x0a)}d${escapes(0xdb40, 0xdc01)}`,
  );
  expect(plainText('.status.conditions[?(@.type=="Ready")]')).toBe('.status.conditions[?(@.type=="Ready")]');
});

it("says what a binding leaves unset rather than drawing blanks", async () => {
  const sparse = manifest();
  sparse.permissions = ["k8s.listCustomResource", "k8s.listEvents", "k8s.listNodes"];
  sparse.capabilities = [
    { name: "loose", title: "", target: "k8s.listCustomResource", arguments: { group: "a.example.io", version: 2 }, inputs: [] } as any,
    { name: "events", title: "List events", target: "k8s.listEvents", arguments: {}, inputs: ["context", "namespace"] },
  ];
  sparse.contributions.pages = [];
  const review = await reviewPasted(JSON.stringify(sparse));
  const loose = reader(review, "loose");
  // Untitled, it is named by its binding name. A non-string value is drawn as its JSON, and
  // a missing one is named as missing.
  expect(within(loose).getByRole("rowheader").textContent).toBe("loose");
  expect(cells(loose)).toEqual(["a.example.io", "2", "Not set", "Not set", "Not set", "None"]);
  const events = within(review).getByRole("listitem", { name: "Binding events" });
  expect(events.textContent).toContain("No dashboard shows them.");
  expect(events.textContent).not.toContain("Fixed arguments");
  const nodes = within(review).getByRole("listitem", { name: "k8s.listNodes bindings" });
  expect(within(nodes).getByText("No binding uses this permission.")).toBeTruthy();
});

it("lists a custom-resource reader's other fixed arguments in their own column", async () => {
  const extra = manifest();
  (extra.capabilities[1].arguments as any).labelSelector = "team=platform";
  const review = await reviewPasted(JSON.stringify(extra));
  expect(within(review).getByRole("columnheader", { name: "Other fixed arguments" })).toBeTruthy();
  expect(cells(reader(review, "providers")).at(-1)).toBe("labelSelector team=platform");
  expect(cells(reader(review, "kustomizations")).at(-1)).toBe("none");
});
