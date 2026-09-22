import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { DiffRow } from "@srelens/core";
import { HostConfirmation, CONFIRM_DISPLAY_MAX_CHARS } from "./HostConfirmation";

/** A right-to-left override: drawn, it reverses the text that follows it. */
const RLO = "‮";

function patchOf(unchanged: number): DiffRow[] {
  const rows: DiffRow[] = [];
  for (let i = 0; i < unchanged; i += 1) {
    rows.push({ tag: "same", left: `  line ${i}`, right: `  line ${i}` });
  }
  rows.push({ tag: "insert", left: null, right: `  suspend: true` });
  return rows;
}

describe("the one host-owned confirmation", () => {
  it("leads with the host's sentence and the level it was given", () => {
    render(
      <HostConfirmation
        question="Suspend HelmRelease team/api in cluster prod?"
        impact="high"
        cluster="prod"
        subject={{ kind: "object", namespace: "team", name: "api" }}
      />,
    );
    expect(screen.getByTestId("host-confirm-question").textContent).toBe(
      "Suspend HelmRelease team/api in cluster prod?",
    );
    expect(screen.getByTestId("host-confirm-impact").textContent).toBe("High impact");
  });

  it("names the pinned cluster and the object the call is about to change", () => {
    render(
      <HostConfirmation
        question="Suspend HelmRelease team/api?"
        impact="medium"
        cluster="prod"
        subject={{ kind: "object", namespace: "team", name: "api" }}
      />,
    );
    expect(screen.getByTestId("host-confirm-cluster").textContent).toBe("prod");
    expect(screen.getByTestId("host-confirm-target").textContent).toBe("team/api");
  });

  it("names a bulk selection as a count of resources", () => {
    render(
      <HostConfirmation
        question="Suspend 12 resources in cluster prod?"
        impact="high"
        cluster="prod"
        subject={{ kind: "bulk", count: 12 }}
      />,
    );
    expect(screen.getByTestId("host-confirm-target").textContent).toBe("12 resources");
  });

  it("names one selected resource in the singular", () => {
    render(
      <HostConfirmation question="Suspend 1 resource?" impact="low" subject={{ kind: "bulk", count: 1 }} />,
    );
    expect(screen.getByTestId("host-confirm-target").textContent).toBe("1 resource");
  });

  it("says which app asked and who signed it", () => {
    render(
      <HostConfirmation
        question="Suspend HelmRelease team/api?"
        impact="medium"
        app={{ name: "Flux Tools", publisher: "srelens" }}
      />,
    );
    expect(screen.getByTestId("host-confirm-requester").textContent).toBe(
      "Requested by app Flux Tools (srelens)",
    );
  });

  it("says an app carrying no signature is unsigned", () => {
    render(
      <HostConfirmation
        question="Suspend HelmRelease team/api?"
        impact="medium"
        app={{ name: "Local Draft", publisher: null }}
      />,
    );
    expect(screen.getByTestId("host-confirm-requester").textContent).toBe(
      "Requested by app Local Draft (unsigned)",
    );
  });

  it("does not let an app-supplied name reorder the question", () => {
    render(
      <HostConfirmation
        question="Suspend HelmRelease team/api?"
        impact="high"
        app={{ name: `Flux${RLO}snoitacilppA`, publisher: null }}
      />,
    );
    const requester = screen.getByTestId("host-confirm-requester").textContent ?? "";
    expect(requester).not.toContain(RLO);
    expect(requester).toContain("\\u202e");
  });

  it("does not let an over-long app-supplied name push the question off the frame", () => {
    render(
      <HostConfirmation
        question="Suspend HelmRelease team/api?"
        impact="high"
        app={{ name: "n".repeat(400), publisher: null }}
      />,
    );
    const drawn = screen.getByTestId("host-confirm-app-name").textContent ?? "";
    expect(drawn.length).toBe(CONFIRM_DISPLAY_MAX_CHARS);
    expect(drawn.endsWith("…")).toBe(true);
    // The sentence still ends with the host's own words, not the app's.
    expect(screen.getByTestId("host-confirm-requester").textContent?.endsWith("(unsigned)")).toBe(true);
  });

  it("draws the exact patch through the shared diff renderer, long unchanged runs collapsed", () => {
    const { container } = render(
      <HostConfirmation question="Suspend?" impact="medium" patch={patchOf(30)} />,
    );
    expect(screen.getByRole("button", { name: /Show 2[0-9] unchanged lines/ })).toBeTruthy();
    const drawn = [...container.querySelectorAll('[data-slot="line"]')].map((n) => n.textContent);
    expect(drawn.some((line) => line?.includes("suspend: true"))).toBe(true);
    expect(drawn.length).toBeLessThan(31);
  });

  it("opens a collapsed run where it stands", async () => {
    const { container } = render(
      <HostConfirmation question="Suspend?" impact="medium" patch={patchOf(30)} />,
    );
    await userEvent.click(screen.getByRole("button", { name: /unchanged lines/ }));
    expect(container.querySelectorAll('[data-slot="line"]').length).toBe(31);
  });

  it("escapes a format character inside the patch rather than drawing it", () => {
    const { container } = render(
      <HostConfirmation
        question="Apply?"
        impact="high"
        patch={[{ tag: "insert", left: null, right: `  name: api${RLO}dexinwp` }]}
      />,
    );
    const text = container.textContent ?? "";
    expect(text).not.toContain(RLO);
    expect(text).toContain("\\u202e");
  });

  it("asks the same question in the transcript's smaller frame", () => {
    render(
      <HostConfirmation
        frame="card"
        question="Suspend HelmRelease team/api in cluster prod?"
        impact="high"
        cluster="prod"
        subject={{ kind: "object", namespace: "team", name: "api" }}
        app={{ name: "Flux Tools", publisher: null }}
      />,
    );
    expect(screen.getByTestId("host-confirm-question").textContent).toBe(
      "Suspend HelmRelease team/api in cluster prod?",
    );
    expect(screen.getByTestId("host-confirm-impact").textContent).toBe("High impact");
    expect(screen.getByTestId("host-confirm-cluster").textContent).toBe("prod");
    expect(screen.getByTestId("host-confirm-target").textContent).toBe("team/api");
    expect(screen.getByTestId("host-confirm-requester").textContent).toBe(
      "Requested by app Flux Tools (unsigned)",
    );
  });

  /**
   * The level is the host's fact about the capability, not this component's
   * guess. A surface that was sent none draws none — labelling a status
   * refresh "Medium impact" because something had to go there would make the
   * badge worth less on the prompt where it matters.
   */
  it("names no level rather than inventing one the host did not send", () => {
    render(<HostConfirmation question="Refresh?" impact={null} cluster="prod" />);
    expect(screen.queryByTestId("host-confirm-impact")).toBeNull();
    expect(screen.getByTestId("host-confirm-question").textContent).toBe("Refresh?");
  });

  it("draws no question at all rather than half of one", () => {
    render(<HostConfirmation question={null} impact="medium" cluster="prod" />);
    expect(screen.queryByTestId("host-confirm-question")).toBeNull();
    expect(screen.getByTestId("host-confirm-cluster").textContent).toBe("prod");
  });

  it("takes no styling from its caller", () => {
    const props = { question: "Suspend?", impact: "low" as const };
    // @ts-expect-error the component accepts no class name: an app must not be
    // able to make the host's own question look like something else.
    render(<HostConfirmation {...props} className="totally-fine" />);
    expect(document.querySelector(".totally-fine")).toBeNull();
  });
});
