import { describe, expect, it } from "vitest";
import inventorySchema from "./extension-inventory.schema.json";
import type { ActionPredicate } from "./actionPredicates";
import type { ExtensionInventory, ExtensionManifest, InstalledExtension } from "./extensions";

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
    contributions: "required",
  } satisfies Presence<ExtensionManifest>,
  Binding: {
    name: "required",
    title: "required",
    target: "required",
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
  } satisfies Presence<Contributions>,
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

  it("has the Rust manifest kinds", () => {
    expect(Object.keys(kinds).sort()).toEqual([...(schema.definitions.ManifestKind.enum ?? [])].sort());
  });

  it.each([
    ["CardSize", { s: true, m: true, l: true } satisfies Record<Card["size"], true>],
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
