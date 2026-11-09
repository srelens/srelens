import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { LogLine } from "./LogLine";

/**
 * A log line is four columns that have to stay in their columns down a stream
 * of thousands, and a level whose colour is the first thing a reader looks for.
 * Both of those, and the colour prop that let a caller name its own. (#320)
 */
describe("LogLine", () => {
  const cell = (container: HTMLElement, slot: string) =>
    container.querySelector(`[data-slot="${slot}"]`) as HTMLElement;

  it("prints the timestamp, source, level and message", () => {
    render(<LogLine ts="14:02:11" source="kubelet" level="error" message="probe failed" />);
    expect(screen.getByText("14:02:11")).toBeDefined();
    expect(screen.getByText("kubelet")).toBeDefined();
    expect(screen.getByText("error")).toBeDefined();
    expect(screen.getByText("probe failed")).toBeDefined();
  });

  it("colours the level from the level itself", () => {
    // The mock made `tone` required at every call site, so the same word was
    // free to arrive red on one screen and grey on the next.
    const { container } = render(<LogLine ts="14:02:11" level="error" message="probe failed" />);
    expect(cell(container, "level").style.color).toContain("var(--sev)");
  });

  it("reads the level whatever case and spacing it arrives in", () => {
    const { container } = render(<LogLine ts="14:02:11" level=" WARN " message="slow" />);
    expect(cell(container, "level").style.color).toContain("var(--warn)");
  });

  it("tones a panic as severely as it reads", () => {
    // `panic` is what a Go process says on its way out, and core's severity rule
    // has always treated it as danger. It was missing from LEVEL_TONE, so the
    // most severe line a process can emit rendered grey.
    const { container } = render(<LogLine ts="14:02:11" level="panic" message="runtime error" />);
    expect(cell(container, "level").style.color).toContain("var(--sev)");
  });

  it("tones every word core calls danger as severely as core means it", () => {
    // core's LEVEL_HEALTH is the vocabulary a log line's declared level is read
    // against, and this map is what colours it. A word core calls danger that
    // this map has never heard of falls through to muted grey — which is how
    // `panic` shipped grey until it was caught. `emerg` and `alert` are
    // syslog's two most severe levels, above `crit`.
    for (const level of ["emerg", "emergency", "alert", "dpanic"]) {
      const { container } = render(<LogLine ts="14:02:11" level={level} message="x" />);
      expect(cell(container, "level").style.color).toContain("var(--sev)");
    }
  });

  it("stays neutral for a level it does not recognise", () => {
    const { container } = render(<LogLine ts="14:02:11" level="audit" message="x" />);
    expect(cell(container, "level").style.color).toContain("var(--ink-muted)");
  });

  it("lets the caller overrule the level's tone", () => {
    const { container } = render(
      <LogLine ts="14:02:11" level="info" tone="sev" message="the one that matters" />,
    );
    expect(cell(container, "level").style.color).toContain("var(--sev)");
  });

  it("keeps the level as text, not only as a colour", () => {
    // Colour is the fast path, not the only one.
    const { container } = render(<LogLine ts="14:02:11" level="error" message="x" />);
    expect(cell(container, "level").textContent).toBe("error");
  });

  it("takes a tone for the source rather than a colour", () => {
    // The mock's `sourceColor?: string` was a hole straight through the token
    // rule: any caller could hand it a hex value.
    const { container } = render(
      <LogLine ts="14:02:11" source="api" sourceTone="info" message="x" />,
    );
    expect(cell(container, "source").style.color).toContain("var(--info)");
  });

  it("gives the source the accent colour by default", () => {
    const { container } = render(<LogLine ts="14:02:11" source="api" message="x" />);
    expect(cell(container, "source").style.color).toContain("var(--accent)");
  });

  it("keeps the source and level columns when they are empty", () => {
    // The reverse of every other slot in the kit: these are gutters in a grid
    // of lines, and a line that drops one shunts its message out of line with
    // every other message on screen.
    const { container } = render(<LogLine ts="14:02:11" message="no source, no level" />);
    expect(cell(container, "source")).not.toBeNull();
    expect(cell(container, "level")).not.toBeNull();
  });

  it("says so when there is no message", () => {
    // An empty row in a log is indistinguishable from a rendering fault.
    render(<LogLine ts="14:02:11" level="info" message="" />);
    expect(screen.getByText("(no message)")).toBeDefined();
  });

  it("renders the trailing slot", () => {
    const { container } = render(
      <LogLine ts="14:02:11" message="x">
        <button type="button">Ask</button>
      </LogLine>,
    );
    expect(cell(container, "trailing").textContent).toBe("Ask");
  });

  it("omits the trailing slot when it resolved to nothing", () => {
    // `{canAsk && <AskChip/>}` on every line of a stream: an empty box per line
    // is a column of gaps down the right-hand side.
    const { container } = render(
      <LogLine ts="14:02:11" message="x">
        {false}
      </LogLine>,
    );
    expect(cell(container, "trailing")).toBeNull();
  });

  it("lays the fields out as the cells of one row, so a copied line comes out as one line", () => {
    // A browser copies each item of a flex or grid row onto a line of its own:
    // the line came out of the clipboard as four lines. Table cells come out
    // tab-separated on one. (Seen in Chromium; jsdom does no layout, so this
    // pins the structure and the live run checks the copy.)
    const { container } = render(<LogLine ts="14:02:11" source="app" level="error" message="x" />);
    expect(container.querySelector(".logline")?.classList.contains("table")).toBe(true);
    for (const slot of ["ts", "source", "level", "message"])
      expect(cell(container, slot).classList.contains("table-cell"), slot).toBe(true);
  });

  it("wears the class the stylesheet hovers", () => {
    const { container } = render(<LogLine ts="14:02:11" message="x" />);
    expect(container.querySelector(".logline")).not.toBeNull();
  });

  it("forwards className onto the line", () => {
    const { container } = render(<LogLine ts="14:02:11" message="x" className="extra" />);
    expect(container.querySelector(".logline.extra")).not.toBeNull();
  });

  it("keeps a message's runs of spaces, and leaves wrapping to the screen", () => {
    // (#1051) `nowrap` and `normal` both collapse spaces, so a stack trace or an
    // aligned table lost its shape on screen and in the copy. Collapse alone is
    // set here; whether a long line wraps is the screen's Wrap, inherited.
    const { container } = render(<LogLine ts="14:02:11" message="    at reconcile   retry=3" />);
    const message = cell(container, "message");
    expect(message.className).toContain("[white-space-collapse:preserve]");
    expect(message.className).not.toMatch(/(^|\s)whitespace-/);
  });
});
