// @vitest-environment node
import { expect, it } from "vitest";

it("does not expose browser storage to Node-only tests", () => {
  expect(typeof window).toBe("undefined");
  expect(typeof localStorage).toBe("undefined");
  expect(typeof sessionStorage).toBe("undefined");
});
