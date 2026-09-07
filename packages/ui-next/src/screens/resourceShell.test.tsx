import { describe, it, expect, beforeEach } from "vitest";
import { act, render, screen } from "@testing-library/react";

import {
  resetContexts,
  setContexts,
  useContextsError,
  useContextsStatus,
} from "../lib/clusters";
import { NoClusterScreen } from "./resourceShell";

/**
 * `NoClusterScreen` is what `Events`, `Workloads`, `Resources`, `Overview`,
 * `Logs` and `Helm` all fall back to, so a sentence that overstates here
 * overstates on six screens at once — which is exactly how a user came to be
 * told to "pick a cluster in the rail" while the title bar read
 * `k8sm01-admin` and every tab carried its chip.
 *
 * The three cases it has to keep apart are the contexts store's three states,
 * not one boolean: nothing selected (the reader's turn), nothing listed yet
 * (srelens's turn), and a listing that failed (the backend's turn, and the
 * only one where srelens already knows the reason).
 */

const shown = () => <NoClusterScreen title="Helm" noun="Helm releases" />;

describe("NoClusterScreen", () => {
  beforeEach(resetContexts);

  it("asks the reader to pick, but only once the clusters have actually been listed", () => {
    setContexts([]);
    render(shown());
    expect(screen.getByText("No cluster in focus")).toBeTruthy();
    expect(screen.getByText("Pick a cluster in the rail to list its Helm releases.")).toBeTruthy();
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("says it is still listing rather than blaming the reader, before the clusters have loaded", () => {
    render(shown());
    expect(screen.getByRole("status", { name: "Loading clusters" })).toBeTruthy();
    expect(screen.getByText("Loading clusters")).toBeTruthy();
    // The accusation is the whole defect: a cluster can be in focus here and
    // the reader has nothing to fix.
    expect(screen.queryByText("No cluster in focus")).toBeNull();
    expect(screen.queryByText(/Pick a cluster in the rail/)).toBeNull();
  });

  it("reports why the clusters could not be listed instead of asking for a pick", () => {
    setContexts([], "ServiceError: client error (Connect)");
    render(shown());
    expect(screen.getByRole("alert")).toBeTruthy();
    expect(screen.getByText("Clusters could not be listed")).toBeTruthy();
    expect(screen.queryByText("No cluster in focus")).toBeNull();
    expect(screen.queryByText(/Pick a cluster in the rail/)).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("classifies the failure through describeError rather than printing the backend's struct", () => {
    setContexts([], "ServiceError: client error (Connect)");
    render(shown());
    expect(
      screen.getByText(/The connection to the API server could not be made/),
    ).toBeTruthy();
    // Nothing is dropped: the original is still one disclosure away.
    expect(screen.getByText("ServiceError: client error (Connect)")).toBeTruthy();
  });

  it("follows the store from listing, through a refusal, to a real pick — one mount, three states", () => {
    render(shown());
    expect(screen.getByRole("status", { name: "Loading clusters" })).toBeTruthy();

    act(() => setContexts([], "ServiceError: client error (Connect)"));
    expect(screen.getByText("Clusters could not be listed")).toBeTruthy();
    expect(screen.queryByRole("status")).toBeNull();

    act(() => setContexts([]));
    expect(screen.getByText("No cluster in focus")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();

    // The screen's own title bar survives all three.
    expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Helm");
  });

  it("names the caller's own noun in the pick sentence", () => {
    setContexts([]);
    render(<NoClusterScreen title="Events" noun="events" />);
    expect(screen.getByText("Pick a cluster in the rail to list its events.")).toBeTruthy();
  });
});

/**
 * Lives here rather than in `clusters.test.ts` because the failure it guards
 * against only exists in React: `useSyncExternalStore` re-reads the snapshot
 * after every render, and a getter that allocates per call never settles —
 * "Maximum update depth exceeded", which this codebase has already shipped
 * once. A plain `getStatus() === getStatus()` assertion cannot see it, because
 * the two states this adds are primitives right up until someone bundles them
 * into an object.
 */
describe("contexts store snapshots under useSyncExternalStore", () => {
  beforeEach(resetContexts);

  it("settles in one render rather than re-reading a freshly allocated snapshot forever", () => {
    let renders = 0;
    function Probe() {
      renders += 1;
      const status = useContextsStatus();
      const error = useContextsError();
      return <span data-testid="probe">{`${status}|${error}`}</span>;
    }
    render(<Probe />);
    expect(screen.getByTestId("probe").textContent).toBe("loading|");
    expect(renders).toBe(1);
  });
});
