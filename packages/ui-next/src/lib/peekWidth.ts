import { useLayoutEffect, useMemo, useState, useSyncExternalStore } from "react";
import { settingsStorage } from "@srelens/core";
import type { Storage } from "./tabsPersist";

/**
 * How wide the resource list's detail peek is, remembered between launches.
 *
 * The same shape as `lib/marks.ts` and `lib/columnPrefs.ts`, with the smallest
 * payload any of them carries: one number, not one per kind. A width per kind
 * would be a preference the reader has to set 34 times before the app agrees
 * with them, and nobody wants pods and deployments peeked at different widths.
 *
 * Persisted through `settingsStorage` like its two neighbours — the desktop's
 * backend settings file, `localStorage` on the web — and injectable, so tests
 * need a Map and no platform. Every accessor is wrapped: `settingsStorage`
 * falls back to raw `localStorage` when the backend file is unavailable, and
 * `localStorage` throws outright in a WebView with storage disabled. A width
 * that does not survive the session is better than a pane that cannot be
 * dragged.
 *
 * What this module deliberately has no opinion about is layout. It held one
 * for a round — it clamped against `window.innerWidth` — and the window was
 * the wrong box: the cluster rail and the navigation sidebar are outside the
 * list entirely, and the sidebar is itself resizable, so a ceiling derived
 * from the window handed the peek space the list never had to give. The space
 * the two panes share is the flex row they are siblings in, which only the
 * screen can measure. So the store holds a preference and {@link peekBounds}
 * is a pure function of a measurement the call site takes. (task 17 review)
 */
export const PEEK_WIDTH_KEY = "srelens.next.peekWidth";

/** What the pane opens at before anyone has dragged it: the 22rem it shipped as. */
export const DEFAULT_PEEK_WIDTH = 352;

/**
 * Narrower than this and the pane stops being able to show what it holds:
 * `Inspector`'s tab strip wraps, its facts stack, and the YAML pane turns
 * into one word per line.
 */
export const MIN_PEEK_WIDTH = 260;

/** Wider than this it is no longer a peek at the row, it is the screen. */
export const MAX_PEEK_WIDTH = 640;

/**
 * What the list keeps for itself whatever the peek asks for — roughly a name,
 * a namespace and a status column. Measured against the row the list and the
 * peek share, never against the window.
 */
export const MIN_LIST_WIDTH = 360;

/** The range the peek may be dragged through, given the room it has. */
export interface PeekBounds {
  minWidth: number;
  maxWidth: number;
}

/**
 * The bounds the pane can honour inside a row this wide.
 *
 * `available` is the width of the box the list and the peek share. Zero means
 * nobody has measured it yet — a first render, or a host that does no layout —
 * and the answer is then the absolute ceiling, since a not-yet-measured row is
 * no reason to pin the pane to its minimum. React runs the measuring layout
 * effect before paint, so the reader never sees that state.
 *
 * A row with room for neither hands back a minimum for a maximum rather than a
 * maximum below it: the pane stays legible and the table scrolls inside itself,
 * which is what `min-w-0` on the list's column is there for.
 */
export function peekBounds(available: number): PeekBounds {
  const room = available > 0 ? available - MIN_LIST_WIDTH : MAX_PEEK_WIDTH;
  return {
    minWidth: MIN_PEEK_WIDTH,
    maxWidth: Math.max(MIN_PEEK_WIDTH, Math.min(MAX_PEEK_WIDTH, Math.round(room))),
  };
}

const clamp = (value: number, min: number, max: number) => Math.max(min, Math.min(max, Math.round(value)));

/**
 * The width to actually render at.
 *
 * Applied on the way out rather than written back, so a pane squeezed by a
 * narrow row widens again to what the reader chose when the row does, instead
 * of the app having quietly forgotten their choice.
 */
export function clampPeekWidth(width: number, bounds: PeekBounds): number {
  return clamp(width, bounds.minWidth, bounds.maxWidth);
}

/**
 * Anything but a positive, finite number reads as no stored width at all —
 * and the pane opens at its default rather than at `NaN`, which renders as no
 * pane the reader can see.
 */
export function parseStoredPeekWidth(raw: string | null): number | null {
  if (!raw) return null;
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return null;
  }
  if (typeof doc !== "number" || !Number.isFinite(doc) || doc <= 0) return null;
  return doc;
}

/** The width the reader chose, within the absolute bounds and nothing else's. */
let chosen = DEFAULT_PEEK_WIDTH;
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * The chosen width. A number, so `useSyncExternalStore` gets a snapshot that
 * is stable by value for free — the composed object this used to hand back
 * needed a cache to stop the store tearing.
 */
export function peekWidth(): number {
  return chosen;
}

/**
 * Read the saved width once at boot — and in tests, as often as they like.
 *
 * Guarded like every accessor in `marks.ts`/`columnPrefs.ts`. Boot must reach
 * `setBooted(true)`, so a refusing storage costs the remembered width and
 * nothing else.
 */
export function loadPeekWidth(storage: Storage = settingsStorage): void {
  let stored: number | null = null;
  try {
    stored = parseStoredPeekWidth(storage.getItem(PEEK_WIDTH_KEY));
  } catch (error) {
    console.error("could not read the saved detail width", error);
  }
  chosen = clamp(stored ?? DEFAULT_PEEK_WIDTH, MIN_PEEK_WIDTH, MAX_PEEK_WIDTH);
  emit();
}

/** Mid-drag: the pane follows the pointer, and nothing is written. */
export function setPeekWidth(width: number): void {
  chosen = clamp(width, MIN_PEEK_WIDTH, MAX_PEEK_WIDTH);
  emit();
}

/** The resize settled — this is the one worth keeping. */
export function savePeekWidth(width: number, storage: Storage = settingsStorage): void {
  setPeekWidth(width);
  try {
    storage.setItem(PEEK_WIDTH_KEY, JSON.stringify(chosen));
  } catch (error) {
    // Best-effort, as `settingsStorage` itself is: a width that does not
    // survive the session is better than a width that cannot be set.
    console.error("could not persist the detail width", error);
  }
}

/** The peek's chosen width, re-rendering whoever reads it when it changes. */
export function usePeekWidth(): number {
  return useSyncExternalStore(subscribe, peekWidth, peekWidth);
}

/** A measured box: put {@link PeekRoom.ref} on it, read the bounds off it. */
export interface PeekRoom {
  /** Goes on the box the list and the peek share. */
  ref: (node: HTMLElement | null) => void;
  bounds: PeekBounds;
}

/**
 * The bounds for the row this hook's ref is put on — the box the two panes
 * share.
 *
 * Observed rather than read once: the row's width changes when the window
 * does, when the reader drags the navigation sidebar, and when the rail
 * appears — none of which this screen hears about any other way. The
 * observation is of the *row*, whose width does not depend on how the peek
 * inside it is sized (`min-w-0` on the list's column is what guarantees that),
 * so measuring cannot chase its own tail.
 *
 * A callback ref rather than a `useRef` handed in, because the row is not
 * always there on the first render: a custom resource's descriptor waits on
 * CRD discovery, so `Resources` renders a loading state first and the row
 * arrives later. An effect keyed on a ref OBJECT never re-runs when that
 * happens — a ref's identity never changes — so it would observe nothing at
 * all, `available` would stay 0, and every CRD list would quietly fall back to
 * the absolute ceiling. Keyed on the node, the effect runs the moment there is
 * something to measure. `setNode` is a state setter, so the ref itself is
 * stable and React never detaches and re-attaches it. (task 17 review)
 */
export function usePeekBounds(): PeekRoom {
  const [node, setNode] = useState<HTMLElement | null>(null);
  const [available, setAvailable] = useState(0);

  useLayoutEffect(() => {
    if (!node) return;
    setAvailable(node.getBoundingClientRect().width);
    if (typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver((entries) => {
      const entry = entries[entries.length - 1];
      setAvailable(entry?.contentRect.width ?? node.getBoundingClientRect().width);
    });
    observer.observe(node);
    return () => observer.disconnect();
  }, [node]);

  const bounds = useMemo(() => peekBounds(available), [available]);
  return { ref: setNode, bounds };
}
