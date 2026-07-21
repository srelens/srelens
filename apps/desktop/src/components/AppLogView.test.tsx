import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";

const { readAppLogMock, appLogPathMock, revealAppLogMock } = vi.hoisted(() => ({
  readAppLogMock: vi.fn(),
  appLogPathMock: vi.fn(),
  revealAppLogMock: vi.fn(),
}));
vi.mock("../lib/appLog", () => ({
  readAppLog: readAppLogMock,
  appLogPath: appLogPathMock,
  revealAppLog: revealAppLogMock,
}));

import { AppLogView, logLineLevel } from "./AppLogView";

const SAMPLE = [
  "[2026-07-21][12:00:00][srelens][INFO] srelens 0.3.0 starting",
  "[2026-07-21][12:00:01][srelens][WARN] capability 'k8s.listPods' failed: timeout",
  "[2026-07-21][12:00:02][srelens][ERROR] connect kube_prod/default: unreachable",
].join("\n");

beforeEach(() => {
  readAppLogMock.mockReset();
  appLogPathMock.mockReset();
  revealAppLogMock.mockReset();
  readAppLogMock.mockResolvedValue(SAMPLE);
  appLogPathMock.mockResolvedValue("/Users/x/Library/Logs/srelens/srelens.log");
  revealAppLogMock.mockResolvedValue(undefined);
});

describe("logLineLevel", () => {
  it("extracts the level from a plugin-formatted line, defaulting to INFO", () => {
    expect(logLineLevel("[d][t][srelens][ERROR] boom")).toBe("ERROR");
    expect(logLineLevel("[d][t][srelens][WARN] hmm")).toBe("WARN");
    expect(logLineLevel("a line with no level")).toBe("INFO");
  });
});

describe("AppLogView", () => {
  it("loads the log and shows its path", async () => {
    render(<AppLogView />);
    await waitFor(() => expect(screen.getByText(/srelens 0.3.0 starting/)).toBeDefined());
    expect(screen.getByText("/Users/x/Library/Logs/srelens/srelens.log")).toBeDefined();
  });

  it("filters by level", async () => {
    render(<AppLogView />);
    await waitFor(() => expect(screen.getByText(/starting/)).toBeDefined());

    await userEvent.click(screen.getByRole("combobox", { name: "Log level" }));
    await userEvent.click(await screen.findByRole("option", { name: "ERROR" }));

    expect(screen.getByText(/unreachable/)).toBeDefined();
    expect(screen.queryByText(/starting/)).toBeNull();
  });

  it("filters by search text", async () => {
    render(<AppLogView />);
    await waitFor(() => expect(screen.getByText(/starting/)).toBeDefined());

    fireEvent.change(screen.getByLabelText("Search log"), { target: { value: "timeout" } });
    expect(screen.getByText(/k8s.listPods/)).toBeDefined();
    expect(screen.queryByText(/starting/)).toBeNull();
  });

  it("reveals the log file in the file manager", async () => {
    render(<AppLogView />);
    await waitFor(() => expect(screen.getByText(/starting/)).toBeDefined());
    fireEvent.click(screen.getByRole("button", { name: "Reveal" }));
    await waitFor(() => expect(revealAppLogMock).toHaveBeenCalled());
  });
});
