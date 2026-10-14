import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import type { NormalizedStatus } from "@srelens/core";
import { StatusBadge, STATUS_TONE } from "./StatusBadge";

const statuses: NormalizedStatus[] = ["healthy", "warning", "error", "progressing", "suspended", "unknown"];

describe("StatusBadge", () => {
  it("always draws the word, whatever the status's colour", () => {
    // Colour is never the only signal: a reader who cannot tell the tones
    // apart reads the same answer from the text.
    for (const status of statuses) {
      const { container, unmount } = render(<StatusBadge resolved={{ status, label: `Word for ${status}` }} />);
      const badge = container.querySelector(".badge") as HTMLElement;
      expect(badge.textContent).toBe(`Word for ${status}`);
      expect(badge.dataset.tone).toBe(STATUS_TONE[status]);
      unmount();
    }
    expect(new Set(Object.values(STATUS_TONE)).size).toBeGreaterThan(3);
  });

  it("falls back to the host's word rather than drawing an empty badge", () => {
    const { container } = render(<StatusBadge resolved={{ status: "suspended", label: "   " }} />);
    expect(container.querySelector(".badge")?.textContent).toBe("Suspended");
    // A status this host does not know is still drawn, muted, as Unknown.
    const { container: odd } = render(
      <StatusBadge resolved={{ status: "degraded" as NormalizedStatus, label: "" }} />,
    );
    expect(odd.querySelector(".badge")?.textContent).toBe("Unknown");
    expect((odd.querySelector(".badge") as HTMLElement).dataset.tone).toBe("muted");
  });

  it("draws an app's label and a cluster's reason as plain text", () => {
    render(
      <StatusBadge
        showReason
        resolved={{ status: "error", label: "<b>Not ready</b>‮", reason: "build failed​<script>x</script>" }}
      />,
    );
    const label = screen.getByText("<b>Not ready</b>\\u202e");
    expect(label.querySelector("b")).toBeNull();
    expect(screen.getByText("build failed\\u200b<script>x</script>")).toBeTruthy();
    expect(document.querySelector("script")).toBeNull();
  });

  it("keeps the reason off the row unless asked, but readable on hover and by assistive tech", () => {
    const { container } = render(
      <StatusBadge resolved={{ status: "healthy", label: "Flux", reason: "apps" }} />,
    );
    const wrapper = container.firstElementChild as HTMLElement;
    expect(wrapper.getAttribute("title")).toBe("apps");
    expect(container.querySelector(".badge")?.textContent).toBe("Flux");
    // Visually hidden, so a screen reader hears the reason a pointer sees.
    expect(container.querySelector(".sr-only")?.textContent).toBe(": apps");
  });
});
