import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const core = vi.hoisted(() => ({
  getMcpToken: vi.fn(),
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
 * with no prefix (`Token::generate`, `crates/mcp/src/auth.rs`) — `srl_` here
 * is the pane's own cosmetic label, not part of the wire value, but the
 * fixture carries it anyway so a reader of this file sees the same shape the
 * masked-length test below reasons about (`srl_${TOKEN}`).
 */
const TOKEN = "srl_4f9a2c7e1b6d80f3c9a1e7b4f2d6c8035a9e1c7b4f2d6c8035f9c1a7e4b2d6f8";

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
    core.rotateMcpToken.mockResolvedValue(TOKEN);
    core.revokeMcpToken.mockResolvedValue(undefined);
  });

  it("masks the token until the reader asks for it", async () => {
    render(<McpServer />);
    expect(await screen.findByText(/^srl_•+$/)).toBeTruthy();
    expect(screen.queryByText(TOKEN)).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Reveal" }));
    expect(screen.getByText(TOKEN)).toBeTruthy();
  });

  it("does not tell the reader how long the secret is", async () => {
    render(<McpServer />);
    const masked = (await screen.findByText(/^srl_•+$/)).textContent ?? "";
    expect(masked.length).not.toBe(`srl_${TOKEN}`.length);
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
    expect(screen.getByText(/^srl_•+$/)).toBeTruthy();
  });

  it("copies the real token, not the mask", async () => {
    const writeText = stubClipboard();
    render(<McpServer />);
    await userEvent.click(await screen.findByRole("button", { name: "Copy" }));
    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith(TOKEN);
  });

  it("rotates through rotateMcpToken and drops the old value from view", async () => {
    const ROTATED = "srl_ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff";
    core.rotateMcpToken.mockResolvedValue(ROTATED);
    render(<McpServer />);
    await userEvent.click(await screen.findByRole("button", { name: "Reveal" }));
    expect(screen.getByText(TOKEN)).toBeTruthy();
    await userEvent.click(screen.getByRole("button", { name: "Rotate" }));
    expect(core.rotateMcpToken).toHaveBeenCalledTimes(1);
    // Rotating re-masks: the reader saw the OLD value, not the new one, so
    // the pane must not carry it forward revealed.
    expect(await screen.findByText(/^srl_•+$/)).toBeTruthy();
    expect(screen.queryByText(TOKEN)).toBeNull();
    expect(screen.queryByText(ROTATED)).toBeNull();
  });

  it("revokes through revokeMcpToken and shows the server as not running", async () => {
    render(<McpServer />);
    await screen.findByText(/^srl_•+$/);
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
    render(<McpServer />);
    expect(await screen.findByText("not running")).toBeTruthy();
    expect(screen.queryByText("running")).toBeNull();
    expect(screen.queryByRole("button", { name: "Reveal" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Copy" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Rotate" })).toBeNull();
    expect(screen.queryByRole("button", { name: "Revoke" })).toBeNull();
    expect(screen.getByText(/not accepting connections/i)).toBeTruthy();
  });
});
