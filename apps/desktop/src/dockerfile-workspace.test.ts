// @vitest-environment node
// (reads files from disk; jsdom buys nothing here)
import { describe, expect, it } from "vitest";
import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";

/**
 * The release image builds the frontend bundle itself, so every workspace
 * member the desktop app depends on has to be COPYed into it — the manifest
 * before `pnpm install`, because pnpm cannot link a `workspace:*` dependency
 * whose package.json is absent, and the sources before the build.
 *
 * Nothing else catches this. Typecheck, tests, coverage and `vite build` all
 * pass on a host where the whole repo is present; the failure only appears in
 * a release Docker build, which does not run on pull requests. Extracting
 * @srelens/core broke exactly this (#311), and the same break is waiting for
 * every package added after it.
 */
const root = join(__dirname, "../../..");
const dockerfile = readFileSync(join(root, "Dockerfile"), "utf8");

/** Workspace members under packages/, which is what pnpm-workspace globs. */
function packageDirs(): string[] {
  const dir = join(root, "packages");
  if (!existsSync(dir)) return [];
  return readdirSync(dir).filter((name) => existsSync(join(dir, name, "package.json")));
}

describe("the release Dockerfile", () => {
  const members = packageDirs();

  it("has workspace packages to check", () => {
    // Guards the guard: if packages/ ever moves, the loops below would vacuously
    // pass and the Dockerfile could silently drift.
    expect(members.length).toBeGreaterThan(0);
  });

  it.each(members)("copies packages/%s's manifest before install", (name) => {
    const copy = dockerfile.indexOf(`COPY packages/${name}/package.json`);
    const install = dockerfile.indexOf("RUN pnpm install");
    expect(copy, `Dockerfile never copies packages/${name}/package.json`).toBeGreaterThan(-1);
    expect(copy, `packages/${name}/package.json is copied after pnpm install`).toBeLessThan(install);
  });

  it.each(members)("copies packages/%s's sources before the frontend build", (name) => {
    const copy = dockerfile.indexOf(`COPY packages/${name} packages/${name}`);
    const build = dockerfile.indexOf("RUN pnpm --filter @srelens/desktop build");
    expect(copy, `Dockerfile never copies the packages/${name} sources`).toBeGreaterThan(-1);
    expect(copy, `packages/${name} sources are copied after the build`).toBeLessThan(build);
  });
});

/** Members of the Cargo workspace, from the root manifest's `members` list. */
function cargoMembers(): string[] {
  const manifest = readFileSync(join(root, "Cargo.toml"), "utf8");
  const list = manifest.match(/^members\s*=\s*\[([^\]]*)\]/m);
  if (!list) return [];
  return [...list[1].matchAll(/"([^"]+)"/g)].map((m) => m[1]);
}

/**
 * Cargo loads every workspace member's manifest even for `cargo build -p
 * srelens-server`, so the backend stage must copy every member, built or not.
 * `apps/tui` broke the release image exactly this way (see the Dockerfile's
 * comment on it), and the release build does not run on pull requests.
 */
describe("the release Dockerfile's backend stage", () => {
  const members = cargoMembers();

  it("has Cargo workspace members to check", () => {
    // Guards the guard: an unparsed `members` list would pass vacuously.
    expect(members.length).toBeGreaterThan(0);
  });

  it.each(members)("copies %s before the server build", (member) => {
    const stage = dockerfile.indexOf("AS backend");
    const build = dockerfile.indexOf("RUN cargo build", stage);
    expect(stage, "no backend stage").toBeGreaterThan(-1);
    expect(build, "no cargo build in the backend stage").toBeGreaterThan(stage);
    // A member is copied by its own path or by a parent's (`COPY crates crates`).
    // Only two-operand COPYs count: `COPY --from=…` and multi-file COPYs do not copy a member.
    const copied = [...dockerfile.slice(stage, build).matchAll(/^COPY (\S+) (\S+)$/gm)].some(
      ([, from]) => member === from || member.startsWith(`${from}/`),
    );
    expect(copied, `the backend stage never copies ${member}`).toBe(true);
  });
});
