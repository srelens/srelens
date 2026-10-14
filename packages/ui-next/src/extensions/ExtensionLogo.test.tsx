import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { ExtensionLogo, extensionPageIcon } from "./ExtensionLogo";
const svg = `data:image/svg+xml;base64,${btoa("<svg xmlns=\"http://www.w3.org/2000/svg\"/>")}`;
it("draws the logo the host read from the app's package, and initials otherwise (#562)",()=>{
  const view=render(<ExtensionLogo icon={svg} name="Packaged example" />);
  expect(view.container.querySelector("image")?.getAttribute("href")).toBe(svg);
  expect(view.container.querySelector("[data-extension-logo]")?.getAttribute("data-extension-logo")).toBe("package");
  view.rerender(<ExtensionLogo icon="data:image/png;base64,iVBORw0KGgo=" name="Packaged example" />);
  expect(view.container.querySelector("image")?.getAttribute("href")).toBe("data:image/png;base64,iVBORw0KGgo=");
  // No package logo: initials, whatever the app's name or ID. Official apps get no bundled one.
  view.rerender(<ExtensionLogo name="Flux" />);
  expect(screen.getByText("F")).toBeTruthy();
  expect(view.container.querySelector("image")).toBeNull();
  view.rerender(<ExtensionLogo name="Community Tool" />);
  expect(screen.getByText("CT")).toBeTruthy();
  expect(view.container.querySelector("[data-extension-logo]")?.getAttribute("data-extension-logo")).toBe("initials");
});
it("never draws a logo from anywhere but an inline package image",()=>{
  for (const icon of [
    "https://example.com/logo.svg",
    "/logos/flux.svg",
    "data:text/html;base64,PHNjcmlwdD4=",
    "data:image/svg+xml,<svg onload=alert(1)>",
    `${svg}" onload="alert(1)`,
    "javascript:alert(1)",
  ]) {
    const view=render(<ExtensionLogo icon={icon} name="Untrusted" />);
    expect(view.container.querySelector("image"), icon).toBeNull();
    expect(view.getByText("U")).toBeTruthy();
    view.unmount();
  }
});
it("distinguishes common app page roles",()=>{
  expect(extensionPageIcon("Overview")).not.toBe(extensionPageIcon("Notifications"));
  expect(extensionPageIcon("Helm releases")).not.toBe(extensionPageIcon("Sources"));
  expect(extensionPageIcon("A custom view")).toBeTruthy();
  expect(extensionPageIcon("constructor")).toBe(extensionPageIcon("A custom view"));
});
