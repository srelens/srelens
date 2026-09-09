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
it("serializes rapid updates so a stale acknowledgement cannot win", async () => {
  let acknowledgeFirst!: (value: number) => void;
  vi.mocked(invokeCommand)
    .mockImplementationOnce(() => new Promise(resolve => { acknowledgeFirst = resolve; }))
    .mockResolvedValueOnce(60);

  const first = updateRequestTimeout(30);
  const second = updateRequestTimeout(60);
  await vi.waitFor(() => expect(invokeCommand).toHaveBeenCalledTimes(1));
  acknowledgeFirst(30);
  await expect(Promise.all([first, second])).resolves.toEqual([30, 60]);

  expect(getRequestTimeoutSecs()).toBe(60);
  expect(vi.mocked(invokeCommand).mock.calls.map(call => call[1])).toEqual([{ secs: 30 }, { secs: 60 }]);
});
