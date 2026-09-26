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
/** The published URL of an API line's schema, which a manifest written for it names. */
const schemaUrl = (line: string) =>
  `https://raw.githubusercontent.com/srelens/srelens/main/schemas/extension-manifest.v${line}.json`;
const schema = JSON.parse(repoFile("schemas/extension-manifest.v0.5.json"));
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

  it.each(examples)("accepts %s, which names its own line's schema", (path) => {
    const manifest = JSON.parse(repoFile(path));
    expect(validate(manifest), JSON.stringify(validate.errors)).toBe(true);
    // `^0.4` names v0.4: an example that needs nothing newer stays on its line (#728).
    expect(manifest.$schema).toBe(schemaUrl(manifest.srelensApiVersion.replace(/^\^/, "")));
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
    expect(manifest.srelensApiVersion).toMatch(/^\^0\.[45]$/);
    expect(validate({ ...manifest, srelensApiVersion: "^0.3" })).toBe(false);
  });
});

// API 0.4's file is kept as it was when 0.5 was cut (#728): srelens 0.15.1-186 and -187
// implement 0.4 without resource links by spec path or to built-in kinds.
describe("frozen API 0.4 manifest schema", () => {
  const validate = new Ajv({ allErrors: true }).compile(
    JSON.parse(repoFile("schemas/extension-manifest.v0.4.json")),
  );

  it("accepts the Argo CD example, which uses nothing 0.5 added", () => {
    const manifest = JSON.parse(repoFile("examples/extensions/argocd.json"));
    expect(manifest.srelensApiVersion).toBe("^0.4");
    expect(validate(manifest), JSON.stringify(validate.errors)).toBe(true);
  });

  it("refuses the Flux example's spec-path links", () => {
    const manifest = JSON.parse(repoFile("examples/extensions/flux.json"));
    expect(manifest.srelensApiVersion).toBe("^0.5");
    expect(validate(manifest)).toBe(false);
    const { resourceLinks, ...contributions } = manifest.contributions;
    const withoutPaths = resourceLinks.filter((link: { match: { path?: string } }) => !link.match.path);
    expect(withoutPaths.length).toBeGreaterThan(0);
    expect(validate({ ...manifest, contributions: { ...contributions, resourceLinks: withoutPaths } }),
      JSON.stringify(validate.errors)).toBe(true);
  });
});

describe("network.http permissions (#568)", () => {
  const validate = new Ajv({ allErrors: true }).compile(schema);
  const frozen = new Ajv({ allErrors: true }).compile(JSON.parse(repoFile("schemas/extension-manifest.v0.3.json")));
  /** An example asking for network.http with its hosts. */
  const scoped = () => ({
    ...JSON.parse(repoFile(examples[0])),
    permissions: [{ capability: "network.http", hosts: ["${settings.prometheusUrl}", "api.github.com"] }],
  });

  it("accepts network.http granted with hosts, which API 0.3's schema does not", () => {
    expect(validate(scoped()), JSON.stringify(validate.errors)).toBe(true);
    // A published 0.3 release that 0.3 accepts, with a scoped entry the only change.
    const published = JSON.parse(repoFile("crates/registry/tests/fixtures/argocd-0.3.0-manifest.json"));
    expect(frozen(published), JSON.stringify(frozen.errors)).toBe(true);
    const withHosts = {
      ...published,
      permissions: [...published.permissions, { capability: "network.http", hosts: ["api.github.com"] }],
    };
    expect(frozen(withHosts)).toBe(false);
  });

  it("rejects a scoped permission with an unknown or a missing field, as the host does", () => {
    const methods = scoped();
    methods.permissions[0].methods = ["POST"];
    expect(validate(methods)).toBe(false);
    const hostless = scoped();
    delete hostless.permissions[0].hosts;
    expect(validate(hostless)).toBe(false);
  });
});
