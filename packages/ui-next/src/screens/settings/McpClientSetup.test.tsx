import { beforeEach, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { McpClientSetup } from "./McpClientSetup";
const core = vi.hoisted(() => ({ installSrelensCli: vi.fn(), srelensCliStatus: vi.fn() }));
vi.mock("@srelens/core", async original => ({ ...await original<object>(), ...core }));
const URL = "http://127.0.0.1:9411/mcp";
const TOKEN = "a".repeat(64);
beforeEach(() => {
  vi.clearAllMocks();
  core.srelensCliStatus.mockResolvedValue({ installed: false, path: "/home/user/.local/bin/srelens", links_to: null, on_path: false });
  core.installSrelensCli.mockResolvedValue("/home/user/.local/bin/srelens");
});
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
it("requires a known URL and token for HTTP config, while stdio stays available", async () => {
  const user = userEvent.setup(); render(<McpClientSetup url={null} token={null} />);
  await user.selectOptions(screen.getByLabelText("Transport"), "http");
  expect((screen.getByRole("button", { name: "Copy configuration" }) as HTMLButtonElement).disabled).toBe(true);
  await user.selectOptions(screen.getByLabelText("Transport"), "stdio");
  expect(screen.getByTestId("mcp-client-config").textContent).toContain("--mcp-stdio");
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
