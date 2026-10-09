import { beforeEach, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const core = vi.hoisted(() => ({
  listSkills: vi.fn(),
  loadSkill: vi.fn(),
  deleteSkill: vi.fn(),
  saveSkill: vi.fn(),
  skillsDirPath: vi.fn(),
  revealSkill: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({ ...(await orig<typeof import("@srelens/core")>()), ...core }));
const setSkillActive = vi.hoisted(() => vi.fn());
vi.mock("../../lib/agentRun", () => ({ setSkillActive }));

import { settingsStorage } from "@srelens/core";
import { SKILL_DEFAULTS_KEY, __resetSkillDefaultsForTests, getSkillDefaults } from "../../lib/skillDefaults";
import { SkillsPane } from "./SkillsPane";

/** Bodies sized so the estimates are round: a quarter of the characters. */
const BODIES: Record<string, string> = {
  "crashloop-triage": "x".repeat(4000),
  "pending-pod": "x".repeat(2000),
  "team-runbook": "x".repeat(800),
};
const METAS = [
  { name: "crashloop-triage", description: "Triage a pod stuck in CrashLoopBackOff", builtin: true },
  { name: "pending-pod", description: "Diagnose a pod stuck in Pending", builtin: true },
  {
    name: "team-runbook",
    description: "How this team rolls back a release",
    builtin: false,
    path: "/home/dana/.config/srelens/assistant/skills/team-runbook.md",
  },
];
const FOLDER = "/home/dana/.config/srelens/assistant/skills";

const card = (name: string) => document.querySelector(`[data-skill="${name}"]`) as HTMLElement;
const toggle = (name: string) => screen.getByRole("switch", { name: `${name}: on for every new conversation` });
const cost = () => document.querySelector('[data-slot="skills-cost"]')?.textContent;

beforeEach(() => {
  settingsStorage.removeItem(SKILL_DEFAULTS_KEY);
  __resetSkillDefaultsForTests();
  setSkillActive.mockReset();
  core.listSkills.mockReset().mockResolvedValue(METAS);
  core.loadSkill.mockReset().mockImplementation(async (name: string) => ({
    ...METAS.find((m) => m.name === name)!,
    body: BODIES[name],
  }));
  core.deleteSkill.mockReset().mockResolvedValue(undefined);
  core.saveSkill.mockReset().mockResolvedValue(undefined);
  core.skillsDirPath.mockReset().mockResolvedValue(FOLDER);
  core.revealSkill.mockReset().mockResolvedValue(undefined);
});

async function shown() {
  render(<SkillsPane />);
  await screen.findByText("crashloop-triage");
}

describe("SkillsPane", () => {
  it("lists every installed skill with what it is and where it came from", async () => {
    await shown();
    expect(screen.getByText("Installed (3)")).toBeDefined();
    expect(within(card("crashloop-triage")).getByText("Triage a pod stuck in CrashLoopBackOff")).toBeDefined();
    expect(within(card("crashloop-triage")).getByText("Preinstalled")).toBeDefined();
    expect(within(card("crashloop-triage")).getByText(/^Ships with srelens\./)).toBeDefined();
    expect(within(card("team-runbook")).getByText("Installed by you")).toBeDefined();
  });

  it("says about how many tokens each skill adds when it is on", async () => {
    await shown();
    expect(within(card("crashloop-triage")).getByText("≈1k tokens when on")).toBeDefined();
    expect(within(card("pending-pod")).getByText("≈500 tokens when on")).toBeDefined();
    expect(within(card("team-runbook")).getByText("≈200 tokens when on")).toBeDefined();
  });

  it("starts with nothing on, and nothing added to a question", async () => {
    await shown();
    expect(toggle("crashloop-triage").getAttribute("aria-checked")).toBe("false");
    expect(cost()).toBe("≈0");
    expect(screen.getByText(/from 0 skills that are on/)).toBeDefined();
  });

  /**
   * The figure is what the reader's choices cost: a skill that is on has its
   * whole instructions sent with every question, so the total is the sum of
   * the ones that are on, and it moves as they are switched.
   */
  it("adds up what the skills that are on send with every question", async () => {
    await shown();
    await userEvent.click(toggle("crashloop-triage"));
    expect(cost()).toBe("≈1k");
    expect(screen.getByText(/from 1 skill that is on/)).toBeDefined();
    await userEvent.click(toggle("pending-pod"));
    expect(cost()).toBe("≈1.5k");
    await userEvent.click(toggle("crashloop-triage"));
    expect(cost()).toBe("≈500");
  });

  it("keeps a skill that is switched on, and turns it on for this window too", async () => {
    await shown();
    await userEvent.click(toggle("pending-pod"));
    expect(getSkillDefaults()).toEqual(["pending-pod"]);
    expect(toggle("pending-pod").getAttribute("aria-checked")).toBe("true");
    expect(setSkillActive).toHaveBeenCalledWith("pending-pod", true);
  });

  it("opens with the skills kept from before already on", async () => {
    settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(["team-runbook"]));
    await shown();
    expect(toggle("team-runbook").getAttribute("aria-checked")).toBe("true");
    expect(cost()).toBe("≈200");
  });

  it("does not count a kept name whose skill is no longer installed", async () => {
    settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(["deleted-long-ago", "pending-pod"]));
    await shown();
    expect(cost()).toBe("≈500");
    expect(screen.getByText(/from 1 skill that is on/)).toBeDefined();
  });

  it("lists a skill whose instructions cannot be read, without a size, and says it is not counted", async () => {
    core.loadSkill.mockImplementation(async (name: string) => {
      if (name === "pending-pod") throw new Error("could not read");
      return { ...METAS.find((m) => m.name === name)!, body: BODIES[name] };
    });
    await shown();
    expect(within(card("pending-pod")).getByText("size unknown")).toBeDefined();
    await userEvent.click(toggle("pending-pod"));
    expect(cost()).toBe("≈0");
    expect(screen.getByText("1 skill could not be read and is not counted.")).toBeDefined();
  });

  it("narrows the list to the skills that match the search, by name or description", async () => {
    await shown();
    await userEvent.type(screen.getByRole("textbox", { name: "Search installed skills" }), "rolls back");
    expect(card("team-runbook")).not.toBeNull();
    expect(card("crashloop-triage")).toBeNull();
    await userEvent.clear(screen.getByRole("textbox", { name: "Search installed skills" }));
    await userEvent.type(screen.getByRole("textbox", { name: "Search installed skills" }), "PENDING");
    expect(card("pending-pod")).not.toBeNull();
    expect(card("team-runbook")).toBeNull();
  });

  it("says so when nothing matches the search", async () => {
    await shown();
    await userEvent.type(screen.getByRole("textbox", { name: "Search installed skills" }), "zzz");
    expect(screen.getByText("No skill matches")).toBeDefined();
  });

  it("reads the skills again on Refresh", async () => {
    await shown();
    core.listSkills.mockResolvedValue([...METAS, { name: "new-one", description: "Just added", builtin: false }]);
    core.loadSkill.mockResolvedValue({ name: "new-one", description: "", body: "abcd" });
    await userEvent.click(screen.getByRole("button", { name: "Refresh" }));
    expect(await screen.findByText("new-one")).toBeDefined();
    expect(screen.getByText("Installed (4)")).toBeDefined();
  });

  it("offers to uninstall a user's skill, and not one that ships with the app", async () => {
    await shown();
    expect(within(card("team-runbook")).getByRole("button", { name: "Uninstall" })).toBeDefined();
    expect(within(card("crashloop-triage")).queryByRole("button", { name: "Uninstall" })).toBeNull();
  });

  it("asks before uninstalling, and leaves the skill alone when the answer is no", async () => {
    await shown();
    await userEvent.click(within(card("team-runbook")).getByRole("button", { name: "Uninstall" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    expect(core.deleteSkill).not.toHaveBeenCalled();
  });

  it("uninstalls on confirmation, stops keeping it on, and reads the list again", async () => {
    settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(["team-runbook"]));
    await shown();
    core.listSkills.mockResolvedValue(METAS.slice(0, 2));
    await userEvent.click(within(card("team-runbook")).getByRole("button", { name: "Uninstall" }));
    const dialog = await screen.findByRole("dialog");
    await userEvent.click(within(dialog).getByRole("button", { name: "Uninstall" }));
    await waitFor(() => expect(card("team-runbook")).toBeNull());
    expect(core.deleteSkill).toHaveBeenCalledExactlyOnceWith("team-runbook");
    expect(getSkillDefaults()).toEqual([]);
    expect(setSkillActive).toHaveBeenCalledWith("team-runbook", false);
  });

  it("says so when a skill could not be removed, and keeps it listed", async () => {
    core.deleteSkill.mockRejectedValue(new Error("permission denied"));
    await shown();
    await userEvent.click(within(card("team-runbook")).getByRole("button", { name: "Uninstall" }));
    await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Uninstall" }));
    expect(await screen.findByText("Could not remove the skill")).toBeDefined();
    expect(card("team-runbook")).not.toBeNull();
  });

  it("says so when the skills could not be loaded, and tries again when asked", async () => {
    core.listSkills.mockRejectedValueOnce(new Error("store unavailable"));
    render(<SkillsPane />);
    expect(await screen.findByText("Could not load skills")).toBeDefined();
    await userEvent.click(screen.getByRole("button", { name: "Try again" }));
    expect(await screen.findByText("crashloop-triage")).toBeDefined();
  });

  it("says there are none, rather than drawing an empty list", async () => {
    core.listSkills.mockResolvedValue([]);
    render(<SkillsPane />);
    expect(await screen.findByText("No skills")).toBeDefined();
    expect(screen.getByText("Installed (0)")).toBeDefined();
  });

  /**
   * A skill is a file. The reader who wants to change one by hand has to know
   * where it is, and the pane is the place that knows.
   */
  describe("where the files are", () => {
    it("shows the folder skill files are kept in, as text that can be copied", async () => {
      await shown();
      const folder = await waitFor(() => {
        const el = document.querySelector('[data-slot="skills-folder"]');
        expect(el).not.toBeNull();
        return el as HTMLElement;
      });
      expect(folder.textContent).toBe(FOLDER);
      expect(folder.className).toContain("select-all");
    });

    it("opens that folder", async () => {
      await shown();
      const where = screen.getByRole("region", { name: "Where skill files are kept" });
      await userEvent.click(within(where).getByRole("button", { name: "Open folder" }));
      // No name: the folder itself.
      expect(core.revealSkill).toHaveBeenCalledExactlyOnceWith(undefined);
    });

    it("shows each of the reader's skills its own file, and opens the folder on it", async () => {
      await shown();
      expect(card("team-runbook").querySelector('[data-slot="skill-path"]')?.textContent).toBe(
        "/home/dana/.config/srelens/assistant/skills/team-runbook.md",
      );
      await userEvent.click(within(card("team-runbook")).getByRole("button", { name: "Open folder" }));
      expect(core.revealSkill).toHaveBeenCalledExactlyOnceWith("team-runbook");
    });

    /**
     * The skills that ship with srelens are preinstalled: part of the app,
     * switched on or off and nothing else.
     */
    it("gives a preinstalled skill nothing to press but its switch, and says why", async () => {
      await shown();
      const shipped = card("crashloop-triage");
      expect(shipped.querySelector('[data-slot="skill-path"]')).toBeNull();
      expect(within(shipped).getByText(/part of the app and cannot be edited or uninstalled/)).toBeDefined();
      expect(within(shipped).queryAllByRole("button")).toEqual([]);
      expect(within(shipped).getAllByRole("switch")).toHaveLength(1);
    });

    it("marks a file that replaces a preinstalled skill, and offers to restore the preinstalled version", async () => {
      core.listSkills.mockResolvedValue([
        { ...METAS[0], builtin: false, overridesBuiltin: true, path: `${FOLDER}/crashloop-triage.md` },
        ...METAS.slice(1),
      ]);
      await shown();
      const mine = card("crashloop-triage");
      // Such a file can only have come from outside this pane: it is shown
      // for what it is, and the way back is one press.
      expect(within(mine).getByText("Replaces a preinstalled skill")).toBeDefined();
      expect(mine.querySelector('[data-slot="skill-path"]')?.textContent).toBe(`${FOLDER}/crashloop-triage.md`);
      expect(within(mine).queryByRole("button", { name: "Uninstall" })).toBeNull();
      await userEvent.click(within(mine).getByRole("button", { name: "Restore preinstalled version" }));
      const dialog = await screen.findByRole("dialog");
      expect(within(dialog).getByText(/the version that ships with srelens is used again/)).toBeDefined();
    });

    it("leaves a skill on when only its copy is removed: the skill is still there", async () => {
      settingsStorage.setItem(SKILL_DEFAULTS_KEY, JSON.stringify(["crashloop-triage"]));
      core.listSkills.mockResolvedValue([
        { ...METAS[0], builtin: false, overridesBuiltin: true, path: `${FOLDER}/crashloop-triage.md` },
        ...METAS.slice(1),
      ]);
      await shown();
      await userEvent.click(within(card("crashloop-triage")).getByRole("button", { name: "Restore preinstalled version" }));
      await userEvent.click(within(await screen.findByRole("dialog")).getByRole("button", { name: "Restore" }));
      await waitFor(() => expect(core.deleteSkill).toHaveBeenCalledExactlyOnceWith("crashloop-triage"));
      expect(getSkillDefaults()).toEqual(["crashloop-triage"]);
    });

    it("says so when the folder could not be opened", async () => {
      core.revealSkill.mockRejectedValue(new Error("no file manager"));
      await shown();
      await userEvent.click(within(card("team-runbook")).getByRole("button", { name: "Open folder" }));
      expect(await screen.findByText("Could not open the skills folder")).toBeDefined();
    });

    it("still lists the skills when the folder's path cannot be read", async () => {
      core.skillsDirPath.mockRejectedValue(new Error("no config dir"));
      await shown();
      expect(screen.getByText("a folder that could not be found.")).toBeDefined();
      expect(card("team-runbook")).not.toBeNull();
    });
  });

  /**
   * A reader's own skill, installed when they need one: from a file they
   * have, or written in place.
   */
  describe("installing a skill", () => {
    const openDialog = async () => {
      await shown();
      await userEvent.click(screen.getByRole("button", { name: "Install skill…" }));
      return screen.findByRole("dialog", { name: "Install a skill" });
    };
    const field = (dialog: HTMLElement, label: string) => within(dialog).getByLabelText(new RegExp(`^${label}`));
    const fill = async (dialog: HTMLElement, name: string, body = "Step 1: look.") => {
      await userEvent.type(field(dialog, "Name"), name);
      await userEvent.type(field(dialog, "Instructions"), body);
    };

    it("installs what was written: the name, the description and the instructions", async () => {
      const dialog = await openDialog();
      await userEvent.type(field(dialog, "Name"), "pvc-resize");
      await userEvent.type(field(dialog, "Description"), "Resize a bound PVC");
      await userEvent.type(field(dialog, "Instructions"), "Check the StorageClass first.");
      await userEvent.click(within(dialog).getByRole("button", { name: "Install" }));
      await waitFor(() =>
        expect(core.saveSkill).toHaveBeenCalledExactlyOnceWith({
          name: "pvc-resize",
          description: "Resize a bound PVC",
          body: "Check the StorageClass first.",
        }),
      );
      // The dialog goes and the list is read again, so the new skill shows.
      await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
      expect(core.listSkills).toHaveBeenCalledTimes(2);
    });

    it("fills the form from a chosen file, and installs nothing until asked", async () => {
      const dialog = await openDialog();
      const file = new File(
        ["---\nname: team-oncall\ndescription: Who to page and when\n---\nPage the primary first.\n"],
        "anything.md",
        { type: "text/markdown" },
      );
      await userEvent.upload(within(dialog).getByLabelText("Skill file"), file);
      await waitFor(() => expect((field(dialog, "Name") as HTMLInputElement).value).toBe("team-oncall"));
      expect((field(dialog, "Description") as HTMLInputElement).value).toBe("Who to page and when");
      expect((field(dialog, "Instructions") as HTMLTextAreaElement).value).toBe("Page the primary first.\n");
      expect(core.saveSkill).not.toHaveBeenCalled();
      await userEvent.click(within(dialog).getByRole("button", { name: "Install" }));
      await waitFor(() => expect(core.saveSkill).toHaveBeenCalledTimes(1));
    });

    it("names a skill for its file when the file has no front matter", async () => {
      const dialog = await openDialog();
      await userEvent.upload(
        within(dialog).getByLabelText("Skill file"),
        new File(["Just check the events.\n"], "My Runbook.md", { type: "text/markdown" }),
      );
      await waitFor(() => expect((field(dialog, "Name") as HTMLInputElement).value).toBe("My-Runbook"));
    });

    it("cannot install without a name and instructions", async () => {
      const dialog = await openDialog();
      const install = within(dialog).getByRole("button", { name: "Install" });
      expect(install.hasAttribute("disabled")).toBe(true);
      await userEvent.type(field(dialog, "Name"), "pvc-resize");
      expect(install.hasAttribute("disabled")).toBe(true);
      await userEvent.type(field(dialog, "Instructions"), "   ");
      expect(install.hasAttribute("disabled")).toBe(true);
      await userEvent.type(field(dialog, "Instructions"), "Do it.");
      expect(install.hasAttribute("disabled")).toBe(false);
    });

    it("refuses a name the skill store would refuse, and says what is wrong with it", async () => {
      const dialog = await openDialog();
      await fill(dialog, "my skill");
      expect(within(dialog).getByText(/letters, numbers, dots, dashes and underscores only/)).toBeDefined();
      expect(within(dialog).getByRole("button", { name: "Install" }).hasAttribute("disabled")).toBe(true);
    });

    it("refuses a preinstalled skill's name: those are not replaced from here", async () => {
      const dialog = await openDialog();
      await fill(dialog, "crashloop-triage");
      expect(within(dialog).getByText("That name belongs to a preinstalled skill. Choose another.")).toBeDefined();
      expect(within(dialog).queryByRole("button", { name: "Replace" })).toBeNull();
      expect(within(dialog).getByRole("button", { name: "Install" }).hasAttribute("disabled")).toBe(true);
      expect(core.saveSkill).not.toHaveBeenCalled();
    });

    it("says it is replacing a skill the reader already has, before it does", async () => {
      const dialog = await openDialog();
      await fill(dialog, "team-runbook");
      expect(within(dialog).getByText("You already have a skill called team-runbook. Installing replaces it.")).toBeDefined();
      await userEvent.click(within(dialog).getByRole("button", { name: "Replace" }));
      await waitFor(() => expect(core.saveSkill).toHaveBeenCalledTimes(1));
    });

    it("shows about what the skill will cost while it is being written", async () => {
      const dialog = await openDialog();
      await userEvent.type(field(dialog, "Instructions"), "x".repeat(40));
      expect(within(dialog).getByText("≈10 tokens when on")).toBeDefined();
    });

    it("says so when the skill could not be installed, and keeps what was written", async () => {
      core.saveSkill.mockRejectedValue(new Error("read-only file system"));
      const dialog = await openDialog();
      await fill(dialog, "pvc-resize");
      await userEvent.click(within(dialog).getByRole("button", { name: "Install" }));
      expect(await within(dialog).findByText("Could not install the skill")).toBeDefined();
      expect((field(dialog, "Name") as HTMLInputElement).value).toBe("pvc-resize");
    });

    it("installs nothing when cancelled", async () => {
      const dialog = await openDialog();
      await fill(dialog, "pvc-resize");
      await userEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
      expect(screen.queryByRole("dialog")).toBeNull();
      expect(core.saveSkill).not.toHaveBeenCalled();
    });
  });

  it("claims nothing it does not do: no registry, no on-demand loading", async () => {
    await shown();
    expect(screen.queryByText(/Browse/)).toBeNull();
    expect(screen.queryByText(/saved/)).toBeNull();
    expect(screen.getByText(/is sent with every question/)).toBeDefined();
  });
});
