import { describe, it, expect } from "vitest";
import { readFileSync, existsSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";

/**
 * `@srelens/core` is consumed by more than one UI, so React reaching the main
 * entry point is the package boundary failing. Asserted rather than left to
 * review: this is the kind of rule that erodes one convenient import at a time.
 *
 * The first version of this test checked only for a direct `from "react"` in
 * each module and exempted the two hook modules by name. That was not enough,
 * and review on #311 caught it: `kubectlMapper` imported a pure mapper from
 * `access.ts`, so `import { toKubectl } from "@srelens/core"` pulled React in
 * transitively while this test stayed green.
 *
 * So it now walks the actual import graph from the barrel. A module is only
 * safe if nothing it reaches, at any depth, imports React — directly, or by
 * depending on a package that requires it. The second half was also caught in
 * review: `notify.ts` imported `sonner`, a React component library, so the
 * barrier held for relative imports and leaked through an external one.
 */

/** Resolve a relative import specifier to a file on disk. */
function resolveImport(fromFile: string, spec: string): string | null {
  const base = resolve(dirname(fromFile), spec);
  for (const candidate of [base, `${base}.ts`, join(base, "index.ts")]) {
    if (existsSync(candidate) && candidate.endsWith(".ts")) return candidate;
  }
  return null;
}

/** Every module reachable from `entry` by relative imports, plus its own path. */
function reachableFrom(entry: string): Map<string, string[]> {
  const seen = new Map<string, string[]>();
  const walk = (file: string, path: string[]) => {
    if (seen.has(file)) return;
    const trail = [...path, file];
    seen.set(file, trail);
    const source = readFileSync(file, "utf8");
    for (const m of source.matchAll(/from\s+["'](\.[^"']*)["']/g)) {
      const next = resolveImport(file, m[1]);
      if (next) walk(next, trail);
    }
  };
  walk(entry, []);
  return seen;
}

function importsReact(file: string): boolean {
  return /from\s+["']react["']/.test(readFileSync(file, "utf8"));
}

/** Bare (non-relative) package specifiers a module imports. */
function bareImports(file: string): string[] {
  const source = readFileSync(file, "utf8");
  return [...source.matchAll(/from\s+["']([^."'][^"']*)["']/g)]
    .map((m) => m[1])
    .filter((spec) => !spec.startsWith("node:"));
}

/** Package name from a specifier: "@scope/pkg/sub" -> "@scope/pkg". */
function packageName(spec: string): string {
  return spec.startsWith("@") ? spec.split("/").slice(0, 2).join("/") : spec.split("/")[0];
}

const manifest = JSON.parse(
  readFileSync(join(__dirname, "../package.json"), "utf8"),
) as { dependencies?: Record<string, string> };

/**
 * Whether a declared runtime dependency needs React.
 *
 * Deliberately reads the *declared* dependencies rather than resolving whatever
 * a module happens to import: an undeclared package is not installed under this
 * package, so resolving it would throw and the check would pass by accident —
 * which is exactly how a first attempt at this test missed `sonner`.
 */
function dependencyNeedsReact(name: string): boolean {
  // Read the manifest from disk rather than require()-ing it: a package whose
  // "exports" map omits ./package.json (@tauri-apps/plugin-process does) throws
  // on resolution, and a throwing check is a check that passes by accident.
  const file = join(__dirname, "../node_modules", name, "package.json");
  if (!existsSync(file)) return false;
  const pkg = JSON.parse(readFileSync(file, "utf8")) as {
    dependencies?: Record<string, string>;
    peerDependencies?: Record<string, string>;
  };
  return Boolean(pkg.dependencies?.react ?? pkg.peerDependencies?.react);
}

const short = (p: string) => relative(join(__dirname, ".."), p);

describe("the service layer", () => {
  it("does not reach React from the main entry point, at any depth", () => {
    const reachable = reachableFrom(join(__dirname, "index.ts"));
    const offenders = [...reachable.entries()]
      .filter(([file]) => importsReact(file))
      // Show the chain, not just the culprit: the import that needs breaking is
      // usually the one before it.
      .map(([, trail]) => trail.map(short).join("\n    -> "));
    expect(offenders, `React reachable from the barrel:\n  ${offenders.join("\n  ")}`).toEqual([]);
  });

  it("declares no runtime dependency that requires React", () => {
    const offenders = Object.keys(manifest.dependencies ?? {}).filter(dependencyNeedsReact);
    expect(
      offenders,
      `these dependencies pull React in: ${offenders.join(", ")}`,
    ).toEqual([]);
  });

  it("imports nothing it has not declared", () => {
    // A phantom dependency resolves through hoisting today and vanishes in the
    // Docker build tomorrow — and it would slip past the check above, which can
    // only see what is declared.
    const declared = new Set(Object.keys(manifest.dependencies ?? {}));
    const offenders = new Set<string>();
    for (const [file] of reachableFrom(join(__dirname, "index.ts"))) {
      for (const spec of bareImports(file)) {
        const name = packageName(spec);
        if (!declared.has(name)) offenders.add(`${short(file)} imports undeclared ${name}`);
      }
    }
    expect([...offenders]).toEqual([]);
  });

  it("keeps the two React hooks out of the main barrel", () => {
    const barrel = readFileSync(join(__dirname, "index.ts"), "utf8");
    expect(barrel).not.toMatch(/lib\/access/);
    expect(barrel).not.toMatch(/lib\/useNamespaceOptions/);
  });

  it("still allows React behind the ./react entry point", () => {
    // Guards the guard: if the hooks stopped importing React the first test
    // would pass for the wrong reason, and the boundary would be untested.
    const reachable = reachableFrom(join(__dirname, "react.ts"));
    expect([...reachable.keys()].some(importsReact)).toBe(true);
  });
});
