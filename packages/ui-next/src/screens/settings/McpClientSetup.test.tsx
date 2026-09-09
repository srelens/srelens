import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { McpClientSetup } from "./McpClientSetup";
const core = vi.hoisted(() => ({ installSrelensCli: vi.fn(), srelensCliStatus: vi.fn() }));
vi.mock("@srelens/core", async original => ({ ...await original<object>(), ...core }));
const URL = "http://127.0.0.1:9411/mcp";
const TOKEN = "a".repeat(64);
const PLATFORM = navigator.platform;
beforeEach(() => {
  vi.clearAllMocks();
  core.srelensCliStatus.mockResolvedValue({ installed: false, path: "/home/user/.local/bin/srelens", links_to: null, on_path: false });
  core.installSrelensCli.mockResolvedValue("/home/user/.local/bin/srelens");
});
afterEach(() => Object.defineProperty(navigator, "platform", { configurable: true, value: PLATFORM }));
it("installs the CLI and refreshes its PATH status", async () => {
  const user = userEvent.setup(); render(<McpClientSetup url={URL} token={TOKEN} />);
  await screen.findByRole("button", { name: "Install srelens CLI" });
  core.srelensCliStatus.mockResolvedValue({ installed: true, path: "/home/user/.local/bin/srelens", on_path: false });
  await user.click(screen.getByRole("button", { name: "Install srelens CLI" }));
  expect(core.installSrelensCli).toHaveBeenCalledOnce();
  expect(await screen.findByRole("button", { name: "Reinstall srelens CLI" })).toBeTruthy();
  expect(screen.getByText(/Add.*to PATH/).textContent).toContain("/home/user/.local/bin");
});
it("generates current HTTP config, masks its token, and copies the usable value", async () => {
  const user = userEvent.setup();
  const copy = vi.spyOn(navigator.clipboard, "writeText").mockResolvedValue();
  const view = render(<McpClientSetup url={URL} token={TOKEN} />);
  await user.selectOptions(screen.getByLabelText("MCP client"), "cursor");
  await user.selectOptions(screen.getByLabelText("Transport"), "http");
  expect(screen.getByTestId("mcp-client-config").textContent).toContain(URL);
  expect(screen.getByTestId("mcp-client-config").textContent).not.toContain(TOKEN);
  await user.click(screen.getByRole("button", { name: "Copy configuration" }));
  expect(JSON.parse(copy.mock.calls[0][0]).mcpServers.srelens.headers.Authorization).toBe(`Bearer ${TOKEN}`);
  view.rerender(<McpClientSetup url="http://127.0.0.1:9511/mcp" token={"b".repeat(64)} />);
  await user.click(screen.getByRole("button", { name: "Copy configuration" }));
  expect(copy.mock.lastCall?.[0]).toContain("9511");
  expect(copy.mock.lastCall?.[0]).toContain("b".repeat(64));
});
it("requires known connection details before enabling either transport", async () => {
  const user = userEvent.setup(); render(<McpClientSetup url={null} token={null} />);
  await user.selectOptions(screen.getByLabelText("Transport"), "http");
  expect((screen.getByRole("button", { name: "Copy configuration" }) as HTMLButtonElement).disabled).toBe(true);
  await user.selectOptions(screen.getByLabelText("Transport"), "stdio");
  expect(screen.queryByTestId("mcp-client-config")).toBeNull();
  expect((screen.getByRole("button", { name: "Copy configuration" }) as HTMLButtonElement).disabled).toBe(true);
});
it("uses an installed Unix CLI's absolute path even when it is not on PATH", async () => {
  const executable = "/home/user/.local/bin/srelens";
  core.srelensCliStatus.mockResolvedValue({ installed: true, path: executable, links_to: "/Applications/srelens", on_path: false });
  render(<McpClientSetup url={URL} token={TOKEN} />);
  expect((await screen.findByTestId("mcp-client-config")).textContent).toContain(executable);
  expect((screen.getByRole("button", { name: "Copy configuration" }) as HTMLButtonElement).disabled).toBe(false);
});
it("reports installation failure and allows retry", async () => {
  core.installSrelensCli.mockRejectedValueOnce(new Error("permission denied"));
  const user = userEvent.setup(); render(<McpClientSetup url={URL} token={TOKEN} />);
  await user.click(await screen.findByRole("button", { name: "Install srelens CLI" }));
  expect((await screen.findByRole("alert")).textContent).toMatch(/permission denied/i);
  await user.click(screen.getByRole("button", { name: "Install srelens CLI" }));
  expect(core.installSrelensCli).toHaveBeenCalledTimes(2);
});
it("does not offer installation after CLI status could not be read", async () => {
  core.srelensCliStatus.mockRejectedValue(new Error("status unavailable"));
  render(<McpClientSetup url={URL} token={TOKEN} />);
  const install = await screen.findByRole("button", { name: "Install srelens CLI" });
  expect((install as HTMLButtonElement).disabled).toBe(true);
  expect(screen.getByRole("alert").textContent).toMatch(/status unavailable/i);
});
it("generates Windows stdio config with the absolute desktop executable", async () => {
  Object.defineProperty(navigator, "platform", { configurable: true, value: "Win32" });
  const executable = String.raw`C:\Program Files\srelens\srelens.exe`;
  core.srelensCliStatus.mockResolvedValue({ installed: true, path: executable, links_to: null, on_path: false });
  render(<McpClientSetup url={URL} token={TOKEN} />);
  expect(await screen.findAllByText(new RegExp(executable.replace(/[\\]/g, "\\\\")))).not.toHaveLength(0);
  await userEvent.selectOptions(screen.getByLabelText("MCP client"), "cursor");
  expect(JSON.parse(screen.getByTestId("mcp-client-config").textContent!).mcpServers.srelens.command).toBe(executable);
  expect(screen.queryByRole("button", { name: /Install srelens CLI/ })).toBeNull();
});
it("reports failed HTTP prerequisites and retries them instead of claiming the server is stopped", async () => {
  const retryStatus = vi.fn(); const retryToken = vi.fn();
  const user = userEvent.setup();
  render(<McpClientSetup url={null} token={null} statusError={new Error("status denied")} tokenError={new Error("token denied")}
    onRetryStatus={retryStatus} onRetryToken={retryToken} />);
  await user.selectOptions(screen.getByLabelText("Transport"), "http");
  expect(screen.getByText(/status denied/)).toBeTruthy();
  expect(screen.getByText(/token denied/)).toBeTruthy();
  expect(screen.queryByText(/Start the MCP server/)).toBeNull();
  await user.click(screen.getByRole("button", { name: "Retry server status" }));
  await user.click(screen.getByRole("button", { name: "Retry bearer token" }));
  expect(retryStatus).toHaveBeenCalledOnce(); expect(retryToken).toHaveBeenCalledOnce();
});
it("shows a failed prerequisite while the sibling read is still loading", async () => {
  const retryStatus = vi.fn(); const retryToken = vi.fn();
  const user = userEvent.setup();
  const view = render(<McpClientSetup url={null} token={null} statusError={new Error("status denied")} tokenLoading
    onRetryStatus={retryStatus} />);
  await user.selectOptions(screen.getByLabelText("Transport"), "http");
  expect(screen.getByText(/status denied/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Retry server status" })).toBeTruthy();
  expect(screen.getByRole("status").textContent).toMatch(/Checking/);

  view.rerender(<McpClientSetup url={null} token={null} tokenError={new Error("token denied")} statusLoading
    onRetryToken={retryToken} />);
  expect(screen.getByText(/token denied/)).toBeTruthy();
  expect(screen.getByRole("button", { name: "Retry bearer token" })).toBeTruthy();
  expect(screen.getByRole("status").textContent).toMatch(/Checking/);
});
it("reveals the usable HTTP configuration when clipboard copying fails", async () => {
  vi.spyOn(navigator.clipboard, "writeText").mockRejectedValue(new Error("clipboard denied"));
  const user = userEvent.setup(); render(<McpClientSetup url={URL} token={TOKEN} />);
  await user.selectOptions(screen.getByLabelText("Transport"), "http");
  expect(screen.getByTestId("mcp-client-config").textContent).not.toContain(TOKEN);
  await user.click(screen.getByRole("button", { name: "Copy configuration" }));
  expect(screen.getByRole("alert").textContent).toMatch(/copy (?:it )?manually/);
  expect(screen.getByTestId("mcp-client-config").textContent).toContain(TOKEN);
  await user.click(screen.getByRole("button", { name: "Hide configuration token" }));
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.getByTestId("mcp-client-config").textContent).not.toContain(TOKEN);
});
