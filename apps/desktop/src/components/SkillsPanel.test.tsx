import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

vi.mock("../lib/notify", () => ({ notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));

const skillsLibMock = vi.hoisted(() => ({
  listSkills: vi.fn(),
  loadSkill: vi.fn(),
  saveSkill: vi.fn(),
  deleteSkill: vi.fn(),
}));
vi.mock("../lib/skills", () => skillsLibMock);

const chatLibMock = vi.hoisted(() => ({
  listAgents: vi.fn(),
  startChat: vi.fn(),
  sendChat: vi.fn(),
  cancelChat: vi.fn(),
}));
vi.mock("../lib/chat", () => chatLibMock);

import { SkillsPanel } from "./SkillsPanel";
import type { Skill, SkillMeta } from "../lib/skills";
import type { AgentEvent, AgentInfo } from "../lib/chat";

const AVAILABLE_AGENT: AgentInfo = {
  kind: "claude",
  label: "Claude Code",
  available: true,
  path: "/usr/bin/claude",
  version: null,
  installUrl: "",
  gated: false,
};

const METAS: SkillMeta[] = [
  { name: "alpha", description: "First skill" },
  { name: "zeta", description: "Last skill" },
];

const ALPHA: Skill = { name: "alpha", description: "First skill", body: "Body for alpha" };

describe("SkillsPanel", () => {
  beforeEach(() => {
    skillsLibMock.listSkills.mockReset().mockResolvedValue(METAS);
    skillsLibMock.loadSkill.mockReset().mockResolvedValue(ALPHA);
    skillsLibMock.saveSkill.mockReset().mockResolvedValue(undefined);
    skillsLibMock.deleteSkill.mockReset().mockResolvedValue(undefined);
    chatLibMock.listAgents.mockReset().mockResolvedValue([AVAILABLE_AGENT]);
    chatLibMock.startChat.mockReset().mockResolvedValue("gen-session-1");
    chatLibMock.sendChat.mockReset().mockResolvedValue(undefined);
    chatLibMock.cancelChat.mockReset().mockResolvedValue(undefined);
  });

  it("lists skills by name and description", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);

    expect(await screen.findByText("alpha")).toBeTruthy();
    expect(screen.getByText("First skill")).toBeTruthy();
    expect(screen.getByText("zeta")).toBeTruthy();
    expect(screen.getByText("Last skill")).toBeTruthy();
  });

  it('"New skill" opens an empty editor', async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByRole("button", { name: "New skill" }));

    const nameInput = (await screen.findByLabelText("Skill name")) as HTMLInputElement;
    const descInput = screen.getByLabelText("Skill description") as HTMLInputElement;
    const bodyInput = screen.getByLabelText("Skill body") as HTMLTextAreaElement;
    expect(nameInput.value).toBe("");
    expect(descInput.value).toBe("");
    expect(bodyInput.value).toBe("");
  });

  it("selecting a skill loads it into the editor", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByText("alpha"));

    expect(skillsLibMock.loadSkill).toHaveBeenCalledWith("alpha");
    await waitFor(() => {
      expect((screen.getByLabelText("Skill name") as HTMLInputElement).value).toBe("alpha");
    });
    expect((screen.getByLabelText("Skill description") as HTMLInputElement).value).toBe("First skill");
    expect((screen.getByLabelText("Skill body") as HTMLTextAreaElement).value).toBe("Body for alpha");
  });

  it("Save calls saveSkill with the exact typed fields and refreshes the list", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByRole("button", { name: "New skill" }));
    fireEvent.change(await screen.findByLabelText("Skill name"), { target: { value: "crashloop-triage" } });
    fireEvent.change(screen.getByLabelText("Skill description"), {
      target: { value: "Systematic triage for a pod that keeps restarting" },
    });
    fireEvent.change(screen.getByLabelText("Skill body"), { target: { value: "Step 1: check the exit code." } });

    skillsLibMock.listSkills.mockResolvedValueOnce([
      ...METAS,
      { name: "crashloop-triage", description: "Systematic triage for a pod that keeps restarting" },
    ]);

    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(skillsLibMock.saveSkill).toHaveBeenCalledTimes(1));
    expect(skillsLibMock.saveSkill).toHaveBeenCalledWith({
      name: "crashloop-triage",
      description: "Systematic triage for a pod that keeps restarting",
      body: "Step 1: check the exit code.",
    });
    // The list is refreshed after a successful save.
    await waitFor(() => expect(skillsLibMock.listSkills).toHaveBeenCalledTimes(2));
    expect(await screen.findByText("crashloop-triage")).toBeTruthy();
  });

  it("renaming an existing skill deletes the old file after writing the new one", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    // Open the existing "alpha" skill and rename it.
    fireEvent.click(screen.getByText("alpha"));
    await waitFor(() =>
      expect((screen.getByLabelText("Skill name") as HTMLInputElement).value).toBe("alpha"),
    );
    fireEvent.change(screen.getByLabelText("Skill name"), { target: { value: "alpha-renamed" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    // New file written, then the old one removed so both don't linger.
    await waitFor(() =>
      expect(skillsLibMock.saveSkill).toHaveBeenCalledWith(
        expect.objectContaining({ name: "alpha-renamed" }),
      ),
    );
    await waitFor(() => expect(skillsLibMock.deleteSkill).toHaveBeenCalledWith("alpha"));
  });

  it("refuses to rename a skill onto another existing skill's name", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByText("alpha"));
    await waitFor(() =>
      expect((screen.getByLabelText("Skill name") as HTMLInputElement).value).toBe("alpha"),
    );
    // "zeta" is another existing skill (METAS[1]).
    fireEvent.change(screen.getByLabelText("Skill name"), { target: { value: "zeta" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    // Neither overwrite nor delete — the collision is refused with a message.
    expect(await screen.findByText(/already exists/i)).toBeTruthy();
    expect(skillsLibMock.saveSkill).not.toHaveBeenCalled();
    expect(skillsLibMock.deleteSkill).not.toHaveBeenCalled();
  });

  it("saving an existing skill without renaming does not delete anything", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByText("alpha"));
    await waitFor(() =>
      expect((screen.getByLabelText("Skill name") as HTMLInputElement).value).toBe("alpha"),
    );
    fireEvent.change(screen.getByLabelText("Skill body"), { target: { value: "edited body" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(skillsLibMock.saveSkill).toHaveBeenCalledTimes(1));
    expect(skillsLibMock.deleteSkill).not.toHaveBeenCalled();
  });

  it("Delete, once confirmed, calls deleteSkill with the skill's name", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByRole("button", { name: "Delete alpha" }));
    expect(await screen.findByText(/Delete "alpha"/)).toBeTruthy();

    skillsLibMock.listSkills.mockResolvedValueOnce([METAS[1]]);
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));

    await waitFor(() => expect(skillsLibMock.deleteSkill).toHaveBeenCalledWith("alpha"));
  });

  it("Delete does not call deleteSkill when the confirmation is cancelled", async () => {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");

    fireEvent.click(screen.getByRole("button", { name: "Delete alpha" }));
    await screen.findByText(/Delete "alpha"/);
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));

    expect(skillsLibMock.deleteSkill).not.toHaveBeenCalled();
  });
});

describe("SkillsPanel — Generate with AI", () => {
  beforeEach(() => {
    skillsLibMock.listSkills.mockReset().mockResolvedValue(METAS);
    skillsLibMock.loadSkill.mockReset().mockResolvedValue(ALPHA);
    skillsLibMock.saveSkill.mockReset().mockResolvedValue(undefined);
    skillsLibMock.deleteSkill.mockReset().mockResolvedValue(undefined);
    chatLibMock.listAgents.mockReset().mockResolvedValue([AVAILABLE_AGENT]);
    chatLibMock.startChat.mockReset().mockResolvedValue("gen-session-1");
    chatLibMock.sendChat.mockReset().mockResolvedValue(undefined);
    chatLibMock.cancelChat.mockReset().mockResolvedValue(undefined);
  });

  async function openNewSkillEditor() {
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");
    fireEvent.click(screen.getByRole("button", { name: "New skill" }));
    await screen.findByLabelText("Skill name");
  }

  it("clicking Generate invokes the bridge with the exact meta-prompt, built from the typed need", async () => {
    chatLibMock.sendChat.mockImplementation(async (_s: string, _p: string, _a: string, onEvent: (e: AgentEvent) => void) => {
      onEvent({ type: "textDelta", text: "## crashloop-triage\n" });
      onEvent({ type: "turnDone" });
    });
    await openNewSkillEditor();

    fireEvent.change(await screen.findByLabelText("Skill need"), {
      target: { value: "triage a pod that keeps restarting" },
    });
    fireEvent.click(screen.getByRole("button", { name: /generate with ai/i }));

    await waitFor(() => expect(chatLibMock.sendChat).toHaveBeenCalledTimes(1));
    expect(chatLibMock.startChat).toHaveBeenCalledTimes(1);
    const [session, prompt, agentPath] = chatLibMock.sendChat.mock.calls[0];
    expect(session).toBe("gen-session-1");
    expect(agentPath).toBe("/usr/bin/claude");
    // Hand-written literal built from the brief's meta-prompt text, not by
    // echoing the component's own builder.
    expect(prompt).toBe(
      "Write a srelens assistant skill as markdown with name/description front-matter for the following need: triage a pod that keeps restarting. Output only the markdown.",
    );
  });

  it("closing the panel mid-generation cancels the backend turn", async () => {
    // Hold the turn open: sendChat never settles until we let it.
    let resolveSend: () => void = () => {};
    chatLibMock.sendChat.mockImplementation(() => new Promise<void>((r) => { resolveSend = () => r(); }));
    const { unmount } = render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");
    fireEvent.click(screen.getByRole("button", { name: "New skill" }));
    await screen.findByLabelText("Skill name");
    fireEvent.change(await screen.findByLabelText("Skill need"), { target: { value: "some need" } });
    fireEvent.click(screen.getByRole("button", { name: /generate with ai/i }));
    await waitFor(() => expect(chatLibMock.sendChat).toHaveBeenCalledTimes(1));

    // Closing the dialog must stop the backend turn — there is no Stop
    // control left anywhere once the panel is gone.
    unmount();
    expect(chatLibMock.cancelChat).toHaveBeenCalledWith("gen-session-1");
    resolveSend();
  });

  it("generates through the pathless native agent (path: null) instead of silently no-oping", async () => {
    chatLibMock.listAgents.mockResolvedValue([
      { kind: "srelens", label: "srelens agent", available: true, path: null, version: null, installUrl: "", gated: false },
    ]);
    chatLibMock.sendChat.mockImplementation(async (_s: string, _p: string, _a: string, onEvent: (e: AgentEvent) => void) => {
      onEvent({ type: "textDelta", text: "generated body" });
      onEvent({ type: "turnDone" });
    });
    await openNewSkillEditor();

    fireEvent.change(await screen.findByLabelText("Skill need"), { target: { value: "some need" } });
    fireEvent.click(screen.getByRole("button", { name: /generate with ai/i }));

    await waitFor(() => expect(chatLibMock.sendChat).toHaveBeenCalledTimes(1));
    const [, , agentPath, , , agentKind] = chatLibMock.sendChat.mock.calls[0];
    expect(agentPath).toBe("");
    expect(agentKind).toBe("srelens");
    await waitFor(() => {
      expect((screen.getByLabelText("Skill body") as HTMLTextAreaElement).value).toBe("generated body");
    });
  });

  it("the streamed markdown lands in the body field once the turn completes", async () => {
    chatLibMock.sendChat.mockImplementation(async (_s: string, _p: string, _a: string, onEvent: (e: AgentEvent) => void) => {
      onEvent({ type: "textDelta", text: "Step 1: check the exit code.\n" });
      onEvent({ type: "textDelta", text: "Step 2: check the logs." });
      onEvent({ type: "turnDone" });
    });
    await openNewSkillEditor();

    fireEvent.change(screen.getByLabelText("Skill need"), { target: { value: "triage a crashlooping pod" } });
    fireEvent.click(screen.getByRole("button", { name: /generate with ai/i }));

    await waitFor(() => {
      expect((screen.getByLabelText("Skill body") as HTMLTextAreaElement).value).toBe(
        "Step 1: check the exit code.\nStep 2: check the logs.",
      );
    });
  });

  it("parses name/description front-matter out of the generated markdown into their fields", async () => {
    chatLibMock.sendChat.mockImplementation(async (_s: string, _p: string, _a: string, onEvent: (e: AgentEvent) => void) => {
      onEvent({
        type: "textDelta",
        text: "---\nname: crashloop-triage\ndescription: Systematic triage for a crashlooping pod\n---\n# Steps\nCheck exit code.",
      });
      onEvent({ type: "turnDone" });
    });
    await openNewSkillEditor();

    fireEvent.change(screen.getByLabelText("Skill need"), { target: { value: "triage a crashlooping pod" } });
    fireEvent.click(screen.getByRole("button", { name: /generate with ai/i }));

    await waitFor(() => {
      expect((screen.getByLabelText("Skill name") as HTMLInputElement).value).toBe("crashloop-triage");
    });
    expect((screen.getByLabelText("Skill description") as HTMLInputElement).value).toBe(
      "Systematic triage for a crashlooping pod",
    );
    expect((screen.getByLabelText("Skill body") as HTMLTextAreaElement).value).toBe("# Steps\nCheck exit code.");
  });

  it("a stream error surfaces inline near Generate and does not wipe existing editor content", async () => {
    chatLibMock.sendChat.mockImplementation(async (_s: string, _p: string, _a: string, onEvent: (e: AgentEvent) => void) => {
      onEvent({ type: "error", message: "agent crashed" });
      onEvent({ type: "turnDone" });
    });
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");
    fireEvent.click(screen.getByText("alpha"));
    await waitFor(() => {
      expect((screen.getByLabelText("Skill body") as HTMLTextAreaElement).value).toBe("Body for alpha");
    });

    fireEvent.change(screen.getByLabelText("Skill need"), { target: { value: "triage a crashlooping pod" } });
    fireEvent.click(screen.getByRole("button", { name: /generate with ai/i }));

    expect(await screen.findByText("agent crashed")).toBeTruthy();
    expect((screen.getByLabelText("Skill body") as HTMLTextAreaElement).value).toBe("Body for alpha");
  });

  it("a thrown sendChat surfaces inline and does not wipe existing editor content", async () => {
    chatLibMock.sendChat.mockRejectedValue(new Error("transport down"));
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");
    fireEvent.click(screen.getByText("alpha"));
    await waitFor(() => {
      expect((screen.getByLabelText("Skill body") as HTMLTextAreaElement).value).toBe("Body for alpha");
    });

    fireEvent.change(screen.getByLabelText("Skill need"), { target: { value: "triage a crashlooping pod" } });
    fireEvent.click(screen.getByRole("button", { name: /generate with ai/i }));

    expect(await screen.findByText("transport down")).toBeTruthy();
    expect((screen.getByLabelText("Skill body") as HTMLTextAreaElement).value).toBe("Body for alpha");
  });

  it("Generate is disabled with a short note when no agent is available/ungated", async () => {
    chatLibMock.listAgents.mockResolvedValue([{ ...AVAILABLE_AGENT, available: false }]);
    await openNewSkillEditor();

    fireEvent.change(screen.getByLabelText("Skill need"), { target: { value: "triage a crashlooping pod" } });

    await waitFor(() => {
      expect((screen.getByRole("button", { name: /generate with ai/i }) as HTMLButtonElement).disabled).toBe(true);
    });
    expect(screen.getByText(/install\/enable an agent to generate/i)).toBeTruthy();
    expect(chatLibMock.sendChat).not.toHaveBeenCalled();
  });

  it("Generate is disabled when only a gated agent is available", async () => {
    chatLibMock.listAgents.mockResolvedValue([{ ...AVAILABLE_AGENT, gated: true }]);
    await openNewSkillEditor();

    fireEvent.change(screen.getByLabelText("Skill need"), { target: { value: "triage a crashlooping pod" } });

    await waitFor(() => {
      expect((screen.getByRole("button", { name: /generate with ai/i }) as HTMLButtonElement).disabled).toBe(true);
    });
  });

  it("Generate is disabled until a need is typed", async () => {
    await openNewSkillEditor();

    expect((screen.getByRole("button", { name: /generate with ai/i }) as HTMLButtonElement).disabled).toBe(true);
  });

  it("disables New skill and the skill-list items while a generation is in flight, and the originally-open editor receives the result", async () => {
    // A `sendChat` that doesn't settle until `resolveSend()` is called, so the
    // test can assert mid-flight state (navigation disabled) before letting
    // the turn complete — guards against a `turnDone` landing on whatever
    // skill is open *then* rather than the one open when Generate was
    // clicked (switching skills, or New skill, mid-generation must be
    // blocked instead).
    let resolveSend: () => void = () => {};
    chatLibMock.sendChat.mockImplementation(
      (_s: string, _p: string, _a: string, onEvent: (e: AgentEvent) => void) =>
        new Promise<void>((resolve) => {
          resolveSend = () => {
            onEvent({ type: "textDelta", text: "Body for crashloop-triage." });
            onEvent({ type: "turnDone" });
            resolve();
          };
        }),
    );
    render(<SkillsPanel onClose={vi.fn()} />);
    await screen.findByText("alpha");
    fireEvent.click(screen.getByText("alpha"));
    await waitFor(() => {
      expect((screen.getByLabelText("Skill body") as HTMLTextAreaElement).value).toBe("Body for alpha");
    });

    fireEvent.change(screen.getByLabelText("Skill need"), { target: { value: "triage a crashlooping pod" } });
    fireEvent.click(screen.getByRole("button", { name: /generate with ai/i }));

    await waitFor(() => {
      expect((screen.getByRole("button", { name: "New skill" }) as HTMLButtonElement).disabled).toBe(true);
    });
    expect((screen.getByText("alpha").closest("button") as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByText("zeta").closest("button") as HTMLButtonElement).disabled).toBe(true);

    resolveSend();

    await waitFor(() => {
      expect((screen.getByLabelText("Skill body") as HTMLTextAreaElement).value).toBe("Body for crashloop-triage.");
    });
    // Still "alpha" the whole time — the originally-open skill got the result.
    expect((screen.getByLabelText("Skill name") as HTMLInputElement).value).toBe("alpha");
    expect((screen.getByRole("button", { name: "New skill" }) as HTMLButtonElement).disabled).toBe(false);
  });
});
