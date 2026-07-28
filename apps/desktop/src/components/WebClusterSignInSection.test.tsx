import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import React from "react";

const { listClustersMock, clusterLogoutMock, notifySuccessMock, notifyErrorMock, invokeCommandMock } =
  vi.hoisted(() => ({
    listClustersMock: vi.fn(),
    clusterLogoutMock: vi.fn(),
    notifySuccessMock: vi.fn(),
    notifyErrorMock: vi.fn(),
    invokeCommandMock: vi.fn(),
  }));
vi.mock("../lib/webClusters", () => ({
  listClusters: listClustersMock,
  clusterLogout: clusterLogoutMock,
}));
vi.mock("../lib/notify", () => ({
  notify: { success: notifySuccessMock, error: notifyErrorMock },
}));
vi.mock("../transport/transport", () => ({ invokeCommand: invokeCommandMock }));

import { WebClusterSignInSection } from "./WebClusterSignInSection";

const ROWS = [
  {
    key: "prod",
    issuer: "https://issuer.example.com",
    clientId: "abc",
    contexts: ["prod-context"],
    signedIn: false,
    expiresAt: null,
  },
  {
    key: "staging",
    issuer: "https://issuer.example.com",
    clientId: "abc",
    contexts: ["staging-context"],
    signedIn: true,
    expiresAt: 4102444800, // 2100-01-01, far future so the assertion is stable
  },
];

beforeEach(() => {
  listClustersMock.mockReset();
  clusterLogoutMock.mockReset();
  notifySuccessMock.mockReset();
  notifyErrorMock.mockReset();
  listClustersMock.mockResolvedValue(ROWS);
  clusterLogoutMock.mockResolvedValue(undefined);
});

describe("WebClusterSignInSection", () => {
  it("renders a Sign in control for a signed-out cluster and a Sign out control for a signed-in one", async () => {
    render(<WebClusterSignInSection />);
    await waitFor(() => expect(screen.getByText("prod-context")).toBeDefined());
    expect(screen.getByText("staging-context")).toBeDefined();
    expect(screen.getByText(/Not signed in/)).toBeDefined();
    expect(screen.getByText(/Signed in/)).toBeDefined();

    // Each row shows the one relevant action: Sign in when signed out, Sign
    // out when signed in.
    const signInButtons = screen.getAllByRole("button", { name: /sign in/i });
    expect(signInButtons.length).toBe(1);
    const signOutButtons = screen.getAllByRole("button", { name: /sign out/i });
    expect(signOutButtons.length).toBe(1);
    expect((signOutButtons[0] as HTMLButtonElement).disabled).toBe(false);
  });

  it("signs out of a cluster and refreshes the list", async () => {
    const user = userEvent.setup();
    render(<WebClusterSignInSection />);
    await waitFor(() => expect(screen.getByText("staging-context")).toBeDefined());

    const signOutButtons = screen.getAllByRole("button", { name: /sign out/i });
    const enabled = signOutButtons.find((b) => !(b as HTMLButtonElement).disabled)!;
    await user.click(enabled);

    await waitFor(() => expect(clusterLogoutMock).toHaveBeenCalledWith("staging"));
    expect(notifySuccessMock).toHaveBeenCalled();
    await waitFor(() => expect(listClustersMock).toHaveBeenCalledTimes(2));
  });

  it("signs in via the tauri command on desktop (not a web navigation)", async () => {
    (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__ = {};
    invokeCommandMock.mockResolvedValue(undefined);
    try {
      const user = userEvent.setup();
      render(<WebClusterSignInSection />);
      await waitFor(() => expect(screen.getByText("prod-context")).toBeDefined());
      await user.click(screen.getByRole("button", { name: /sign in/i }));
      await waitFor(() =>
        expect(invokeCommandMock).toHaveBeenCalledWith("cluster_login", { key: "prod" }),
      );
    } finally {
      delete (window as unknown as { __TAURI_INTERNALS__?: object }).__TAURI_INTERNALS__;
    }
  });
});
