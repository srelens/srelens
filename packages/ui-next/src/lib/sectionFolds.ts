import { useSyncExternalStore } from "react";
import { settingsStorage } from "@srelens/core";
import type { Storage } from "./tabsPersist";

/**
 * Which blocks of a resource detail the reader has opened, remembered between
 * launches.
 *
 * The reader's own words: "by default everything is uncollapsed, first open
 * should keep everything collapsed, and from for next one remember what all
 * was uncollapsed". So a titled block opens shut, and opening one is a choice
 * that outlives the pane, the tab and the session.
 *
 * PER KIND, not per resource. Opening Conditions on one Pod means "I care
 * about Conditions on Pods", not "on this pod" — and per instance the
 * document would have no bound at all, since a cluster has thousands of pods
 * and every one of them peeked at would leave a row behind.
 *
 * The same shape as `marks.ts`, `peekWidth.ts` and the namespace selection in
 * `workspace.ts`: `settingsStorage` by default, so the desktop writes the
 * backend's settings file and the web writes `localStorage`, and injectable so
 * tests need a Map and no platform. Every accessor is wrapped, because
 * `settingsStorage` falls back to raw `localStorage` and `localStorage` throws
 * outright in a WebView with storage disabled — a refusing storage costs the
 * memory and nothing else.
 *
 * WHAT IT RECORDS IS A DEVIATION FROM THE DEFAULT, not a state. Almost every
 * block starts shut, so almost every entry is an id the reader OPENED. The
 * exception is a block whose pane has nothing else in it — the peek's
 * Containers tab, a ConfigMap's Data — which its own caller declares starts
 * open, because a pane that opens showing nothing at all is hostile; shutting
 * one of those is the deviation, and it is recorded as `!id`. A choice that
 * comes back round to the default is dropped again, so the document stays the
 * size of what the reader actually chose.
 *
 * WHAT THE DOCUMENT CAN AND CANNOT SAY, which is a security property rather
 * than a shape. There is no default entry, no wildcard, no version and no
 * migration that could fill a missing key in, so every path that fails — no
 * document, an unreadable one, one entry this build cannot parse, a kind
 * nobody has touched — arrives at THE CALLER'S DEFAULT, and the caller's
 * default is shut everywhere but the two blocks named above. Nothing a
 * document says can open a block whose caller did not ask for it to start
 * open: `!id` only ever closes. A `Secret`'s annotations can therefore never
 * be opened by anything but a reader opening a Secret's annotations — that
 * caller passes no default, a memory recorded on another kind is a different
 * key, and a value that is not a list of ids is dropped rather than read as
 * "everything". (The gate itself is a second lock behind this one — see
 * `AnnotationsSection`.)
 */
export const SECTION_FOLDS_KEY = "srelens.next.sectionFolds";

const isRecord = (v: unknown): v is Record<string, unknown> =>
  typeof v === "object" && v !== null && !Array.isArray(v);

/**
 * Anything but a map of `kind -> opened ids` reads as nothing opened.
 *
 * There is no version and nothing to migrate: a kind whose entry this build
 * cannot read is dropped on its own rather than taking the others with it —
 * losing one kind's folds is a nuisance, losing every kind's is not. A
 * non-list entry is dropped rather than interpreted; see the module comment
 * for why that direction is the only safe one.
 */
export function parseStoredSectionFolds(raw: string | null): Record<string, string[]> {
  if (!raw) return {};
  let doc: unknown;
  try {
    doc = JSON.parse(raw);
  } catch {
    return {};
  }
  if (!isRecord(doc)) return {};
  const folds: Record<string, string[]> = {};
  for (const [kind, value] of Object.entries(doc)) {
    if (!Array.isArray(value)) continue;
    const ids = value.filter((id): id is string => typeof id === "string");
    // An empty list is the shut state spelled a second way, and two spellings
    // of one state is how they come to disagree.
    if (ids.length > 0) folds[kind] = ids;
  }
  return folds;
}

let folds: Record<string, string[]> = {};
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
 * Read the saved folds once at boot — and in tests, as often as they like.
 *
 * Guarded like every accessor in `marks.ts`. Boot must reach
 * `setBooted(true)`, so a storage that refuses to be read costs the reader
 * their opened blocks and nothing else.
 */
export function loadSectionFolds(storage: Storage = settingsStorage): void {
  let next: Record<string, string[]> = {};
  try {
    next = parseStoredSectionFolds(storage.getItem(SECTION_FOLDS_KEY));
  } catch (error) {
    console.error("could not read the saved detail sections", error);
  }
  folds = next;
  emit();
}

function save(storage: Storage) {
  try {
    storage.setItem(SECTION_FOLDS_KEY, JSON.stringify(folds));
  } catch (error) {
    // Best-effort, as `settingsStorage` itself is: a block that does not stay
    // open past this launch is better than one that cannot be opened.
    console.error("could not persist the detail sections", error);
  }
}

/**
 * How a reader's choice to SHUT a block that starts open is written down.
 *
 * A prefix rather than a second document: the shape stays `kind -> ids`, so
 * every launch that has ever written one of these can still read it, and the
 * parser needs no version. It reads as "not open", which is what it means, and
 * no section heading in this design begins with it. It can only ever close a
 * block — see the module comment for why that direction is the one that
 * matters.
 */
const SHUT = "!";

/** What a caller declares about a block that has nothing remembered yet. */
export interface SectionFoldOptions {
  /**
   * Start open rather than shut. For a block that is the only content of its
   * pane, which would otherwise open the pane completely empty. Absent — the
   * default of the default — is shut, so a caller that says nothing gets the
   * shut-everything rule and no block can start open by accident.
   */
  defaultOpen?: boolean;
  storage?: Storage;
}

/**
 * Whether this kind's block is open. A boolean, so `useSyncExternalStore`
 * compares snapshots by value and there is no cache to keep — the composed
 * answers in `marks.ts` needed one.
 *
 * Unknown kind, unknown id, no document at all: `defaultOpen`, which is false
 * unless the block's own caller asked otherwise. There is no path through this
 * function that opens a block whose caller did not ask for it to start open.
 */
export function isSectionOpen(kind: string, id: string, defaultOpen = false): boolean {
  const ids = folds[kind];
  if (!ids) return defaultOpen;
  if (ids.includes(id)) return true;
  if (ids.includes(`${SHUT}${id}`)) return false;
  return defaultOpen;
}

/** Open this kind's block, or shut it, and keep the answer. */
export function setSectionOpen(
  kind: string,
  id: string,
  open: boolean,
  { defaultOpen = false, storage = settingsStorage }: SectionFoldOptions = {},
): void {
  const current = folds[kind] ?? [];
  // Whatever this block used to say, gone: a block records one choice, and
  // the two spellings of it must never both be present.
  const rest = current.filter((each) => each !== id && each !== `${SHUT}${id}`);
  // A choice that has come back round to the default is not a choice — drop
  // it, so the document stays the size of what the reader actually chose.
  const next = open === defaultOpen ? rest : [...rest, open ? id : `${SHUT}${id}`];
  if (next.length === current.length && next.every((each, i) => each === current[i])) return;
  if (next.length > 0) {
    folds = { ...folds, [kind]: next };
  } else {
    const { [kind]: _none, ...others } = folds;
    folds = others;
  }
  emit();
  save(storage);
}

/**
 * Whether the block is open, re-rendering whoever reads it when that changes.
 *
 * `null` for either half — a section drawn outside a detail host, or one whose
 * heading gives no stable id — is "nothing is remembered about this", which
 * reads shut here and is what stops the caller offering a control at all.
 */
export function useSectionOpen(kind: string | null, id: string | null, defaultOpen = false): boolean {
  const read = () => (kind !== null && id !== null ? isSectionOpen(kind, id, defaultOpen) : false);
  return useSyncExternalStore(subscribe, read, read);
}
