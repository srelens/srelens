import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ContextAvatar } from "./ContextAvatar";

describe("ContextAvatar", () => {
  it("shows a short-name badge together with a selected logo", () => {
    const { container } = render(
      <ContextAvatar context="production-eu" profile={{ logo: "shield", shortName: "EU" }} />,
    );
    expect(screen.getByText("EU")).toBeDefined();
    expect(container.querySelector("svg")).not.toBeNull();
  });

  it("does not duplicate the short name for an initials logo", () => {
    render(<ContextAvatar context="production-eu" profile={{ logo: "initials", shortName: "EU" }} />);
    expect(screen.getAllByText("EU")).toHaveLength(1);
  });
});

it("draws new-design symbols and respects the shared badge preference", () => {
  const { container } = render(<ContextAvatar context="prod" profile={{ logo: "cluster", markIcon: "terminal", shortName: "PR", showShortName: false }} />);
  expect(container.querySelector(".lucide-terminal")).not.toBeNull();
  expect(screen.queryByText("PR")).toBeNull();
});

it("derives badge text when a shared symbol explicitly enables it", () => {
  const { container } = render(
    <ContextAvatar context="production-eu" profile={{ logo: "cluster", markIcon: "terminal", showShortName: true }} />,
  );
  expect(container.querySelector(".fl-context-avatar--has-short")).not.toBeNull();
  expect(screen.getByText("PE")).toBeDefined();
});

it("draws an uploaded SVG mark shared by the new design", () => {
  const svg = "data:image/svg+xml;base64,PHN2Zy8+";
  const { container } = render(
    <ContextAvatar context="production-eu" profile={{ logo: "custom", logoUrl: svg }} />,
  );
  expect(container.querySelector("img")?.getAttribute("src")).toBe(svg);
});
