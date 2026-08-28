import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { Badge } from "./Badge";

describe("Badge", () => {
  it("renders its children", () => {
    render(<Badge>Running</Badge>);
    expect(screen.getByText("Running")).toBeDefined();
  });

  it("defaults to the muted tone, which has no wash", () => {
    // A neutral badge sits on whatever surface it lands on.
    render(<Badge>Unknown</Badge>);
    const badge = screen.getByText("Unknown");
    expect(badge.dataset.tone).toBe("muted");
    expect(badge.style.background).toBe("transparent");
  });

  it("colours the text and washes the background for a tone", () => {
    render(<Badge tone="sev">Failed</Badge>);
    const badge = screen.getByText("Failed");
    expect(badge.style.color).toContain("--sev");
    expect(badge.style.background).toContain("--sev-wash");
  });

  it("inverts fill and text when solid", () => {
    // For the one badge on screen that should read first.
    render(<Badge tone="sev" solid>Critical</Badge>);
    const badge = screen.getByText("Critical");
    expect(badge.style.background).toContain("--sev");
    expect(badge.style.color).toContain("--surface");
  });

});
