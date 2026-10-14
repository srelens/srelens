import { beforeEach, describe, expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ClusterContext } from "@srelens/core";
import { resetContexts, setContexts } from "./clusters";
import { defaultState } from "./tabs";
import { getState, setState, setWorkspaceProtected } from "./tabsStore";
import { useClusterGate } from "./clusterMoved";

const PROD: ClusterContext = {
  name: "prod-eu",
  stableId: "prod",
  key: "prod",
  cluster: "prod",
  server: "https://prod",
  isCurrent: true,
  sourceFile: "/home/u/.kube/config",
  authKind: "token",
};
const STAGE: ClusterContext = { ...PROD, name: "stage-eu", stableId: "stage", key: "stage", cluster: "stage", server: "https://stage", isCurrent: false };

beforeEach(() => {
  resetContexts();
  setContexts([PROD, STAGE]);
  // Only prod-eu is in the workspace, so only prod-eu can be protected.
  setState(defaultState([PROD]));
});

const protect = () => setWorkspaceProtected(getState().currentId, true);

function Gate({ pinned, live }: { pinned: string | null; live: string }) {
  const gate = useClusterGate({ pinned, live, verb: "delete" });
  return (
    <div>
      {gate.alert}
      <p data-testid="refusal">{gate.refusal ?? "none"}</p>
      <p data-testid="ready">{String(gate.ready)}</p>
      <button type="button" onClick={gate.reset}>
        Reset
      </button>
    </div>
  );
}

const refusal = () => screen.getByTestId("refusal").textContent;
const ready = () => screen.getByTestId("ready").textContent;
const field = () => screen.getByRole("textbox", { name: "Type prod-eu to confirm" }) as HTMLInputElement;
const tick = () => screen.getByRole("checkbox", { name: "Yes, still delete on prod-eu." }) as HTMLInputElement;
const MOVED = "This runs on prod-eu, not stage-eu. Confirm the cluster above, or cancel.";

describe("useClusterGate", () => {
  it("asks nothing of an unprotected cluster the rail has not left, as before", () => {
    render(<Gate pinned="stage-eu" live="stage-eu" />);
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.queryByRole("checkbox")).toBeNull();
    expect(refusal()).toBe("none");
    expect(ready()).toBe("true");
  });

  it("keeps the moved-rail question as it was, and is ready once it is answered", async () => {
    render(<Gate pinned="prod-eu" live="stage-eu" />);
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(refusal()).toBe(MOVED);
    expect(ready()).toBe("false");

    await userEvent.click(tick());

    expect(refusal()).toBe("none");
    expect(ready()).toBe("true");
  });

  it("on a protected cluster, adds the typed name and refuses until it matches", async () => {
    protect();
    render(<Gate pinned="prod-eu" live="prod-eu" />);
    expect(screen.getByText("prod-eu is protected (workspace Default).")).toBeTruthy();
    expect(refusal()).toBe("Type prod-eu to confirm.");
    expect(ready()).toBe("false");

    await userEvent.type(field(), "prod-eu");

    expect(refusal()).toBe("none");
    expect(ready()).toBe("true");
  });

  it("asks which cluster first, then for its name, and draws them in that order", async () => {
    protect();
    render(<Gate pinned="prod-eu" live="stage-eu" />);
    // The moved banner's tick comes before the typed field in the document.
    expect(tick().compareDocumentPosition(field()) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(refusal()).toBe(MOVED);

    await userEvent.click(tick());
    expect(refusal()).toBe("Type prod-eu to confirm.");

    await userEvent.type(field(), "prod-eu");
    expect(refusal()).toBe("none");
    expect(ready()).toBe("true");
  });

  it("asks of the cluster the action runs on, not the one in focus", () => {
    protect();
    // Opened on stage-eu, which no protected workspace holds; the rail is on
    // prod-eu now. The write goes to stage-eu, so only the moved question stands.
    render(<Gate pinned="stage-eu" live="prod-eu" />);
    expect(screen.queryByRole("textbox")).toBeNull();
    expect(refusal()).toBe("This runs on stage-eu, not prod-eu. Confirm the cluster above, or cancel.");
  });

  it("forgets both the tick and the typed name on reset", async () => {
    protect();
    render(<Gate pinned="prod-eu" live="stage-eu" />);
    await userEvent.click(tick());
    await userEvent.type(field(), "prod-eu");
    expect(ready()).toBe("true");

    await userEvent.click(screen.getByRole("button", { name: "Reset" }));

    expect(tick().checked).toBe(false);
    expect(field().value).toBe("");
    expect(refusal()).toBe(MOVED);
  });

  it("hands back the same reset on every render", async () => {
    // `Overview` builds its node columns off `reset` (Overview.tsx:744-756);
    // a new function per render rebuilt them on every keystroke.
    protect();
    const seen = new Set<() => void>();
    function Probe() {
      const gate = useClusterGate({ pinned: "prod-eu", live: "prod-eu", verb: "delete" });
      seen.add(gate.reset);
      return <>{gate.alert}</>;
    }
    render(<Probe />);
    await userEvent.type(field(), "pr");
    expect(seen.size).toBe(1);
  });
});
