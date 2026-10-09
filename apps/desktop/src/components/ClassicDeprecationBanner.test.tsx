import { describe, it, expect, vi, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

const { switchDesignMock } = vi.hoisted(() => ({ switchDesignMock: vi.fn() }));
vi.mock("../design", async (importOriginal) => ({
  ...(await importOriginal<typeof import("../design")>()),
  switchDesign: switchDesignMock,
}));

import { ClassicDeprecationBanner } from "./ClassicDeprecationBanner";

beforeEach(() => {
  switchDesignMock.mockReset().mockResolvedValue({ ok: true });
  sessionStorage.clear();
});

describe("ClassicDeprecationBanner", () => {
  it("says the classic design is deprecated and will be removed", () => {
    render(<ClassicDeprecationBanner />);
    expect(screen.getByRole("status").textContent).toMatch(
      /classic design is deprecated and will be removed in a future version/i,
    );
  });

  it("switches to the new design through the existing switch", async () => {
    render(<ClassicDeprecationBanner />);
    await userEvent.click(screen.getByRole("button", { name: "Switch to the new design" }));
    expect(switchDesignMock).toHaveBeenCalledWith("next");
  });

  it("says why when the switch is refused, and can be retried", async () => {
    // A successful switch reloads, so only a refusal comes back here. Without
    // the reason the button would look inert.
    switchDesignMock.mockResolvedValue({ ok: false, reason: "storage refused it" });
    render(<ClassicDeprecationBanner />);
    const button = screen.getByRole("button", { name: "Switch to the new design" });
    await userEvent.click(button);
    expect(screen.getByRole("alert").textContent).toContain("storage refused it");
    expect(button.hasAttribute("disabled")).toBe(false);
  });

  it("goes away when dismissed, and stays away for the rest of the session", async () => {
    const first = render(<ClassicDeprecationBanner />);
    await userEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    expect(screen.queryByRole("status")).toBeNull();
    first.unmount();

    // A remount in the same session — a reload of the window — keeps it gone.
    render(<ClassicDeprecationBanner />);
    expect(screen.queryByRole("status")).toBeNull();
  });
});
