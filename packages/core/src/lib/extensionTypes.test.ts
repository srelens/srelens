import { describe, expect, it } from "vitest";
import inventorySchema from "./extension-inventory.schema.json";
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

const tables: Record<string, Record<string, "required" | "optional">> = {
  Inventory: {
    schemaVersion: "required",
    nextRevision: "required",
    plugins: "required",
  } satisfies Presence<ExtensionInventory>,
  Installed: {
    signatureProof: "optional",
    quarantined: "optional",
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
    contributions: "required",
  } satisfies Presence<ExtensionManifest>,
  Binding: {
    name: "required",
    title: "required",
    target: "required",
    arguments: "required",
    inputs: "required",
  } satisfies Presence<ExtensionManifest["capabilities"][number]>,
  Contributions: {
    pages: "required",
    detailTabs: "required",
    detailLinks: "required",
  } satisfies Presence<Contributions>,
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
});
