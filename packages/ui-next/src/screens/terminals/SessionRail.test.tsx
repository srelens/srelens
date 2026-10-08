import { describe, it, expect, vi, afterEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { TerminalSessionRow } from "../../lib/sessions";
import { SessionRail, SESSION_RAIL_WIDTH, sessionRailHead } from "./SessionRail";

/** A session row with every field a real one carries, overridable per test. */
function row(overrides: Partial<TerminalSessionRow> = {}): TerminalSessionRow {
  return {
    id: 1,
    kind: "pod",
    title: "checkout-api-5c8b7f2d9-mk3wl · api",
    context: "prod-eu",
    namespace: "checkout",
    state: "attached",
    startedAt: 0,
    lastOutputAt: 0,
    ...overrides,
  };
}

afterEach(() => {
  vi.useRealTimers();
});

describe("SessionRail", () => {
  it("draws a row per session, its kind, and how long since it last spoke", () => {
    const now = new Date("2026-08-25T00:10:00Z").getTime();
    vi.useFakeTimers();
    vi.setSystemTime(now);

    const sessions = [
      row({ id: 1, kind: "pod", title: "checkout-api-5c8b7f2d9-mk3wl · api", state: "attached", lastOutputAt: now - 12_000 }),
      row({ id: 2, kind: "node", title: "eu-w4-c3-standard-a1", state: "attached", lastOutputAt: now - 4 * 60_000 }),
      row({ id: 3, kind: "local", title: "prod-eu context shell", state: "idle", lastOutputAt: now - 22 * 60_000 }),
    ];

    render(<SessionRail sessions={sessions} activeId={1} onSelect={vi.fn()} onNewSession={vi.fn()} onRename={() => {}} onDetach={() => {}} />);

    expect(screen.getByText("checkout-api-5c8b7f2d9-mk3wl · api")).not.toBeNull();
    expect(screen.getByText("eu-w4-c3-standard-a1")).not.toBeNull();
    expect(screen.getByText("prod-eu context shell")).not.toBeNull();
    // §14's own prose for the kind, off `SESSION_KIND_LABEL` — `pod` beside a
    // pod's own name says nothing that `pod exec` does not say better.
    expect(screen.getByText("pod exec · 12s")).not.toBeNull();
    expect(screen.getByText("node shell · 4m")).not.toBeNull();
    expect(screen.getByText("local · 22m")).not.toBeNull();
  });

  it("gives the session's name a line of its own", () => {
    // The rail is 230px and a row carries four things: a glyph, the name, the
    // kind and idle time, and a state pill. Laid out on ONE line the name got
    // 44px of 229 — two pods from the same namespace were told apart by four
    // characters. §14 shows these stacked, and the widths are why.
    //
    // jsdom lays nothing out, so this asserts the mechanism: the name is not a
    // flex sibling of the kind and the pill. Fourth time on this migration a
    // width problem has been invisible to the suite.
    const only = row();
    render(<SessionRail sessions={[only]} activeId={only.id} onSelect={() => {}} onNewSession={() => {}} onRename={() => {}} onDetach={() => {}} />);
    const name = screen.getByText(only.title);
    const kind = screen.getByText(/pod exec ·/);
    expect(name.parentElement).not.toBe(kind.parentElement);
  });

  it("marks the active session and no other, with more than one row on screen", () => {
    const sessions = [
      row({ id: 1, title: "first" }),
      row({ id: 2, title: "second" }),
      row({ id: 3, title: "third" }),
    ];
    render(<SessionRail sessions={sessions} activeId={2} onSelect={vi.fn()} onNewSession={vi.fn()} onRename={() => {}} onDetach={() => {}} />);

    const active = screen.getByRole("button", { current: true });
    expect(active.textContent).toContain("second");
    // Only one row carries the marker.
    expect(screen.getAllByRole("button", { current: true })).toHaveLength(1);
    const others = screen.getAllByRole("button").filter((b) => b !== active);
    expect(others.every((b) => b.getAttribute("aria-current") !== "true")).toBe(true);
  });

  it("keeps a closed session listed, reading Closed rather than vanishing", () => {
    const sessions = [
      row({ id: 1, title: "still going", state: "attached" }),
      row({ id: 2, title: "gave up", state: "closed", error: "exit 137" }),
    ];
    render(<SessionRail sessions={sessions} activeId={1} onSelect={vi.fn()} onNewSession={vi.fn()} onRename={() => {}} onDetach={() => {}} />);

    expect(screen.getByText("gave up")).not.toBeNull();
    expect(screen.getByText("Closed")).not.toBeNull();
  });

  it("draws attached, idle and closed with three different verdicts, not one collapsed state", () => {
    const sessions = [
      row({ id: 1, title: "a", state: "attached" }),
      row({ id: 2, title: "b", state: "idle" }),
      row({ id: 3, title: "c", state: "closed" }),
    ];
    const { container } = render(
      <SessionRail sessions={sessions} activeId={null} onSelect={vi.fn()} onNewSession={vi.fn()} onRename={() => {}} onDetach={() => {}} />,
    );

    expect(container.querySelectorAll("[data-kind='success']")).toHaveLength(1);
    expect(container.querySelectorAll("[data-kind='warning']")).toHaveLength(1);
    expect(container.querySelectorAll("[data-kind='neutral']")).toHaveLength(1);
  });

  it("selecting a row hands the screen that session's id, not any other's", async () => {
    const user = userEvent.setup();
    const onSelect = vi.fn();
    const sessions = [
      row({ id: 11, title: "alpha" }),
      row({ id: 22, title: "bravo" }),
      row({ id: 33, title: "charlie" }),
    ];
    render(<SessionRail sessions={sessions} activeId={11} onSelect={onSelect} onNewSession={vi.fn()} onRename={() => {}} onDetach={() => {}} />);

    await user.click(screen.getByText("charlie"));

    expect(onSelect).toHaveBeenCalledExactlyOnceWith(33);
  });

  /**
   * Three shells opened from the status bar are all "Local shell". A name of
   * the reader's own is what tells them apart, and a Detach on the row is what
   * lets one be closed without first putting it on screen.
   */
  describe("a row's own actions", () => {
    function rail(overrides: Partial<Parameters<typeof SessionRail>[0]> = {}) {
      const props = {
        sessions: [row({ id: 11, title: "alpha" }), row({ id: 22, title: "bravo" })],
        activeId: 11,
        onSelect: vi.fn(),
        onRename: vi.fn(),
        onDetach: vi.fn(),
        onNewSession: vi.fn(),
        ...overrides,
      };
      render(<SessionRail {...props} />);
      return props;
    }

    it("detaches the row it is on, without selecting it", async () => {
      const props = rail();
      await userEvent.click(screen.getByRole("button", { name: "Detach bravo" }));
      expect(props.onDetach).toHaveBeenCalledExactlyOnceWith(22);
      expect(props.onSelect).not.toHaveBeenCalled();
    });

    it("renames the row it is on to what was typed, on Enter", async () => {
      const props = rail();
      await userEvent.click(screen.getByRole("button", { name: "Rename bravo" }));
      const field = screen.getByRole("textbox", { name: "Rename bravo" });
      // The old name is there, selected, so typing replaces it.
      expect((field as HTMLInputElement).value).toBe("bravo");
      await userEvent.keyboard("drain watch{Enter}");
      expect(props.onRename).toHaveBeenCalledExactlyOnceWith(22, "drain watch");
      expect(screen.queryByRole("textbox")).toBeNull();
      expect(props.onSelect).not.toHaveBeenCalled();
    });

    it("keeps what was typed when the field is left", async () => {
      const props = rail();
      await userEvent.click(screen.getByRole("button", { name: "Rename alpha" }));
      await userEvent.keyboard("logs");
      await userEvent.click(screen.getByText("bravo"));
      expect(props.onRename).toHaveBeenCalledExactlyOnceWith(11, "logs");
    });

    it("leaves focus where the reader moved it when the field is left by Tab", async () => {
      rail();
      await userEvent.click(screen.getByRole("button", { name: "Rename alpha" }));
      await userEvent.tab();
      // Onward to the next row, not pulled back to the one just renamed.
      expect(document.activeElement?.textContent).toContain("bravo");
    });

    it("hands focus back to the row when the field is closed with Enter", async () => {
      rail();
      await userEvent.click(screen.getByRole("button", { name: "Rename bravo" }));
      await userEvent.keyboard("{Enter}");
      expect(document.activeElement?.textContent).toContain("bravo");
      expect(document.activeElement?.tagName).toBe("BUTTON");
    });

    it("keeps the old name on Escape, and says nothing to the store", async () => {
      const props = rail();
      await userEvent.click(screen.getByRole("button", { name: "Rename bravo" }));
      await userEvent.keyboard("oops{Escape}");
      expect(props.onRename).not.toHaveBeenCalled();
      expect(screen.queryByRole("textbox")).toBeNull();
      expect(screen.getByText("bravo")).toBeDefined();
    });

    it("opens the field from a double-click on the row, and from F2", async () => {
      rail();
      await userEvent.dblClick(screen.getByText("bravo"));
      expect(screen.getByRole("textbox", { name: "Rename bravo" })).toBeDefined();
      await userEvent.keyboard("{Escape}");
      // Escape hands the keyboard back to the row it came from.
      expect(document.activeElement?.textContent).toContain("bravo");
      await userEvent.keyboard("{F2}");
      expect(screen.getByRole("textbox", { name: "Rename bravo" })).toBeDefined();
    });

    it("offers no second rename or detach while a name is being typed", async () => {
      rail();
      await userEvent.click(screen.getByRole("button", { name: "Rename bravo" }));
      expect(screen.queryByRole("button", { name: "Detach bravo" })).toBeNull();
      // The other row is untouched.
      expect(screen.getByRole("button", { name: "Detach alpha" })).toBeDefined();
    });

    it("nests no button inside another", () => {
      const { container } = render(
        <SessionRail sessions={[row()]} activeId={1} onSelect={() => {}} onNewSession={() => {}} onRename={() => {}} onDetach={() => {}} />,
      );
      expect(container.querySelector("button button")).toBeNull();
    });
  });

  it("an empty rail offers New session and says why it's empty", async () => {
    const user = userEvent.setup();
    const onNewSession = vi.fn();
    render(<SessionRail sessions={[]} activeId={null} onSelect={vi.fn()} onNewSession={onNewSession} onRename={() => {}} onDetach={() => {}} />);

    expect(screen.getByText(/no sessions/i)).not.toBeNull();
    const button = screen.getByRole("button", { name: "New session" });
    await user.click(button);
    expect(onNewSession).toHaveBeenCalledOnce();
  });

  it("names no width of its own beyond the exported constant", () => {
    expect(SESSION_RAIL_WIDTH).toBe(230);
  });
});

describe("sessionRailHead", () => {
  it("counts only attached sessions, not idle or closed ones", () => {
    const sessions = [
      row({ id: 1, state: "attached" }),
      row({ id: 2, state: "attached" }),
      row({ id: 3, state: "idle" }),
      row({ id: 4, state: "closed" }),
    ];
    expect(sessionRailHead(sessions)).toBe("Sessions · 2 attached");
  });

  it("reads zero attached rather than omitting the count", () => {
    expect(sessionRailHead([row({ id: 1, state: "closed" })])).toBe("Sessions · 0 attached");
  });

  it("reads zero attached for an empty rail", () => {
    expect(sessionRailHead([])).toBe("Sessions · 0 attached");
  });
});
