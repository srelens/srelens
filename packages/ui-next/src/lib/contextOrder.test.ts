import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it } from "vitest";
import { loadContextOrder, saveContextOrder } from "@srelens/core";
import { moveContext, useOrderedContexts } from "./contextOrder";
beforeEach(() => localStorage.clear());
it("uses classic's saved order and persists a move while keeping unlisted contexts", () => {
  saveContextOrder(["staging", "prod", "offline"]);
  const contexts = [{ name: "prod", stableId: "prod" }, { name: "staging", stableId: "staging" }, { name: "dev", stableId: "dev" }];
  const { result } = renderHook(() => useOrderedContexts(contexts));
  expect(result.current.map(c => c.name)).toEqual(["staging", "prod", "dev"]);
  act(() => moveContext(contexts, "dev", "staging"));
  expect(result.current.map(c => c.name)).toEqual(["dev", "staging", "prod"]);
  expect(loadContextOrder()).toEqual(["dev", "staging", "prod", "offline"]);
});
it("reads classic's stable-ID order after kubeconfig names change", () => {
  saveContextOrder(["staging-id", "prod-id"]);
  const contexts = [{ name: "config/prod", stableId: "prod-id" }, { name: "config/staging", stableId: "staging-id" }];
  const { result } = renderHook(() => useOrderedContexts(contexts));
  expect(result.current.map(c => c.stableId)).toEqual(["staging-id", "prod-id"]);
  act(() => moveContext(contexts, "config/prod", "config/staging"));
  expect(loadContextOrder()).toEqual(["prod-id", "staging-id"]);
});
it("persists a legacy name-keyed order as stable IDs when contexts become known", () => {
  saveContextOrder(["staging", "prod"]);
  const contexts = [{ name: "prod", stableId: "prod-id" }, { name: "staging", stableId: "staging-id" }];

  renderHook(() => useOrderedContexts(contexts));

  expect(loadContextOrder()).toEqual(["staging-id", "prod-id"]);
});
