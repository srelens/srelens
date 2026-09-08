import { act, render, screen } from "@testing-library/react";
import { beforeEach, expect, it } from "vitest";
import { Sidebar } from "@srelens/ui-kit";
import { loadWorkspaceLayout } from "@srelens/core";
import { saveNavigationWidth, useNavigationWidth } from "./navigationWidth";
function Navigation() { const width = useNavigationWidth(); return <Sidebar label="Navigation" width={width} />; }
beforeEach(() => { localStorage.clear(); });
it("updates an open navigation sidebar and persists its chosen width", () => {
  render(<Navigation />);
  act(() => saveNavigationWidth(320));
  expect(screen.getByRole("navigation").style.width).toBe("320px");
  expect(loadWorkspaceLayout().leftSidebarWidth).toBe(320);
});
