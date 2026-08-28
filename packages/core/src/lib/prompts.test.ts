import { describe, expect, it, vi, beforeEach } from "vitest";

const invokeCommand = vi.fn();
vi.mock("../transport/transport", () => ({ invokeCommand: (...a: unknown[]) => invokeCommand(...a) }));

import { getPrompt, listPrompts } from "./prompts";

describe("prompts", () => {
  beforeEach(() => invokeCommand.mockReset());

  it("listPrompts calls assistant_prompts_list and returns the summaries", async () => {
    const summaries = [
      { name: "pod-crashloop", description: "Work out why a pod keeps restarting", arguments: [] },
    ];
    invokeCommand.mockResolvedValue(summaries);

    await expect(listPrompts()).resolves.toEqual(summaries);
    expect(invokeCommand).toHaveBeenCalledWith("assistant_prompts_list");
  });

  it("getPrompt calls assistant_prompt_get with the name and args, returning the rendered text", async () => {
    invokeCommand.mockResolvedValue("Triage `my-cluster`.");

    await expect(getPrompt("pod-crashloop", { context: "my-cluster" })).resolves.toBe("Triage `my-cluster`.");
    expect(invokeCommand).toHaveBeenCalledWith("assistant_prompt_get", {
      name: "pod-crashloop",
      args: { context: "my-cluster" },
    });
  });
});
