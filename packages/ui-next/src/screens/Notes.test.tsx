import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { parseReleaseNotes } from "@srelens/core";

import { Notes } from "./Notes";

// The parser is core's and has its own tests; what these check is the markup it
// turns into — real headings, lists and paragraphs rather than a dump of the
// raw text, and never markup built from the note text itself.

describe("Notes", () => {
  it("renders a heading as an h3", () => {
    render(<Notes blocks={parseReleaseNotes("### What's new")} />);
    expect(screen.getByRole("heading", { level: 3, name: "What's new" })).toBeDefined();
  });

  it("renders bullets as a list with one item each", () => {
    const { container } = render(<Notes blocks={parseReleaseNotes("- one\n- two")} />);
    const lists = container.querySelectorAll("ul");
    expect(lists.length).toBe(1);
    const items = [...lists[0].querySelectorAll("li")].map((li) => li.textContent);
    expect(items).toEqual(["one", "two"]);
  });

  it("renders prose as a paragraph", () => {
    const { container } = render(<Notes blocks={parseReleaseNotes("A plain line.")} />);
    const paragraphs = [...container.querySelectorAll("p")].map((p) => p.textContent);
    expect(paragraphs).toEqual(["A plain line."]);
  });

  it("renders a code span as a code element", () => {
    const { container } = render(<Notes blocks={parseReleaseNotes("`x`")} />);
    expect(container.innerHTML).toContain("<code>x</code>");
  });

  it("renders a bold span as a strong element", () => {
    const { container } = render(<Notes blocks={parseReleaseNotes("**loud**")} />);
    expect(container.innerHTML).toContain("<strong>loud</strong>");
  });

  it("keeps the text around a span, in order", () => {
    const { container } = render(<Notes blocks={parseReleaseNotes("run `kubectl` now")} />);
    expect(container.querySelector("p")?.textContent).toBe("run kubectl now");
  });

  it("renders markup in the notes as text, never as markup", () => {
    const { container } = render(
      <Notes blocks={parseReleaseNotes("Fixed <script>alert(1)</script> handling")} />,
    );
    expect(container.querySelector("script")).toBeNull();
    expect(container.innerHTML).toContain("&lt;script&gt;");
    expect(screen.getByText(/Fixed <script>alert\(1\)<\/script> handling/)).toBeDefined();
  });

  it("renders nothing when there are no blocks", () => {
    const { container } = render(<Notes blocks={[]} />);
    expect(container.innerHTML).toBe("");
  });
});
