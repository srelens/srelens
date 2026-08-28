import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { DiffView } from "./DiffView";
import type { DiffDoc } from "@srelens/core";

function doc(rows: DiffDoc["rows"], over: Partial<DiffDoc> = {}): DiffDoc {
  return { kind: "ConfigMap", name: "a", namespace: "d", exists: true, changed: true, rows, currentResourceVersion: "1", ...over };
}

describe("DiffView", () => {
  it("renders changed lines on both sides", () => {
    render(
      <DiffView doc={doc([
        { tag: "same", left: "spec:", right: "spec:" },
        { tag: "replace", left: "  replicas: 3", right: "  replicas: 5" },
        { tag: "insert", left: null, right: "  paused: true" },
        { tag: "delete", left: "  old: 1", right: null },
      ])} />,
    );
    expect(screen.getByText("replicas: 3", { exact: false })).toBeDefined();
    expect(screen.getByText("replicas: 5", { exact: false })).toBeDefined();
    expect(screen.getByText("paused: true", { exact: false })).toBeDefined();
    expect(screen.getByText("old: 1", { exact: false })).toBeDefined();
  });

  it("shows a no-changes state", () => {
    render(<DiffView doc={doc([{ tag: "same", left: "a", right: "a" }], { changed: false })} />);
    expect(screen.getByText(/no changes/i)).toBeDefined();
  });

  it("labels a create", () => {
    render(<DiffView doc={doc([{ tag: "insert", left: null, right: "a" }], { exists: false })} />);
    expect(screen.getByText(/new resource/i)).toBeDefined();
  });
});
