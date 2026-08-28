import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import type { K8sObject } from "@srelens/core";
import { GenericBody } from "./GenericBody";
import { detailFacts } from "./detailData";
import { NodeDetailsBody } from "./NodeBody";

function node(
  spec: Record<string, unknown>,
  status: Record<string, unknown> = {},
  metadata: NonNullable<K8sObject["metadata"]> = { name: "node-a" },
): K8sObject {
  return { kind: "Node", apiVersion: "v1", metadata, spec, status } as K8sObject;
}

describe("NodeDetailsBody", () => {
  describe("Info", () => {
    it("shows the kubelet, OS image, kernel, runtime and architecture", () => {
      render(
        <NodeDetailsBody
          object={node(
            {},
            {
              nodeInfo: {
                kubeletVersion: "v1.29.2",
                osImage: "Ubuntu 22.04.3 LTS",
                kernelVersion: "5.15.0-1053-aws",
                containerRuntimeVersion: "containerd://1.7.11",
                architecture: "amd64",
              },
            },
          )}
        />,
      );
      expect(screen.getByText("v1.29.2")).toBeDefined();
      expect(screen.getByText("Ubuntu 22.04.3 LTS")).toBeDefined();
      expect(screen.getByText("5.15.0-1053-aws")).toBeDefined();
      expect(screen.getByText("containerd://1.7.11")).toBeDefined();
      expect(screen.getByText("amd64")).toBeDefined();
    });

    it("shows Enabled scheduling for a schedulable node", () => {
      render(<NodeDetailsBody object={node({})} />);
      expect(screen.getByText("Enabled")).toBeDefined();
    });

    it("shows Disabled (cordoned) scheduling for an unschedulable node", () => {
      render(<NodeDetailsBody object={node({ unschedulable: true })} />);
      expect(screen.getByText("Disabled (cordoned)")).toBeDefined();
    });
  });

  describe("Capacity", () => {
    it("shows CPU, memory and pod counts as allocatable / capacity", () => {
      render(
        <NodeDetailsBody
          object={node(
            {},
            {
              capacity: { cpu: "8", memory: "32Gi", pods: "110" },
              allocatable: { cpu: "7800m", memory: "30Gi", pods: "110" },
            },
          )}
        />,
      );
      expect(screen.getByText("7800m / 8")).toBeDefined();
      expect(screen.getByText("30Gi / 32Gi")).toBeDefined();
      expect(screen.getByText("110 / 110")).toBeDefined();
    });

    it("shows a node with no capacity reported as blank fractions, not an error", () => {
      render(<NodeDetailsBody object={node({})} />);
      expect(screen.getAllByText("/")).toHaveLength(3);
    });
  });

  it("is a run of flat blocks, not a stack of cards", () => {
    const { container } = render(<NodeDetailsBody object={node({})} />);
    const blocks = [...container.children];
    expect(blocks).toHaveLength(2);
    for (const block of blocks) expect(block.matches("section.section")).toBe(true);
    expect(container.querySelector(".card")).toBeNull();
  });

  describe("composition with GenericBody", () => {
    it("renders the wrapper's facts, Info and Capacity together with no related-pods section", async () => {
      const n = node(
        {},
        { capacity: { cpu: "8" }, allocatable: { cpu: "7800m" } },
        { name: "node-a", creationTimestamp: "2026-08-20T00:00:00Z", labels: { "kubernetes.io/hostname": "node-a" } },
      );
      render(
        <GenericBody kind="Node" object={n} context="ctx">
          <NodeDetailsBody object={n} />
        </GenericBody>,
      );
      // The age is a fact the SCREEN draws, off the one derivation both
      // screens read; this body must not state it a second time.
      expect(detailFacts({ kind: "Node", object: n }).map((f) => f.label)).toContain("Created");
      expect(screen.queryByText("Created")).toBeNull();
      expect(screen.getByRole("heading", { name: "Info" })).toBeDefined();
      expect(screen.getByRole("heading", { name: "Capacity" })).toBeDefined();
      // Node has no `relatedPodSelector` case, so GenericBody fetches nothing.
      expect(screen.queryByRole("heading", { name: "Pods" })).toBeNull();
    });
  });
});
