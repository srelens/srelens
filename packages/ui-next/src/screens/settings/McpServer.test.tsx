import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const core = vi.hoisted(() => ({
  getMcpToken: vi.fn(),
  mcpHttpStatus: vi.fn(),
  rotateMcpToken: vi.fn(),
  revokeMcpToken: vi.fn(),
  startMcpHttp: vi.fn(),
  stopMcpHttp: vi.fn(),
  loadMcpSettings: vi.fn(),
  saveMcpSettings: vi.fn(),
}));
vi.mock("@srelens/core", async (orig) => ({
  ...(await orig<typeof import("@srelens/core")>()),
  ...core,
}));

import { McpServer } from "./McpServer";

/**
 * A realistic bearer value. The real backend mints a 64-character hex string
 * with NO prefix (`Token::generate`, `crates/mcp/src/auth.rs`) — an earlier
 * version of this fixture baked a `srl_` label into the value itself, which
 * is exactly why no test caught the mask inventing that same label: both
 * states agreed by construction. This one carries no label, matching what
 * `getMcpToken()` actually returns in production.
 */
const TOKEN = "4f9a2c7e1b6d80f3c9a1e7b4f2d6c8035a9e1c7b4f2d6c8035f9c1a7e4b2d6f8";

/** A second, equally realistic value — distinct from TOKEN — for rotation. */
const ROTATED = "f".repeat(64);

/** What a running loopback server's own status call answers with. */
const STATUS_URL = "http://127.0.0.1:8765/mcp";

/**
 * A port that is NOT the default. Every address assertion here uses it, so a
 * pane that hardcodes 8765 — which is what the default would have hidden —
 * fails rather than agreeing with the fixture by coincidence.
 */
const PORT = 9411;

/** What that port's stopped server would bind, and what a start returns. */
const PORT_URL = `http://127.0.0.1:${PORT}/mcp`;

/** jsdom ships no clipboard at all, so there is nothing to spy on. */
function stubClipboard() {
  const writeText = vi.fn().mockResolvedValue(undefined);
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
  return writeText;
}

describe("McpServer", () => {
  beforeEach(() => {
    vi.clearAllMocks();
    core.getMcpToken.mockResolvedValue(TOKEN);
    core.mcpHttpStatus.mockResolvedValue(STATUS_URL);
    core.rotateMcpToken.mockResolvedValue(TOKEN);
    core.revokeMcpToken.mockResolvedValue(undefined);
    core.startMcpHttp.mockResolvedValue(PORT_URL);
    core.stopMcpHttp.mockResolvedValue(undefined);
    core.loadMcpSettings.mockReturnValue({ enabled: false, port: PORT });
    core.saveMcpSettings.mockReturnValue(undefined);
  });

  /** The one element that carries the pane's claim about the address. */
  function address(): string {
    return screen.getByTestId("mcp-address").textContent ?? "";
  }

  it("masks the token until the reader asks for it", async () => {
    render(<McpServer />);
    expect(await screen.findByText(/^•+$/)).toBeTruthy();
    expect(screen.queryByText(TOKEN)).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Reveal" }));
    expect(screen.getByText(TOKEN)).toBeTruthy();
  });

  it("does not tell the reader how long the secret is", async () => {
    render(<McpServer />);
    const masked = (await screen.findByText(/^•+$/)).textContent ?? "";
    expect(masked.length).not.toBe(TOKEN.length);
  });

  it("masked and revealed agree on format — neither carries a label the other lacks", async () => {
    render(<McpServer />);
    const masked = (await screen.findByText(/^•+$/)).textContent ?? "";
    expect(masked).not.toContain("srl_");
    await userEvent.click(screen.getByRole("button", { name: "Reveal" }));
    const revealed = screen.getByText(TOKEN).textContent ?? "";
    expect(revealed).not.toContain("srl_");
    expect(revealed).toBe(TOKEN);
  });

  it("warns what rotating costs before it is done", async () => {
    render(<McpServer />);
    expect(await screen.findByText(/drops in-flight requests/i)).toBeTruthy();
  });

  it("offers no client list, and says why", async () => {
    render(<McpServer />);
    expect(screen.queryByText(/claude code|cursor/i)).toBeNull();
    expect(await screen.findByText(/which clients are connected/i)).toBeTruthy();
  });

  it("hides the token again once the reader is done looking", async () => {
    render(<McpServer />);
    await userEvent.click(await screen.findByRole("button", { name: "Reveal" }));
    expect(screen.getByText(TOKEN)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Hide" }));
    expect(screen.queryByText(TOKEN)).toBeNull();
    expect(screen.getByText(/^•+$/)).toBeTruthy();
  });

  it("copies the real token, not the mask", async () => {
    const writeText = stubClipboard();
    render(<McpServer />);
    await userEvent.click(await screen.findByRole("button", { name: "Copy" }));
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith(TOKEN);
  });

  it("rotates through rotateMcpToken and drops the old value from view", async () => {
    core.rotateMcpToken.mockResolvedValue(ROTATED);
    render(<McpServer />);
    await userEvent.click(await screen.findByRole("button", { name: "Reveal" }));
    expect(screen.getByText(TOKEN)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Rotate" }));
    expect(core.rotateMcpToken).toHaveBeenCalledTimes(1);
    // Rotating re-masks: the reader saw the OLD value, not the new one, so
    // the pane must not carry it forward revealed.
    expect(await screen.findByText(/^•+$/)).toBeTruthy();
    expect(screen.queryByText(TOKEN)).toBeNull();
    expect(screen.queryByText(ROTATED)).toBeNull();
  });

  it("revokes through revokeMcpToken and shows the server as not running", async () => {
    render(<McpServer />);
    await screen.findByText(/^•+$/);
    await userEvent.click(screen.getByRole("button", { name: "Revoke" }));
    expect(core.revokeMcpToken).toHaveBeenCalledTimes(1);
    expect(await screen.findByText("not running")).toBeTruthy();
    expect(screen.queryByText("running")).toBeNull();
    expect(screen.queryByRole("button", { name: "Reveal" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Rotate" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Revoke" })).toBeNull();
  });

  it("says the server is not running when there is no token yet, and offers no dead controls", async () => {
    core.getMcpToken.mockResolvedValue(null);
    core.mcpHttpStatus.mockResolvedValue(null);
    render(<McpServer />);
    expect(await screen.findByText("not running")).toBeTruthy();
    expect(screen.queryByText("running")).toBeNull();
    expect(screen.queryByRole("button", { name: "Reveal" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Copy" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Rotate" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Revoke" })).toBeNull();
    expect(screen.getByTestId("no-token-note").textContent).toMatch(/nothing to reveal/i);
  });

  /**
   * "No bearer token has been generated, so the MCP server is not accepting
   * connections" printed regardless of `statusRead` — the badge-level version
   * of this was fixed in an earlier round and it survived in prose. It is also
   * not implied by the absence of a token: `mcp_http_start`
   * (`apps/desktop/src-tauri/src/mcp.rs:206-212`) MINTS one when none exists,
   * so a server can be brought up from this state. The sentence now says only
   * what the absent token establishes.
   */
  it("makes no claim about the server from the absence of a token", async () => {
    core.getMcpToken.mockResolvedValue(null);
    // A status read that never settles, so nothing on screen may assert
    // anything about whether the server is listening.
    core.mcpHttpStatus.mockReturnValue(new Promise<string | null>(() => {}));
    render(<McpServer />);
    const note = await screen.findByTestId("no-token-note");
    expect(note.textContent).not.toMatch(/not accepting connections/i);
    expect(note.textContent).not.toMatch(/not (currently )?listening/i);
    expect(screen.queryByText("not running")).toBeNull();
  });

  it("does not read the badge as running just because a token exists", async () => {
    core.getMcpToken.mockResolvedValue(TOKEN);
    core.mcpHttpStatus.mockResolvedValue(null);
    render(<McpServer />);
    expect(await screen.findByText("not running")).toBeTruthy();
    expect(screen.queryByText("running")).toBeNull();
    // The token itself is still a real, actionable credential — rotate and
    // revoke work whether or not the listener is up — so its controls stay.
    expect(screen.getByRole("button", { name: "Reveal" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Rotate" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Revoke" })).toBeTruthy();
    expect(address()).toMatch(/not listening/i);
  });

  /**
   * Round 2 finding 1 (first half): a rejecting initial read must not fall
   * through to the "no token" branch — that sentence is a claim about a
   * DEFINITE fact, and a failed read never established one.
   */
  it("does not claim no token exists when the initial read fails", async () => {
    core.getMcpToken.mockRejectedValue(new Error("boom"));
    render(<McpServer />);
    expect(await screen.findByText(/could not be read/i)).toBeTruthy();
    expect(screen.queryByTestId("no-token-note")).toBeNull();
    // Unknown, not "definitely absent": no controls implying either.
    expect(screen.queryByRole("button", { name: "Reveal" })).toBeNull();
  });

  /**
   * Round 2 finding 1 (second half): the two reads must not share a fate.
   * A rejecting status check must not discard a token that was fetched
   * successfully in its own, independent effect.
   */
  it("does not discard a fetched token when the status check fails", async () => {
    core.getMcpToken.mockResolvedValue(TOKEN);
    core.mcpHttpStatus.mockRejectedValue(new Error("status boom"));
    render(<McpServer />);
    expect(await screen.findByText(/could not be checked/i)).toBeTruthy();
    // The token is still known and usable, despite the unrelated failure.
    expect(await screen.findByText(/^•+$/)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Reveal" }));
    expect(screen.getByText(TOKEN)).toBeTruthy();
    // The badge asserts neither state while the status is genuinely unknown.
    expect(screen.queryByText("running")).toBeNull();
    expect(screen.queryByText("not running")).toBeNull();
  });

  /**
   * The stale-response guard, pinned the way the sibling branch (a late
   * answer is never believed — `cachedResource.test.tsx`) learned to: the
   * stale value resolves AFTER the newer state is already set. Resolving in
   * mount order would prove nothing — that's the passing case even with no
   * guard at all.
   */
  it("does not let a late status response overwrite an accurate post-revoke state", async () => {
    core.getMcpToken.mockResolvedValue(TOKEN);
    let answerStatus!: (url: string | null) => void;
    core.mcpHttpStatus.mockImplementation(
      () =>
        new Promise<string | null>((resolve) => {
          answerStatus = resolve;
        }),
    );
    render(<McpServer />);

    // The token read settles (buttons appear); the mount's own status read
    // is still in flight — the Revoke button never waits on it.
    await userEvent.click(await screen.findByRole("button", { name: "Revoke" }));
    expect(core.revokeMcpToken).toHaveBeenCalledTimes(1);

    // Revoke's own, authoritative write has already landed.
    expect(await screen.findByText("not running")).toBeTruthy();

    // NOW the stale in-flight response from mount arrives, claiming the
    // server IS listening — strictly after the newer, correct state.
    // Flushed inside `act`: an update scheduled outside it would never reach
    // the DOM before the assertion below, and this test would pass with the
    // guard removed.
    await act(async () => {
      answerStatus(STATUS_URL);
      await Promise.resolve();
    });

    expect(screen.queryByText("running")).toBeNull();
    expect(screen.getByText("not running")).toBeTruthy();
  });

  // ---- Starting and stopping the server (the whole of it, from here) ----

  /**
   * The pane read status and managed a token and did NOTHING else: it never
   * called `startMcpHttp` or `stopMcpHttp`, and the only callers of either were
   * classic's — `App.tsx`'s auto-start effect and `McpSettingsSection`. Since
   * `main.tsx` mounts one tree or the other, a reader in the new design could
   * not start the server at all, and the empty-token note pointed them at
   * "when the loopback HTTP server starts" as though that were something they
   * could make happen.
   */
  describe("starting and stopping", () => {
    it("starts the server on the port the reader actually persisted", async () => {
      core.getMcpToken.mockResolvedValue(null);
      core.mcpHttpStatus.mockResolvedValue(null);
      render(<McpServer />);
      await userEvent.click(await screen.findByRole("button", { name: "Start server" }));
      expect(core.startMcpHttp).toHaveBeenCalledTimes(1);
      expect(core.startMcpHttp).toHaveBeenCalledWith(PORT);
    });

    it("shows the server running at the URL the start itself returned", async () => {
      core.getMcpToken.mockResolvedValue(null);
      core.mcpHttpStatus.mockResolvedValue(null);
      render(<McpServer />);
      await userEvent.click(await screen.findByRole("button", { name: "Start server" }));
      expect(await screen.findByText("running")).toBeTruthy();
      expect(address()).toContain(PORT_URL);
      expect(address()).toMatch(/listening at/i);
    });

    it("re-reads the token after a start, because the first start mints one", async () => {
      core.getMcpToken.mockResolvedValueOnce(null).mockResolvedValue(TOKEN);
      core.mcpHttpStatus.mockResolvedValue(null);
      render(<McpServer />);
      expect(await screen.findByTestId("no-token-note")).toBeTruthy();
      await userEvent.click(screen.getByRole("button", { name: "Start server" }));
      expect(await screen.findByRole("button", { name: "Reveal" })).toBeTruthy();
      expect(screen.queryByTestId("no-token-note")).toBeNull();
    });

    it("stops the server through stopMcpHttp and says so", async () => {
      render(<McpServer />);
      await userEvent.click(await screen.findByRole("button", { name: "Stop server" }));
      expect(core.stopMcpHttp).toHaveBeenCalledTimes(1);
      expect(await screen.findByText("not running")).toBeTruthy();
      expect(screen.queryByText("running")).toBeNull();
      // The token is untouched by a stop — it is the credential, not the
      // listener.
      expect(screen.getByRole("button", { name: "Reveal" })).toBeTruthy();
    });

    it("remembers the choice the same way classic does, so one setting means one thing", async () => {
      render(<McpServer />);
      await userEvent.click(await screen.findByRole("button", { name: "Stop server" }));
      expect(core.saveMcpSettings).toHaveBeenCalledWith({ enabled: false, port: PORT });
      await userEvent.click(await screen.findByRole("button", { name: "Start server" }));
      expect(core.saveMcpSettings).toHaveBeenCalledWith({ enabled: true, port: PORT });
    });

    it("reports a refused start instead of claiming the server came up", async () => {
      core.mcpHttpStatus.mockResolvedValue(null);
      core.startMcpHttp.mockRejectedValue(new Error("address already in use"));
      render(<McpServer />);
      await userEvent.click(await screen.findByRole("button", { name: "Start server" }));
      expect(await screen.findByText(/could not be started/i)).toBeTruthy();
      expect(screen.getByText(/address already in use/i)).toBeTruthy();
      // `start_server` binds before it records anything as running, so a
      // rejection is a KNOWN not-running, not a guess.
      expect(screen.getByText("not running")).toBeTruthy();
      expect(screen.queryByText("running")).toBeNull();
      expect(core.saveMcpSettings).toHaveBeenLastCalledWith({ enabled: false, port: PORT });
    });

    it("offers no start or stop while it has not established which one it would be", async () => {
      // A status read that never settles: the pane knows neither state, and a
      // control has to name one of them.
      core.mcpHttpStatus.mockReturnValue(new Promise<string | null>(() => {}));
      render(<McpServer />);
      expect(await screen.findByRole("button", { name: "Reveal" })).toBeTruthy();
      expect(screen.queryByRole("button", { name: /start server/i })).toBeNull();
      expect(screen.queryByRole("button", { name: /stop server/i })).toBeNull();
      expect(screen.queryByTestId("mcp-address")).toBeNull();
    });

    /**
     * The mutation pass killed the test that stood here. It resolved the
     * mount's status promise a SECOND time to play a stale response — which a
     * settled promise ignores, so it passed with `establishStatus`'s sequence
     * bump removed. And the scenario itself cannot happen: the Start button
     * does not exist until the mount read is `ready`, so a start can never
     * race it. (`revoke()` can, because its button appears on the TOKEN read
     * — that race has its own test above, and it is the one the shared guard
     * is load-bearing for.)
     *
     * What is real, and what this asserts instead: a start takes the URL the
     * start itself returned and makes no second status read. There is then no
     * later answer to be raced by, rather than a guard against one.
     */
    it("takes the URL the start returned instead of reading the status again", async () => {
      core.mcpHttpStatus.mockResolvedValue(null);
      render(<McpServer />);
      await userEvent.click(await screen.findByRole("button", { name: "Start server" }));
      expect(await screen.findByText("running")).toBeTruthy();
      expect(address()).toContain(PORT_URL);
      // Once, at mount. A start that re-read would be asking for a fact it was
      // just handed, and inviting exactly the stale-answer race it then has to
      // guard.
      expect(core.mcpHttpStatus).toHaveBeenCalledTimes(1);
      await userEvent.click(screen.getByRole("button", { name: "Stop server" }));
      expect(await screen.findByText("not running")).toBeTruthy();
      expect(core.mcpHttpStatus).toHaveBeenCalledTimes(1);
    });

    it("points the empty-token note at the control this pane actually has", async () => {
      core.getMcpToken.mockResolvedValue(null);
      core.mcpHttpStatus.mockResolvedValue(null);
      render(<McpServer />);
      const note = await screen.findByTestId("no-token-note");
      expect(note.textContent).toMatch(/start/i);
      // And the action it names is really here, in this tree, one click away.
      expect(screen.getByRole("button", { name: "Start server" })).toBeTruthy();
    });
  });

  // ---- The address (finding 3) ----------------------------------------

  /**
   * `mcpHttpStatus()` returns the running server's URL and the pane threw it
   * away into a boolean, then printed a hardcoded `127.0.0.1:8765` in the
   * head. A reader who set a non-default port in classic kept a server on that
   * port, and this pane advertised an endpoint with nothing on it.
   */
  describe("the address", () => {
    it("renders the URL the running server reported, not one of its own", async () => {
      // The persisted port disagrees with where the server is actually bound
      // — exactly the state a port change leaves behind. The live read wins.
      core.loadMcpSettings.mockReturnValue({ enabled: true, port: 8765 });
      core.mcpHttpStatus.mockResolvedValue(PORT_URL);
      render(<McpServer />);
      expect(await screen.findByText("running")).toBeTruthy();
      expect(address()).toContain(PORT_URL);
      expect(address()).not.toContain("8765");
    });

    it("reads the persisted port for a stopped server rather than guessing", async () => {
      core.mcpHttpStatus.mockResolvedValue(null);
      render(<McpServer />);
      expect(await screen.findByText("not running")).toBeTruthy();
      expect(address()).toContain(PORT_URL);
      expect(address()).not.toContain("8765");
    });

    it("names no address anywhere else, so there is one claim to keep true", async () => {
      core.mcpHttpStatus.mockResolvedValue(PORT_URL);
      render(<McpServer />);
      await screen.findByText("running");
      const stray = Array.from(document.querySelectorAll("*")).filter(
        (node) =>
          node.children.length === 0 &&
          /127\.0\.0\.1|8765/.test(node.textContent ?? "") &&
          !screen.getByTestId("mcp-address").contains(node),
      );
      expect(stray.map((n) => n.textContent)).toEqual([]);
    });
  });
});
