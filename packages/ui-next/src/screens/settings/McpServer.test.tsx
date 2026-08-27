import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const core = vi.hoisted(() => ({
  getMcpToken: vi.fn(),
  mcpHttpStatus: vi.fn(),
  rotateMcpToken: vi.fn(),
  revokeMcpToken: vi.fn(),
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
  });

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
    expect(screen.getByText(/not currently listening/i)).toBeTruthy();
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
});
