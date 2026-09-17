import { describe, it, expect, vi, beforeEach } from "vitest";
import { act, render, screen, fireEvent, waitFor } from "@testing-library/react";

const { mcpSecurity, listeners } = vi.hoisted(() => ({
  mcpSecurity: {
    vaultStatus: vi.fn(),
    vaultSetupPassword: vi.fn(),
    vaultUnlockPassword: vi.fn(),
    vaultRecoverPassword: vi.fn(),
    vaultBiometricUnlock: vi.fn(),
  },
  listeners: {} as Record<string, (payload?: unknown) => void>,
}));
vi.mock("@srelens/core/lib/mcpSecurity", () => mcpSecurity);
vi.mock("@srelens/core", async (orig) => {
  const actual = await orig<typeof import("@srelens/core")>();
  return {
    ...actual,
    on: (channel: string, handler: (payload?: unknown) => void) => {
      listeners[channel] = handler;
      return () => {
        delete listeners[channel];
      };
    },
  };
});

import { VaultGate } from "./VaultGate";

// The gate only renders inside a Tauri window.
beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  Object.values(mcpSecurity).forEach((m) => m.mockReset());
  for (const key of Object.keys(listeners)) delete listeners[key];
  mcpSecurity.vaultSetupPassword.mockResolvedValue(undefined);
  mcpSecurity.vaultUnlockPassword.mockResolvedValue(undefined);
  mcpSecurity.vaultBiometricUnlock.mockResolvedValue(undefined);
});

const status = (over: Partial<Parameters<typeof mcpSecurity.vaultStatus>[0]> & Record<string, unknown>) => ({
  mode: "locked",
  keySource: "password-locked",
  biometricAvailable: false,
  biometricEnrolled: false,
  ...over,
});

describe("VaultGate", () => {
  it("stays closed with a retry when the status command itself fails", async () => {
    mcpSecurity.vaultStatus.mockRejectedValue(new Error("vault state unavailable"));
    render(<VaultGate onReady={() => { throw new Error("onReady must not fire on failure"); }} />);
    expect(await screen.findByText(/secrets unavailable/i)).toBeTruthy();

    // Retry succeeds → the gate proceeds normally (locked form).
    mcpSecurity.vaultStatus.mockResolvedValue(status({}));
    fireEvent.click(screen.getByRole("button", { name: /retry/i }));
    expect(await screen.findByRole("heading", { name: /unlock srelens/i })).toBeTruthy();
  });

  it("renders nothing once the vault is unlocked, and broadcasts the unlock", async () => {
    mcpSecurity.vaultStatus.mockResolvedValue(status({ mode: "unlocked", keySource: "password" }));
    const unlocked = vi.fn();
    window.addEventListener("srelens:vault-unlocked", unlocked);
    const { container } = render(<VaultGate />);
    await waitFor(() => expect(mcpSecurity.vaultStatus).toHaveBeenCalled());
    expect(container.textContent).toBe("");
    // Vault-dependent views mounted under the gate rely on this broadcast.
    await waitFor(() => expect(unlocked).toHaveBeenCalledTimes(1));
    window.removeEventListener("srelens:vault-unlocked", unlocked);
  });

  it("first launch shows the mandatory setup and creates the password", async () => {
    mcpSecurity.vaultStatus.mockResolvedValue(status({ mode: "setup-required", keySource: "keychain" }));
    render(<VaultGate />);
    await screen.findByText(/protect your secrets/i);

    fireEvent.change(screen.getByPlaceholderText(/at least 8 characters/i), {
      target: { value: "hunter22hunter22" },
    });
    // Mismatched confirm is caught locally, before any backend call.
    fireEvent.click(screen.getByRole("button", { name: /create password/i }));
    expect(await screen.findByText(/don't match/i)).toBeTruthy();
    expect(mcpSecurity.vaultSetupPassword).not.toHaveBeenCalled();

    const inputs = screen.getAllByDisplayValue("", { exact: true });
    fireEvent.change(inputs[0], { target: { value: "hunter22hunter22" } });
    mcpSecurity.vaultStatus.mockResolvedValue(status({ mode: "unlocked", keySource: "password" }));
    fireEvent.click(screen.getByRole("button", { name: /create password/i }));
    await waitFor(() =>
      expect(mcpSecurity.vaultSetupPassword).toHaveBeenCalledWith("hunter22hunter22", true),
    );
  });

  it("locked launches unlock with the password", async () => {
    mcpSecurity.vaultStatus.mockResolvedValue(status({}));
    render(<VaultGate />);
    await screen.findByRole("heading", { name: /unlock srelens/i });
    fireEvent.change(screen.getByPlaceholderText(/master password/i), {
      target: { value: "hunter22hunter22" },
    });
    mcpSecurity.vaultStatus.mockResolvedValue(status({ mode: "unlocked", keySource: "password" }));
    fireEvent.click(screen.getByRole("button", { name: /^unlock$/i }));
    await waitFor(() =>
      expect(mcpSecurity.vaultUnlockPassword).toHaveBeenCalledWith("hunter22hunter22"),
    );
  });

  it("auto-raises the biometric prompt once when the skip is enrolled", async () => {
    mcpSecurity.vaultStatus.mockResolvedValue(
      status({ biometricAvailable: true, biometricEnrolled: true, keySource: "biometric-locked" }),
    );
    render(<VaultGate />);
    await waitFor(() => expect(mcpSecurity.vaultBiometricUnlock).toHaveBeenCalledTimes(1));
    // The manual button is there too, as the retry affordance.
    expect(await screen.findByRole("button", { name: /unlock with touch id/i })).toBeTruthy();
  });

  it("forgot password shows the recovered password once, then dismisses", async () => {
    mcpSecurity.vaultStatus.mockResolvedValue(status({}));
    mcpSecurity.vaultRecoverPassword.mockResolvedValue("recovered-pass-123");
    render(<VaultGate />);
    await screen.findByRole("heading", { name: /unlock srelens/i });

    fireEvent.click(screen.getByRole("button", { name: /forgot password/i }));
    expect(await screen.findByText("recovered-pass-123")).toBeTruthy();

    mcpSecurity.vaultStatus.mockResolvedValue(status({ mode: "unlocked", keySource: "password" }));
    fireEvent.click(screen.getByRole("button", { name: /continue/i }));
    await waitFor(() => expect(screen.queryByText("recovered-pass-123")).toBeFalsy());
  });

  it("covers classic window immediately when vault-locked fires, before refresh resolves", async () => {
    mcpSecurity.vaultStatus.mockResolvedValue(status({ mode: "unlocked", keySource: "password" }));
    const onLocked = vi.fn();
    const { container } = render(<VaultGate onLocked={onLocked} />);
    await waitFor(() => expect(mcpSecurity.vaultStatus).toHaveBeenCalledTimes(1));
    expect(container.textContent).toBe("");

    // Simulate an in-flight status call that has not yet resolved
    let resolveStatus: () => void = () => {};
    mcpSecurity.vaultStatus.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolveStatus = () => resolve(status({ mode: "locked" }));
        }),
    );

    // Trigger vault-locked event
    act(() => {
      listeners["vault-locked"]?.();
    });

    // Synchronously covered with the unlock screen
    expect(screen.getByRole("heading", { name: /unlock srelens/i })).toBeTruthy();
    expect(onLocked).toHaveBeenCalledTimes(1);

    // After refresh completes, remains covered
    act(() => {
      resolveStatus();
    });
    expect(screen.getByRole("heading", { name: /unlock srelens/i })).toBeTruthy();
  });

  it("lowers the gate when vault-unlocked fires from another window", async () => {
    mcpSecurity.vaultStatus.mockResolvedValue(status({ mode: "locked" }));
    const onReady = vi.fn();
    render(<VaultGate onReady={onReady} />);
    expect(await screen.findByRole("heading", { name: /unlock srelens/i })).toBeTruthy();

    mcpSecurity.vaultStatus.mockResolvedValue(status({ mode: "unlocked", keySource: "password" }));
    await act(async () => {
      listeners["vault-unlocked"]?.();
    });
    await waitFor(() => expect(onReady).toHaveBeenCalledTimes(1));
    expect(screen.queryByRole("heading", { name: /unlock srelens/i })).toBeNull();
  });

  it("ignores a stale unlock status that resolves after vault-locked", async () => {
    mcpSecurity.vaultStatus.mockResolvedValue(status({ mode: "locked" }));
    const onReady = vi.fn();
    render(<VaultGate onReady={onReady} />);
    expect(await screen.findByRole("heading", { name: /unlock srelens/i })).toBeTruthy();

    const resolvers: Array<(s: ReturnType<typeof status>) => void> = [];
    mcpSecurity.vaultStatus.mockImplementation(
      () =>
        new Promise((resolve) => {
          resolvers.push(resolve);
        }),
    );
    act(() => {
      listeners["vault-unlocked"]?.();
    });
    act(() => {
      listeners["vault-locked"]?.();
    });
    expect(resolvers).toHaveLength(2);
    await act(async () => {
      resolvers[0](status({ mode: "unlocked", keySource: "password" }));
    });
    await act(async () => {
      resolvers[1](status({ mode: "locked" }));
    });
    expect(screen.getByRole("heading", { name: /unlock srelens/i })).toBeTruthy();
    expect(onReady).not.toHaveBeenCalled();
  });

  it("keeps one vault-locked subscription across onLocked identity changes", async () => {
    mcpSecurity.vaultStatus.mockResolvedValue(status({ mode: "unlocked", keySource: "password" }));
    const first = vi.fn();
    const { rerender } = render(<VaultGate onLocked={first} />);
    await waitFor(() => expect(mcpSecurity.vaultStatus).toHaveBeenCalled());
    expect(listeners["vault-locked"]).toBeDefined();

    // App passes an inline `() => setVaultReady(false)` every render. A
    // dependency on that identity would unlisten and re-listen here, leaving a
    // gap where a broadcast is dropped.
    const second = vi.fn();
    rerender(<VaultGate onLocked={second} />);
    expect(listeners["vault-locked"]).toBeDefined();

    act(() => {
      listeners["vault-locked"]?.();
    });
    expect(first).not.toHaveBeenCalled();
    expect(second).toHaveBeenCalledTimes(1);
  });
});
