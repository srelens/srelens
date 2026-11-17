import { describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import type { NormalizedStatus } from "@srelens/core";
import { AppStatus, STATUS_KIND } from "./AppStatus";

const statuses: NormalizedStatus[] = ["healthy", "warning", "error", "progressing", "suspended", "unknown"];

describe("AppStatus", () => {
  it("always draws the word, whatever the status's colour", () => {
    // Colour is never the only signal: a reader who cannot tell the tones
    // apart reads the same answer from the text.
    for (const status of statuses) {
      const { container, unmount } = render(<AppStatus resolved={{ status, label: `Word for ${status}` }} />);
      const pill = container.querySelector(".status") as HTMLElement;
      expect(pill.textContent).toBe(`Word for ${status}`);
      expect(pill.dataset.kind).toBe(STATUS_KIND[status]);
      unmount();
    }
    expect(new Set(Object.values(STATUS_KIND)).size).toBeGreaterThan(3);
  });

  it("falls back to the host's word rather than drawing a bare dot", () => {
    const { container } = render(<AppStatus resolved={{ status: "suspended", label: "   " }} />);
    expect(container.querySelector(".status")?.textContent).toBe("Suspended");
    // A status this host does not know is still drawn, muted, as Unknown.
    const { container: odd } = render(
      <AppStatus resolved={{ status: "degraded" as NormalizedStatus, label: "" }} />,
    );
    expect(odd.querySelector(".status")?.textContent).toBe("Unknown");
    expect((odd.querySelector(".status") as HTMLElement).dataset.kind).toBe("neutral");
  });

  it("draws an app's label and a cluster's reason as plain text", () => {
    render(
      <AppStatus
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
      <AppStatus resolved={{ status: "healthy", label: "Flux", reason: "apps" }} />,
    );
    const wrapper = container.firstElementChild as HTMLElement;
    expect(wrapper.getAttribute("title")).toBe("apps");
    expect(container.querySelector(".status")?.textContent).toBe("Flux");
    // Visually hidden, so a screen reader hears the reason a pointer sees.
    expect(container.querySelector(".sr-only")?.textContent).toBe(": apps");
  });
});
