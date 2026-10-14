import { beforeEach, describe, expect, it } from "vitest";
import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext } from "@srelens/core";
import { resetContexts, setContexts } from "./clusters";
import { defaultState } from "./tabs";
import { getState, setState, setWorkspaceProtected } from "./tabsStore";
import { useProtectedConfirm } from "./protectedConfirm";

const PROD: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  key: "prod",
  pinnedId: "srelens-context:/home/u/.kube/config#prod-eu",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/u/.kube/config",
  authKind: "token",
};

beforeEach(() => {
  resetContexts();
  setContexts([PROD]);
  setState(defaultState([PROD]));
});

const protect = () => setWorkspaceProtected(getState().currentId, true);

/** The hook's whole answer, drawn: its field, and the three facts beside it. */
function Typed({ context }: { context: string | null }) {
  const typed = useProtectedConfirm(context);
  return (
    <div>
      {typed.field}
      <p data-testid="refusal">{typed.refusal ?? "none"}</p>
      <p data-testid="ready">{String(typed.ready)}</p>
      <p data-testid="protection">
        {typed.protection ? `${typed.protection.name} · ${typed.protection.workspace}` : "none"}
      </p>
      <button type="button" onClick={typed.reset}>
        Reset
      </button>
    </div>
  );
}

const refusal = () => screen.getByTestId("refusal").textContent;
const ready = () => screen.getByTestId("ready").textContent;
const field = () => screen.getByRole("textbox", { name: "Type prod-eu to confirm" }) as HTMLInputElement;

describe("useProtectedConfirm", () => {
  it("asks nothing of a cluster no protected workspace holds, nor of no cluster at all", () => {
    const view = render(<Typed context="prod-eu" />);
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(refusal()).toBe("none");
    expect(ready()).toBe("true");
    expect(screen.getByTestId("protection").textContent).toBe("none");

    // Protecting prod-eu says nothing about a request that names no cluster,
    // or one this window does not know.
    act(() => protect());
    for (const context of [null, "", "nowhere"]) {
      view.rerender(<Typed context={context} />);
      expect(screen.queryByRole("textbox")).toBeNull();
      expect(ready()).toBe("true");
    }
  });

  it("draws the line and the field once the cluster is protected, without being opened again", () => {
    render(<Typed context="prod-eu" />);
    expect(screen.queryByRole("textbox")).toBeNull();

    act(() => protect());

    expect(screen.getByText("prod-eu is protected (workspace Default).")).toBeTruthy();
    expect(field().value).toBe("");
    expect(refusal()).toBe("Type prod-eu to confirm.");
    expect(ready()).toBe("false");
    expect(screen.getByTestId("protection").textContent).toBe("prod-eu · Default");
  });

  it("describes the field with its line, so a screen reader hears why it is asked", () => {
    protect();
    render(<Typed context="prod-eu" />);
    const describedBy = field().getAttribute("aria-describedby") ?? "";
    expect(document.getElementById(describedBy)?.textContent).toBe("prod-eu is protected (workspace Default).");
  });

  it("refuses until the trimmed text is the name exactly, case included", async () => {
    protect();
    render(<Typed context="prod-eu" />);

    await userEvent.type(field(), "prod-e");
    expect(refusal()).toBe("Type prod-eu to confirm.");

    await userEvent.clear(field());
    await userEvent.type(field(), "PROD-EU");
    expect(refusal()).toBe("Type prod-eu to confirm.");

    await userEvent.clear(field());
    await userEvent.type(field(), "prod-eu-x");
    expect(refusal()).toBe("Type prod-eu to confirm.");

    // A paste's stray spaces are forgiven; nothing else is.
    await userEvent.clear(field());
    await userEvent.type(field(), "  prod-eu ");
    expect(refusal()).toBe("none");
    expect(ready()).toBe("true");
  });

  it("forgets what was typed on reset", async () => {
    protect();
    render(<Typed context="prod-eu" />);
    await userEvent.type(field(), "prod-eu");
    expect(ready()).toBe("true");

    await userEvent.click(screen.getByRole("button", { name: "Reset" }));

    expect(field().value).toBe("");
    expect(refusal()).toBe("Type prod-eu to confirm.");
    expect(ready()).toBe("false");
  });

  it("reads a cluster named by its pinned ID, the way an app surface names it", () => {
    protect();
    render(<Typed context={PROD.pinnedId!} />);
    expect(field()).toBeTruthy();
    expect(screen.getByTestId("protection").textContent).toBe("prod-eu · Default");
  });
});
