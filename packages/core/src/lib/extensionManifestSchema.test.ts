import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import Ajv from "ajv";
import { describe, expect, it } from "vitest";

// The committed schema (crates/plugin-host/tests/schema.rs keeps it equal to the Rust
// contract) is what authors' editors validate against, so the examples must pass it.
// Not `new URL(template, import.meta.url)`: Vite rewrites that form as an asset import.
const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../../..");
const repoFile = (path: string) => readFileSync(resolve(repoRoot, path), "utf8");
const SCHEMA_URL =
  "https://raw.githubusercontent.com/srelens/srelens/main/schemas/extension-manifest.v0.4.json";
const schema = JSON.parse(repoFile("schemas/extension-manifest.v0.4.json"));
// Every example, so a new one cannot skip validation.
const examples = readdirSync(resolve(repoRoot, "examples/extensions"))
  .filter((name) => name.endsWith(".json"))
  .map((name) => `examples/extensions/${name}`);

describe("committed extension manifest schema", () => {
  // Ajv's default options, as an author's validator would use: strict mode rejects
  // formats JSON Schema does not define.
  const validate = new Ajv({ allErrors: true }).compile(schema);

  it("finds the examples", () => {
    expect(examples.length).toBeGreaterThan(0);
  });

  it.each(examples)("accepts %s and is named by it", (path) => {
    const manifest = JSON.parse(repoFile(path));
    expect(validate(manifest), JSON.stringify(validate.errors)).toBe(true);
    expect(manifest.$schema).toBe(SCHEMA_URL);
  });

  it("rejects unknown and missing fields, as the host does", () => {
    const manifest = JSON.parse(repoFile(examples[1]));
    expect(validate({ ...manifest, backend: { entry: "evil.js" } })).toBe(false);
    const { permissions: _permissions, ...withoutPermissions } = manifest;
    expect(validate(withoutPermissions)).toBe(false);
  });
});

// API 0.3's file is kept as it was when 0.4 was cut (#709), for manifests that still
// require ^0.3. It is what a host on the 0.3 line accepts: the signed releases published
// on that line, and none of what 0.4 added.
describe("frozen API 0.3 manifest schema", () => {
  const validate = new Ajv({ allErrors: true }).compile(
    JSON.parse(repoFile("schemas/extension-manifest.v0.3.json")),
  );

  it.each([
    "crates/registry/tests/fixtures/argocd-0.3.0-manifest.json",
    "crates/registry/tests/fixtures/flux-0.4.0-manifest.json",
  ])("accepts the published 0.3 release %s", (path) => {
    const manifest = JSON.parse(repoFile(path));
    expect(manifest.srelensApiVersion).toBe("^0.3");
    expect(validate(manifest), JSON.stringify(validate.errors)).toBe(true);
  });

  it.each(examples)("refuses %s, which uses API 0.4 fields", (path) => {
    const manifest = JSON.parse(repoFile(path));
    expect(manifest.srelensApiVersion).toBe("^0.4");
    expect(validate({ ...manifest, srelensApiVersion: "^0.3" })).toBe(false);
  });
});
