import { afterEach, beforeEach, expect, it, vi } from "vitest";

const core = vi.hoisted(() => ({ tauri: true, saveTextFile: vi.fn() }));
vi.mock("@srelens/core", () => ({ isTauri: () => core.tauri, saveTextFile: core.saveTextFile }));

import { saveOrDownload } from "./saveOrDownload";

const created = vi.fn(() => "blob:settings");
const revoked = vi.fn();
beforeEach(() => {
  core.tauri = true;
  core.saveTextFile.mockReset();
  Object.assign(URL, { createObjectURL: created, revokeObjectURL: revoked });
});
afterEach(() => vi.restoreAllMocks());

it("saves through the native dialog on the desktop", async () => {
  await saveOrDownload("app-settings.json", "{}\n");
  expect(core.saveTextFile).toHaveBeenCalledWith("app-settings.json", "{}\n");
  expect(created).not.toHaveBeenCalled();
});

/** The web has no `save_text_file` command: an Apps settings export there is a download. */
it("downloads in the browser on the web, and asks no host command", async () => {
  core.tauri = false;
  const click = vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
    expect(this.download).toBe("app-settings.json");
    expect(this.href).toBe("blob:settings");
  });
  await saveOrDownload("app-settings.json", "{}\n");
  expect(click).toHaveBeenCalledTimes(1);
  expect(core.saveTextFile).not.toHaveBeenCalled();
  expect(revoked).toHaveBeenCalledWith("blob:settings");
  expect(document.querySelector("a[download]")).toBeNull();
});
