import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { ResolvedPanelView } from "./ExtensionPanelSlot";

it("renders a declared cert-manager panel with missing fields and native conditions", () => {
  render(<ResolvedPanelView panel={{
    id: "certificate", title: "Certificate", sections: [
      { type: "fields", fields: [
        { label: "Issuer", value: "letsencrypt" },
        { label: "Renewal time", value: null },
      ] },
      { type: "conditions", items: [{ type: "Ready", status: "True", reason: "Issued" }] },
    ],
  }} onRetry={() => {}} />);
  expect(screen.getByRole("heading", { name: "Certificate" })).toBeTruthy();
  expect(screen.getByRole("heading", { name: "Certificate" }).closest("section")?.classList.contains("section")).toBe(true);
  expect(screen.getByText("letsencrypt")).toBeTruthy();
  expect(screen.getByText("Renewal time").parentElement?.textContent).toContain("—");
  expect(screen.getByText("Issued")).toBeTruthy();
});
