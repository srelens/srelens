import { expect, it } from "vitest";

it("uses jsdom storage rather than Node's process-wide Web Storage", () => {
  const browser = (globalThis as unknown as { jsdom: { window: Window } }).jsdom.window;
  expect(globalThis.localStorage).toBe(browser.localStorage);
  expect(globalThis.sessionStorage).toBe(browser.sessionStorage);
  localStorage.setItem("storage-regression", "browser-owned");
  expect(browser.localStorage.getItem("storage-regression")).toBe("browser-owned");
  localStorage.removeItem("storage-regression");
});
