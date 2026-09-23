import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
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

it("preserves readable fields and offers Retry for a failed joined field", async () => {
  const onRetry = vi.fn();
  render(<ResolvedPanelView panel={{
    id:"application", title:"Application", sections:[{type:"fields",fields:[
      {label:"Name",value:"app"},
      {label:"Related",value:null,error:"Related resource read failed"},
    ]}],
  }} onRetry={onRetry}/>);
  expect(screen.getByText("app")).toBeTruthy();
  expect(screen.getByText(/Related resource read failed/)).toBeTruthy();
  await userEvent.click(screen.getByRole("button", {name:/Retry/}));
  expect(onRetry).toHaveBeenCalledOnce();
});
