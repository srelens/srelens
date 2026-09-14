import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { ExtensionLogo, extensionPageIcon } from "./ExtensionLogo";
it("bundles project logos locally and gives other extensions a readable fallback",()=>{
  const view=render(<ExtensionLogo id="org.srelens.flux" name="Flux" />);
  expect(view.container.querySelector("image")?.getAttribute("href")).toContain("/logos/flux.svg");
  view.rerender(<ExtensionLogo id="org.srelens.argocd" name="Argo CD" />);
  expect(view.container.querySelector("image")?.getAttribute("href")).toContain("/logos/argo.svg");
  view.rerender(<ExtensionLogo id="org.community.tool" name="Community Tool" />);
  expect(screen.getByText("CT")).toBeTruthy();
  expect(view.container.querySelector("image")).toBeNull();
});
it("distinguishes common extension page roles",()=>{
  expect(extensionPageIcon("Overview")).not.toBe(extensionPageIcon("Notifications"));
  expect(extensionPageIcon("Helm releases")).not.toBe(extensionPageIcon("Sources"));
  expect(extensionPageIcon("A custom view")).toBeTruthy();
  expect(extensionPageIcon("constructor")).toBe(extensionPageIcon("A custom view"));
});
