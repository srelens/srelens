import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it } from "vitest";
import { loadContextOrder, saveContextOrder } from "@srelens/core";
import { moveContext, moveContextBy, removeContextFromOrder, useOrderedContexts } from "./contextOrder";
beforeEach(() => localStorage.clear());
it("uses classic's saved order and persists a move while keeping unlisted contexts", () => {
  saveContextOrder(["staging", "prod", "offline"]);
  const contexts = [{ name: "prod", stableId: "prod", key: "prod" }, { name: "staging", stableId: "staging", key: "staging" }, { name: "dev", stableId: "dev", key: "dev" }];
  const { result } = renderHook(() => useOrderedContexts(contexts));
  expect(result.current.map(c => c.name)).toEqual(["staging", "prod", "dev"]);
  act(() => moveContext(contexts, "dev", "staging"));
  expect(result.current.map(c => c.name)).toEqual(["dev", "staging", "prod"]);
  expect(loadContextOrder()).toEqual(["dev", "staging", "prod", "offline"]);
});
it("reads classic's stable-ID order after kubeconfig names change", () => {
  saveContextOrder(["staging-id", "prod-id"]);
  const contexts = [{ name: "config/prod", stableId: "prod-id", key: "prod-id" }, { name: "config/staging", stableId: "staging-id", key: "staging-id" }];
  const { result } = renderHook(() => useOrderedContexts(contexts));
  expect(result.current.map(c => c.stableId)).toEqual(["staging-id", "prod-id"]);
  act(() => moveContext(contexts, "config/prod", "config/staging"));
  expect(loadContextOrder()).toEqual(["prod-id", "staging-id"]);
});
it("persists a legacy name-keyed order as stable IDs when contexts become known", () => {
  saveContextOrder(["staging", "prod"]);
  const contexts = [{ name: "prod", stableId: "prod-id", key: "prod-id" }, { name: "staging", stableId: "staging-id", key: "staging-id" }];

  renderHook(() => useOrderedContexts(contexts));

  expect(loadContextOrder()).toEqual(["staging-id", "prod-id"]);
});
it("does not resolve an ambiguous legacy order from a workspace subset", () => {
  saveContextOrder(["prod"]);
  const all = [
    { name: "file-a/prod", stableId: "a-id", key: "a-id" },
    { name: "file-b/prod", stableId: "b-id", key: "b-id" },
  ];

  renderHook(() => useOrderedContexts([all[0]], all));

  expect(loadContextOrder()).toEqual(["prod"]);
});
it("forgets a confirmed deletion while retaining offline contexts", () => {
  saveContextOrder(["staging-id", "prod-id", "offline-id"]);
  removeContextFromOrder("prod-id");
  expect(loadContextOrder()).toEqual(["staging-id", "offline-id"]);
});

it("remembers ambiguous order entries after a duplicate disappears and the view remounts", () => {
  const all = [{ name: "file-a/prod", stableId: "a-id", key: "a-id" }, { name: "file-b/prod", stableId: "b-id", key: "b-id" }];
  saveContextOrder(["prod"]);
  const view = renderHook(() => useOrderedContexts(all)); view.unmount();
  renderHook(() => useOrderedContexts([all[1]]));
  expect(loadContextOrder()).toEqual(["prod"]);
});
it("does not migrate order from a partial context inventory", async () => {
  const { setContexts, resetContexts } = await import("./clusters");
  const all = [{ name: "prod", stableId: "a-id", key: "a-id" }];
  saveContextOrder(["prod"]);
  act(() => setContexts(all as import("@srelens/core").ClusterContext[], "Unreadable source"));
  const view = renderHook(() => useOrderedContexts(all));
  expect(loadContextOrder()).toEqual(["prod"]);
  view.unmount(); resetContexts();
});

/**
 * A workspace's rail holds only its own clusters (#829). Reordering them must
 * rewrite the slots they hold in the shared order and no others: every other
 * context — connected and in another workspace, or offline — stays where it is.
 */
const c = (name: string) => ({ name, stableId: name, key: name });
it("rewrites only the subset's own slots, leaving contexts between them where they were", () => {
  saveContextOrder(["prod", "other", "staging", "offline", "dev"]);
  const workspace = [c("prod"), c("staging"), c("dev")];

  act(() => moveContext(workspace, "dev", "prod", true));

  // dev, prod, staging into the first, third and fifth slots, in that order.
  expect(loadContextOrder()).toEqual(["dev", "other", "prod", "offline", "staging"]);
});
it("sent those contexts to the end without subset — which is right only for a caller holding every context", () => {
  saveContextOrder(["prod", "other", "staging", "dev"]);
  act(() => moveContext([c("prod"), c("staging"), c("dev")], "dev", "prod"));
  expect(loadContextOrder()).toEqual(["dev", "prod", "staging", "other"]);
});
it("moves a subset context to the end of its own slots, not of the whole order", () => {
  saveContextOrder(["prod", "other", "staging", "dev", "tail"]);
  act(() => moveContext([c("prod"), c("staging"), c("dev")], "prod", null, true));
  expect(loadContextOrder()).toEqual(["staging", "other", "dev", "prod", "tail"]);
});
it("appends a subset context the saved order never held, and keeps the rest in place", () => {
  saveContextOrder(["prod", "other", "staging"]);
  // `dev` is new: connected, in this workspace, never ordered.
  act(() => moveContext([c("prod"), c("staging"), c("dev")], "dev", "prod", true));
  expect(loadContextOrder()).toEqual(["dev", "other", "prod", "staging"]);
});
it("saves just the subset's order when nothing was saved before", () => {
  act(() => moveContext([c("prod"), c("staging"), c("dev")], "dev", "prod", true));
  expect(loadContextOrder()).toEqual(["dev", "prod", "staging"]);
});
it("steps a subset context one place either way without disturbing the others", () => {
  saveContextOrder(["prod", "other", "staging", "dev"]);
  const workspace = [c("prod"), c("staging"), c("dev")];

  act(() => moveContextBy(workspace, "dev", -1, true));
  expect(loadContextOrder()).toEqual(["prod", "other", "dev", "staging"]);

  act(() => moveContextBy(workspace, "prod", 1, true));
  expect(loadContextOrder()).toEqual(["dev", "other", "prod", "staging"]);
});
