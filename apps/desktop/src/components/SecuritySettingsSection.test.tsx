import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";

vi.mock("../lib/notify", () => ({ notify: { success: vi.fn(), error: vi.fn(), info: vi.fn() } }));

const { mcpSecurity } = vi.hoisted(() => ({
  mcpSecurity: {
    getMcpTokenStorage: vi.fn(),
    vaultBiometricStatus: vi.fn(),
    vaultBiometricEnable: vi.fn(),
    vaultBiometricDisable: vi.fn(),
    vaultBiometricUnlock: vi.fn(),
    vaultChangePassword: vi.fn(),
  },
}));
vi.mock("../lib/mcpSecurity", () => mcpSecurity);

import { SecuritySettingsSection } from "./SecuritySettingsSection";

beforeEach(() => {
  Object.values(mcpSecurity).forEach((m) => m.mockReset());
  mcpSecurity.getMcpTokenStorage.mockResolvedValue("password");
  mcpSecurity.vaultBiometricStatus.mockResolvedValue({ available: false, enabled: false, unlocked: true });
  mcpSecurity.vaultBiometricEnable.mockResolvedValue(undefined);
  mcpSecurity.vaultBiometricDisable.mockResolvedValue(undefined);
  mcpSecurity.vaultBiometricUnlock.mockResolvedValue(undefined);
  mcpSecurity.vaultChangePassword.mockResolvedValue(undefined);
});

describe("SecuritySettingsSection", () => {
  it("warns when the vault master key fell back to the plain file", async () => {
    mcpSecurity.getMcpTokenStorage.mockResolvedValue("file");
    render(<SecuritySettingsSection />);
    expect(await screen.findByText(/key that encrypts srelens's secrets is stored/)).toBeDefined();
  });

  it("explains a locked vault (keychain unreachable) without implying data loss", async () => {
    mcpSecurity.getMcpTokenStorage.mockResolvedValue("locked");
    render(<SecuritySettingsSection />);
    expect(await screen.findByText(/couldn't load the key that encrypts its secrets/)).toBeDefined();
    expect(screen.getByText(/they are untouched/)).toBeDefined();
  });

  it("offers the biometric toggle only when a sensor exists, and enabling calls through", async () => {
    mcpSecurity.vaultBiometricStatus.mockResolvedValue({ available: true, enabled: false, unlocked: true });
    render(<SecuritySettingsSection />);
    const toggle = (await screen.findByRole("checkbox", {
      name: /unlock with touch id instead of the master password/i,
    })) as HTMLInputElement;
    expect(toggle.checked).toBe(false);

    mcpSecurity.vaultBiometricStatus.mockResolvedValue({ available: true, enabled: true, unlocked: true });
    fireEvent.click(toggle);
    await waitFor(() => expect(mcpSecurity.vaultBiometricEnable).toHaveBeenCalled());
  });

  it("shows the unlock control while biometric-locked and unlocks on click", async () => {
    mcpSecurity.getMcpTokenStorage.mockResolvedValue("biometric-locked");
    mcpSecurity.vaultBiometricStatus.mockResolvedValue({ available: true, enabled: true, unlocked: false });
    render(<SecuritySettingsSection />);
    expect(await screen.findByText(/locked behind touch id for this session/i)).toBeDefined();

    mcpSecurity.getMcpTokenStorage.mockResolvedValue("biometric");
    fireEvent.click(screen.getByRole("button", { name: /unlock with touch id/i }));
    await waitFor(() => expect(mcpSecurity.vaultBiometricUnlock).toHaveBeenCalled());
  });

  it("changes the master password once current and matching new values are entered", async () => {
    render(<SecuritySettingsSection />);
    await screen.findByText(/change master password/i);
    fireEvent.change(screen.getByPlaceholderText(/current password/i), { target: { value: "old-password" } });
    fireEvent.change(screen.getByPlaceholderText(/new password \(at least 8/i), {
      target: { value: "new-password-1" },
    });
    fireEvent.change(screen.getByPlaceholderText(/confirm new password/i), {
      target: { value: "new-password-1" },
    });
    fireEvent.click(screen.getByRole("button", { name: /change password/i }));
    await waitFor(() =>
      expect(mcpSecurity.vaultChangePassword).toHaveBeenCalledWith("old-password", "new-password-1"),
    );
  });
});
