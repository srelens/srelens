import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { expect, it } from "vitest";

// Apps are built for the new design only (#1001): classic shows a notice in
// their place, so nothing here may draw ui-next's extension views or load
// their stylesheet again.
it("classic imports nothing from @srelens/ui-next/extensions", () => {
  const offenders = (readdirSync(__dirname, { recursive: true }) as string[])
    .filter((file) => /\.(tsx?|css)$/.test(file) && !/\.test\./.test(file))
    .filter((file) => readFileSync(join(__dirname, file), "utf8").includes("@srelens/ui-next/extensions"))
    .map((file) => file.replaceAll("\\", "/"));
  expect(offenders).toEqual([]);
});
