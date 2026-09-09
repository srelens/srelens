// @vitest-environment node
import { expect, it } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

it("defines every shared mark token for classic dark and light modes", () => {
  const kit = readFileSync(join(__dirname, "../../../../packages/ui-kit/src/styles/tokens.css"), "utf8");
  const classic = readFileSync(join(__dirname, "styles.css"), "utf8");
  const tokens = new Set(kit.match(/--mark-[a-z]+/g) ?? []);

  expect(tokens.size).toBeGreaterThan(0);
  for (const token of tokens) {
    expect(classic.match(new RegExp(`${token}:`, "g"))?.length ?? 0, token).toBeGreaterThanOrEqual(2);
  }
});
