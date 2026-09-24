import { describe, expect, it } from "vitest";
import inventorySchema from "./extension-inventory.schema.json";
import type { ActionPredicate } from "./actionPredicates";
import type { ExtensionInventory, ExtensionLinkRelation, ExtensionManifest, ExtensionStatusRule, InstalledExtension, NormalizedStatus } from "./extensions";

// extension-inventory.schema.json is generated from the Rust inventory and manifest
// types (crates/registry/src/extensions.rs keeps it current). Each table below is held
// to both sides: `tsc` forces it to list exactly the TypeScript type's fields and which
// are optional, and this test forces it to equal the Rust type's. A field renamed on one
// side only fails CI.
type Presence<T> = { [K in keyof T]-?: {} extends Pick<T, K> ? "optional" : "required" };

type Contributions = ExtensionManifest["contributions"];
type Page = Contributions["pages"][number];
type Dashboard = NonNullable<Page["dashboard"]>;
type Card = NonNullable<Contributions["dashboardCards"]>[number];
type Setting = NonNullable<ExtensionManifest["settings"]>[number];

const tables: Record<string, Record<string, "required" | "optional">> = {
  Inventory: {
    allowUnsignedApps: "optional",
    schemaVersion: "required",
    nextRevision: "required",
    plugins: "required",
  } satisfies Presence<ExtensionInventory>,
  Installed: {
    signatureProof: "optional",
    quarantined: "optional",
    policyBlocked: "optional",
    manifest: "required",
    grants: "required",
    enabled: "required",
    revision: "required",
    settings: "required",
    source: "required",
    installedAt: "required",
    history: "required",
    contexts: "optional",
  } satisfies Presence<InstalledExtension>,
  PreviousVersion: {
    signatureProof: "optional",
    manifest: "required",
    grants: "required",
    revision: "required",
    source: "required",
    installedAt: "required",
  } satisfies Presence<InstalledExtension["history"][number]>,
  SignatureProof: {
    manifest: "required",
    signature: "required",
  } satisfies Presence<NonNullable<InstalledExtension["signatureProof"]>>,
  Manifest: {
    $schema: "optional",
    id: "required",
    name: "required",
    version: "required",
    srelensApiVersion: "required",
    kind: "required",
    permissions: "required",
    capabilities: "required",
    actions: "optional",
    settings: "optional",
    contributions: "required",
  } satisfies Presence<ExtensionManifest>,
  Setting: {
    id: "required", type: "required", title: "required", description: "optional", required: "optional",
    default: "optional", options: "optional", minimum: "optional", maximum: "optional", integer: "optional",
    maxLength: "optional",
  } satisfies Presence<Setting>,
  SettingOption: { value: "required", label: "required" } satisfies Presence<NonNullable<Setting["options"]>[number]>,
  Binding: {
    name: "required",
    title: "required",
    target: "required",
    versions: "optional",
    jsonPathOverrides: "optional",
    arguments: "required",
    inputs: "required",
  } satisfies Presence<ExtensionManifest["capabilities"][number]>,
  ActionBinding: {
    name: "required",
    title: "required",
    target: "required",
    resource: "required",
    arguments: "required",
    preconditions: "optional",
    availableWhen: "optional",
  } satisfies Presence<NonNullable<ExtensionManifest["actions"]>[number]>,
  Predicate: {
    jsonPath: "required",
    equals: "optional",
    notEquals: "optional",
    present: "optional",
    absent: "optional",
    reason: "required",
  } satisfies Presence<ActionPredicate>,
  Contributions: {
    pages: "required",
    detailTabs: "required",
    detailLinks: "required",
    joins: "optional",
    tableColumns: "optional",
    dashboardCards: "optional",
    detailPanels: "optional",
    statusResolvers: "optional",
    badges: "optional",
    commands: "optional",
    resourceLinks: "optional",
  } satisfies Presence<Contributions>,
  PaletteCommand: { id:"required", title:"required", target:"required", forKinds:"optional" } satisfies Presence<NonNullable<Contributions["commands"]>[number]>,
  ResourceLink: { id:"required", from:"required", to:"required", relation:"required", match:"required" } satisfies Presence<NonNullable<Contributions["resourceLinks"]>[number]>,
  LinkMatch: { label:"optional", namespaceLabel:"optional", ownerReference:"optional", annotation:"optional", parse:"optional", defaultNamespace:"optional", name:"optional" } satisfies Presence<NonNullable<Contributions["resourceLinks"]>[number]["match"]>,
  StatusResolver: { forKinds:"required", rules:"required" } satisfies Presence<NonNullable<Contributions["statusResolvers"]>[number]>,
  Badge: { id:"required", forKinds:"required", join:"optional", rules:"required" } satisfies Presence<NonNullable<Contributions["badges"]>[number]>,
  StatusRule: { when:"required", status:"required", label:"required", reason:"optional" } satisfies Presence<ExtensionStatusRule>,
  Condition: { jsonPath:"required", equals:"optional", notEquals:"optional", present:"optional", absent:"optional", selfReference:"optional" } satisfies Presence<ExtensionStatusRule["when"][number]>,
  DashboardCard: {
    id: "required", title: "required", size: "required", type: "required", source: "required",
    predicate: "optional", target: "optional", metric: "optional", list: "optional",
  } satisfies Presence<Card>,
  CardPredicate: {
    jsonPath: "required", equals: "optional", absent: "optional", within: "optional", before: "optional",
  } satisfies Presence<NonNullable<Card["predicate"]>>,
  CardTarget: { page: "required" } satisfies Presence<NonNullable<Card["target"]>>,
  CardMetric: { jsonPath: "required", aggregate: "required" } satisfies Presence<NonNullable<Card["metric"]>>,
  CardList: { jsonPath: "optional", order: "optional", limit: "optional" } satisfies Presence<NonNullable<Card["list"]>>,
  Join: { id:"required", capability:"required", match:"required" } satisfies Presence<NonNullable<Contributions["joins"]>[number]>,
  JoinMatch: { label:"optional", kindLabel:"optional", ownerReference:"optional", annotation:"optional", name:"optional" } satisfies Presence<NonNullable<Contributions["joins"]>[number]["match"]>,
  TableColumn: { id:"required", title:"required", forKinds:"required", source:"required", format:"required", sortable:"optional", filterable:"optional" } satisfies Presence<NonNullable<Contributions["tableColumns"]>[number]>,
  ColumnSource: { join:"optional", jsonPath:"required" } satisfies Presence<NonNullable<Contributions["tableColumns"]>[number]["source"]>,
  DetailPanel: { id:"required", title:"required", forKinds:"required", sections:"required" } satisfies Presence<NonNullable<Contributions["detailPanels"]>[number]>,
  DetailField: { label:"required", jsonPath:"required", join:"optional", format:"optional" } satisfies Presence<Extract<NonNullable<Contributions["detailPanels"]>[number]["sections"][number], {type:"fields"}>["fields"][number]>,
  Page: {
    id: "required",
    title: "required",
    capability: "required",
    group: "optional",
    statusColumns: "optional",
    dashboard: "optional",
  } satisfies Presence<Page>,
  StatusColumns: {
    ready: "required",
    suspended: "optional",
    progressing: "optional",
  } satisfies Presence<NonNullable<Page["statusColumns"]>>,
  Dashboard: {
    pages: "required",
    events: "optional",
  } satisfies Presence<Dashboard>,
  DashboardEvents: {
    capability: "required",
    apiGroups: "required",
  } satisfies Presence<NonNullable<Dashboard["events"]>>,
  DetailTab: {
    id: "required",
    title: "required",
    capability: "required",
    forKinds: "required",
  } satisfies Presence<Contributions["detailTabs"][number]>,
  DetailLink: {
    id: "required",
    title: "required",
    capability: "required",
    forKinds: "required",
  } satisfies Presence<Contributions["detailLinks"][number]>,
};

const kinds = { declarative: true } satisfies Record<ExtensionManifest["kind"], true>;

interface ObjectSchema {
  properties?: Record<string, unknown>;
  required?: string[];
  enum?: string[];
}
const schema = inventorySchema as unknown as ObjectSchema & {
  definitions: Record<string, ObjectSchema>;
};
const rustType = (name: string) => (name === "Inventory" ? schema : schema.definitions[name]);

describe("extension TypeScript types match the Rust contract", () => {
  it.each(Object.keys(tables))("%s has the Rust field names and optionality", (name) => {
    const rust = rustType(name);
    expect(rust?.properties, `${name} is not a Rust type`).toBeDefined();
    const presence = Object.fromEntries(
      Object.keys(rust.properties!).map((field) => [
        field,
        rust.required?.includes(field) ? "required" : "optional",
      ]),
    );
    expect(tables[name]).toEqual(presence);
  });

  it("has a table for every Rust struct", () => {
    const structs = Object.entries(schema.definitions)
      .filter(([, definition]) => definition.properties)
      .map(([name]) => name);
    expect(Object.keys(tables).sort()).toEqual(["Inventory", ...structs].sort());
  });

  it("has the Rust install sources", () => {
    const sources = { local: true, catalog: true } satisfies Record<InstalledExtension["source"], true>;
    expect(Object.keys(sources).sort()).toEqual([...(schema.definitions.Source.enum ?? [])].sort());
  });

  it("has the Rust normalized statuses", () => {
    const statuses = { healthy: true, warning: true, error: true, progressing: true, suspended: true, unknown: true } satisfies Record<NormalizedStatus, true>;
    expect(Object.keys(statuses).sort()).toEqual([...(schema.definitions.NormalizedStatus.enum ?? [])].sort());
  });

  it("has the Rust self-reference formats", () => {
    const formats = { "argocd-tracking-id": true } satisfies Record<NonNullable<ExtensionStatusRule["when"][number]["selfReference"]>, true>;
    // A documented variant makes schemars emit `oneOf` rather than a flat `enum`.
    const rust = schema.definitions.ReferenceFormat as ObjectSchema & { oneOf?: ObjectSchema[] };
    const values = rust.enum ?? (rust.oneOf ?? []).flatMap((variant) => variant.enum ?? []);
    expect(values.length).toBeGreaterThan(0);
    expect(Object.keys(formats).sort()).toEqual([...values].sort());
  });

  it("has the Rust link relations", () => {
    const relations = { ownedBy: true, managedBy: true, exposedBy: true, references: true } satisfies Record<ExtensionLinkRelation, true>;
    expect(Object.keys(relations).sort()).toEqual([...(schema.definitions.LinkRelation.enum ?? [])].sort());
  });

  it("has the Rust manifest kinds", () => {
    expect(Object.keys(kinds).sort()).toEqual([...(schema.definitions.ManifestKind.enum ?? [])].sort());
  });

  it.each([
    ["CardSize", { s: true, m: true, l: true } satisfies Record<Card["size"], true>],
    ["SettingType", {
      string: true, number: true, boolean: true, select: true, "multi-select": true, url: true,
      "namespace-selector": true, "cluster-selector": true, "secret-reference": true,
    } satisfies Record<Setting["type"], true>],
    ["CardType", { count: true, countByStatus: true, metric: true, list: true } satisfies Record<Card["type"], true>],
    ["CardAggregate", { sum: true, min: true, max: true } satisfies Record<NonNullable<Card["metric"]>["aggregate"], true>],
    ["CardOrder", { asc: true, desc: true } satisfies Record<NonNullable<NonNullable<Card["list"]>["order"]>, true>],
  ])("has the Rust %s values", (name, values) => {
    // A documented variant is its own `oneOf` branch rather than one `enum` entry.
    const definition = schema.definitions[name] as ObjectSchema & { oneOf?: ObjectSchema[] };
    const rust = definition.enum ?? (definition.oneOf ?? []).flatMap((branch) => branch.enum ?? []);
    expect(rust.length).toBeGreaterThan(0);
    expect(Object.keys(values).sort()).toEqual([...rust].sort());
  });
});
