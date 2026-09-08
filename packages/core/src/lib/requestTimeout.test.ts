import { beforeEach, expect, it, vi } from "vitest";
vi.mock("../transport/transport", () => ({ invokeCommand: vi.fn() }));
import { invokeCommand } from "../transport/transport";
import { updateRequestTimeout } from "./requestTimeout";
import { getRequestTimeoutSecs, setRequestTimeoutSecs } from "./settings";
beforeEach(() => { vi.clearAllMocks(); localStorage.clear(); });
it("persists the backend's acknowledged timeout", async () => {
  vi.mocked(invokeCommand).mockResolvedValue(30);
  expect(await updateRequestTimeout(30)).toBe(30);
  expect(invokeCommand).toHaveBeenCalledWith("set_request_timeout", { secs: 30 });
  expect(getRequestTimeoutSecs()).toBe(30);
});
it("rejects a failed update and retains the previous preference", async () => {
  setRequestTimeoutSecs(12);
  vi.mocked(invokeCommand).mockRejectedValue(new Error("backend unavailable"));
  await expect(updateRequestTimeout(30)).rejects.toThrow("backend unavailable");
  expect(getRequestTimeoutSecs()).toBe(12);
});
