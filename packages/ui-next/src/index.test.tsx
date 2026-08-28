import { describe, it, expect, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { NextApp } from "./index";

describe("NextApp", () => {
  it("says what it is, so nobody thinks the app is broken", () => {
    // The whole new design is one placeholder at this point. Someone who opts
    // in has to be told that, not left guessing.
    render(<NextApp onExit={() => null} />);
    expect(screen.getByRole("heading", { name: /new design/i })).toBeDefined();
    expect(screen.getByText(/not.*(built|there)/i)).toBeDefined();
  });

  it("offers a way back without going through Settings", () => {
    // Settings does not exist here yet, so this button is the only exit.
    render(<NextApp onExit={() => null} />);
    expect(screen.getByRole("button", { name: /classic design/i })).toBeDefined();
  });

  it("calls back when asked to leave", async () => {
    const onExit = vi.fn().mockReturnValue(null);
    render(<NextApp onExit={onExit} />);
    await userEvent.click(screen.getByRole("button", { name: /classic design/i }));
    expect(onExit).toHaveBeenCalledTimes(1);
  });

  it("shows why it could not leave, since there is no toast host here", () => {
    // The Toaster lives in the classic tree, so a failure on the way out would
    // be invisible and the button would look inert. (#314 review)
    render(<NextApp onExit={() => "storage refused the preference"} />);
    return userEvent
      .click(screen.getByRole("button", { name: /classic design/i }))
      .then(() => {
        expect(screen.getByRole("alert").textContent).toContain("storage refused");
      });
  });

  it("follows the hash after mount, not only on a fresh load", async () => {
    // Reading window.location.hash during render subscribes to nothing, so
    // navigating to #gallery left the placeholder up and navigating away left
    // the gallery up, until a reload. (#317 review)
    render(<NextApp onExit={() => null} />);
    expect(screen.getByRole("heading", { name: /new design/i })).toBeDefined();

    window.location.hash = "#gallery";
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    expect(await screen.findByRole("heading", { name: /design system/i })).toBeDefined();

    window.location.hash = "";
    window.dispatchEvent(new HashChangeEvent("hashchange"));
    expect(await screen.findByRole("heading", { name: /new design/i })).toBeDefined();
  });
});
