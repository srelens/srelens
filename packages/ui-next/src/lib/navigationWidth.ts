import { useSyncExternalStore } from "react";
import { loadWorkspaceLayout, saveWorkspaceLayout } from "@srelens/core";

export const MIN_NAVIGATION_WIDTH = 180;
export const MAX_NAVIGATION_WIDTH = 420;
let dragging: number | null = null;
const listeners = new Set<() => void>();
const read = () => dragging ?? Math.max(MIN_NAVIGATION_WIDTH, Math.min(MAX_NAVIGATION_WIDTH, loadWorkspaceLayout().leftSidebarWidth));
const subscribe = (listener: () => void) => { listeners.add(listener); return () => { listeners.delete(listener); }; };
export function setNavigationWidth(width: number) {
  dragging = Math.max(MIN_NAVIGATION_WIDTH, Math.min(MAX_NAVIGATION_WIDTH, width));
  listeners.forEach((listener) => listener());
}
export function saveNavigationWidth(width: number) {
  saveWorkspaceLayout({ ...loadWorkspaceLayout(), leftSidebarWidth: Math.max(MIN_NAVIGATION_WIDTH, Math.min(MAX_NAVIGATION_WIDTH, width)) });
  dragging = null;
  listeners.forEach((listener) => listener());
}
export function useNavigationWidth() { return useSyncExternalStore(subscribe, read, read); }
