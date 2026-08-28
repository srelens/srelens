// The three appearance axes — theme, accent, density — and the one place they
// are remembered.
//
// **Why this is not inside `AppearancePane`.** `data-theme` has two writers a
// reader can drive: that pane's Theme control, and the titlebar's light/dark
// button (`shell/Chrome`, calling the host's `toggleNextDesignTheme`). While the
// store lived in the pane, only the pane could write it — so the titlebar's
// choice was recorded nowhere, and boot's `applyStoredAppearance` put the
// pane's older value back over it. The reader's most recent explicit theme
// choice was discarded at the next launch with nothing to tell them why.
//
// Both writers now record through {@link remember}, and every write is
// PER-AXIS: picking an accent stores an accent and nothing else. Those two
// properties together are what make the two writers unable to fight — there is
// one record of the theme axis, whoever wrote it last owns it, and no write
// carries an axis its author was not asked about.
import { UI_SCALE, settingsStorage } from "@srelens/core";
import type { Storage } from "./tabsPersist";

export type ThemeId = "light" | "paper" | "dark" | "midnight" | "contrast";
export type AccentId = "violet" | "blue" | "teal" | "amber" | "rose";
export type DensityId = "compact" | "default" | "comfortable";

/** Where the three appearance choices are remembered, as one document. */
export const APPEARANCE_KEY = "srelens.next.appearance";

/**
 * §23's five themes, in §23's order, with §23's hints verbatim. Each id is the
 * `data-theme` value its token block is keyed on — `light` excepted, which is
 * the bare `:root` and so has no value at all.
 */
export const THEMES: ReadonlyArray<{ id: ThemeId; label: string; hint: string }> = [
  { id: "light", label: "Light", hint: "lavender paper, the default" },
  { id: "paper", label: "Paper", hint: "warm light, easier under office lighting" },
  { id: "dark", label: "Dark", hint: "ink violet control room" },
  { id: "midnight", label: "Midnight", hint: "near black, for a dark room" },
  { id: "contrast", label: "High contrast", hint: "AAA text, heavier rules, no washes" },
];

/** §23's five accents, in §23's order. `violet` is `:root`'s own `--accent`. */
export const ACCENTS: ReadonlyArray<{ id: AccentId; label: string }> = [
  { id: "violet", label: "Violet" },
  { id: "blue", label: "Blue" },
  { id: "teal", label: "Teal" },
  { id: "amber", label: "Amber" },
  { id: "rose", label: "Rose" },
];

/** §23's three densities. `default` is `:root`'s own row height. */
export const DENSITIES: ReadonlyArray<{ id: DensityId; label: string }> = [
  { id: "compact", label: "Compact" },
  { id: "default", label: "Default" },
  { id: "comfortable", label: "Comfortable" },
];

/**
 * The scales the app can actually be set to, from core's own bounds.
 *
 * `MAX` is appended when the range does not land on it, so widening
 * `UI_SCALE.MAX` to a value off the step grid cannot silently drop the top of
 * the range out of this control.
 */
export const ZOOM_STEPS: readonly number[] = (() => {
  const steps: number[] = [];
  for (let percent = UI_SCALE.MIN; percent <= UI_SCALE.MAX; percent += UI_SCALE.STEP) {
    steps.push(percent);
  }
  if (steps[steps.length - 1] !== UI_SCALE.MAX) steps.push(UI_SCALE.MAX);
  return steps;
})();

/** The value that means "no attribute", per axis. */
export const BARE = { theme: "light", accent: "violet", density: "default" } as const;

export interface Appearance {
  theme: ThemeId;
  accent: AccentId;
  density: DensityId;
}

function asTheme(value: unknown): ThemeId | null {
  return THEMES.find((t) => t.id === value)?.id ?? null;
}

function asAccent(value: unknown): AccentId | null {
  return ACCENTS.find((a) => a.id === value)?.id ?? null;
}

function asDensity(value: unknown): DensityId | null {
  return DENSITIES.find((d) => d.id === value)?.id ?? null;
}

/**
 * Write one axis onto the document root. The bare value removes the attribute
 * rather than writing itself, because `[data-theme="light"]`,
 * `[data-accent="violet"]` and `[data-density="default"]` are selectors no rule
 * in the stylesheet matches — the defaults live on `:root`.
 */
export function writeAxis(attribute: string, value: string, bare: string): void {
  const root = document.documentElement;
  if (value === bare) root.removeAttribute(attribute);
  else root.setAttribute(attribute, value);
}

export function readRootTheme(): ThemeId {
  return asTheme(document.documentElement.getAttribute("data-theme")) ?? BARE.theme;
}

export function readRootAccent(): AccentId {
  return asAccent(document.documentElement.getAttribute("data-accent")) ?? BARE.accent;
}

export function readRootDensity(): DensityId {
  return asDensity(document.documentElement.getAttribute("data-density")) ?? BARE.density;
}

/**
 * Watch the three attributes this pane and its two co-writers share.
 *
 * One observer per subscriber, torn down with it. The alternative — a
 * module-level observer — would outlive the pane and keep the document under
 * observation for the rest of the session for nothing.
 */
export function subscribeToRoot(onChange: () => void): () => void {
  const observer = new MutationObserver(onChange);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme", "data-accent", "data-density"],
  });
  return () => observer.disconnect();
}

export function readStored(storage: Storage): Partial<Appearance> {
  let doc: unknown;
  try {
    const raw = storage.getItem(APPEARANCE_KEY);
    if (raw === null) return {};
    doc = JSON.parse(raw);
  } catch {
    // Unreadable storage, or a document this build cannot parse. An appearance
    // that falls back to the design's default costs nothing; failing here
    // would cost the boot that calls this.
    return {};
  }
  if (typeof doc !== "object" || doc === null) return {};
  const record = doc as Record<string, unknown>;
  const theme = asTheme(record.theme);
  const accent = asAccent(record.accent);
  const density = asDensity(record.density);
  return {
    ...(theme === null ? {} : { theme }),
    ...(accent === null ? {} : { accent }),
    ...(density === null ? {} : { density }),
  };
}

/**
 * Put the remembered appearance on the document root.
 *
 * **The boot half of this pane.** Every axis here is a preference read once and
 * applied before anything renders, exactly like the design preference itself;
 * the pane is only the place it is chosen. `apps/desktop/src/main.tsx` calls
 * this beside `applyNextDesignTheme()` — after it, so a stored theme wins over
 * the light/dark preference that function derives from classic's store.
 *
 * **Only the axes the stored document actually carries are written**, and that
 * is the whole reason this is not a three-line spread over
 * {@link BARE}. `applyNextDesignTheme()` runs first and puts
 * `data-theme="dark"` on the root for anyone whose classic preference resolves
 * dark — which is the DEFAULT (`DEFAULT_THEME` in `apps/desktop/src/ui/theme.ts`
 * is `{ name: "slate", mode: "dark" }`). A pass that wrote every axis would
 * write `theme: "light"` for every reader who has never opened this pane, and
 * `writeAxis` spells light as the ABSENCE of the attribute — so it would strip
 * the dark that boot just set and the new design would come up light for
 * almost everyone. Writing nothing where nothing was chosen leaves that
 * preference standing.
 *
 * The other half of that rule is {@link remember}: it stores ONLY the axis its
 * caller was asked about, so a stored `theme` exists exactly where a reader
 * chose a theme — on this pane, or on the titlebar, whose button records
 * through {@link rememberTheme}. That is what makes the read below faithful
 * rather than merely conservative.
 *
 * Exported and tested rather than left implicit so that wiring it is one line
 * against a function that already provably works.
 */
export function applyStoredAppearance(storage: Storage = settingsStorage): void {
  const stored = readStored(storage);
  if (stored.theme !== undefined) writeAxis("data-theme", stored.theme, BARE.theme);
  if (stored.accent !== undefined) writeAxis("data-accent", stored.accent, BARE.accent);
  if (stored.density !== undefined) writeAxis("data-density", stored.density, BARE.density);
}

/**
 * Whether the reader has NAMED a theme, as opposed to being shown one.
 *
 * **Why the document cannot answer this.** `data-theme` has two kinds of value
 * on it: a choice out of {@link THEMES}, and the light/dark reading
 * `applyNextDesignTheme` derives from classic's preference. Those overlap
 * exactly where it matters — `dark` is both a derivation and the third named
 * theme, and a bare root is both "nothing has been read yet" and a chosen
 * `light`. So {@link readRootTheme} can say what the window is wearing and
 * never why. The stored record is the only thing that separates the two,
 * because {@link remember} is per-axis: a stored `theme` exists exactly where
 * the pane's Theme control or the titlebar's light/dark button put one.
 *
 * **What the host does with it.** `apps/desktop/src/main.tsx` boots by arming a
 * `prefers-color-scheme` listener for a reader whose classic mode is `system`,
 * and that listener writes `data-theme` too — knowing only `dark` and bare
 * light. Left armed, the next OS change turned a chosen Midnight into plain
 * dark and a chosen Paper into bare light, for the rest of the session. This
 * predicate is what stands it down. (#373 review)
 *
 * **All five themes count the same.** The pane offers no `System` entry, so
 * naming nothing is the only way to say "follow the OS" — and naming Dark is
 * as explicit as naming Midnight. A rule that let a chosen light or dark keep
 * following the OS would flip that reader at dusk while leaving the reader who
 * picked Midnight alone, with nothing in the control to explain the difference.
 *
 * A theme this build cannot read — an unparsable document, or an id no
 * stylesheet defines — is not a choice it can honour, so {@link readStored}
 * drops it and the OS keeps its vote.
 */
export function hasChosenTheme(storage: Storage = settingsStorage): boolean {
  return readStored(storage).theme !== undefined;
}

/**
 * Remember one axis, leaving the other two as they were STORED.
 *
 * Read-modify-write against the stored document, and never against the
 * document root. Reading the root looks equivalent and is not: it writes all
 * three axes on every call, so choosing an accent stored whatever `data-theme`
 * happened to be on the element at that moment — and boot puts
 * `applyNextDesignTheme()`'s dark there for every reader who has never opened
 * this pane. Pick Teal, click the titlebar's sun for light, restart: boot
 * applied light and then this store's stray `theme: "dark"` put dark back.
 *
 * Per-axis, a stored theme exists only where a reader chose a theme, and the
 * titlebar records its own choice through {@link rememberTheme} — so the two
 * writers share one record and the last explicit choice is the one that
 * survives.
 */
export function remember(patch: Partial<Appearance>, storage: Storage = settingsStorage): void {
  const next: Partial<Appearance> = { ...readStored(storage), ...patch };
  try {
    storage.setItem(APPEARANCE_KEY, JSON.stringify(next));
  } catch (error) {
    // Best-effort, as every other preference in this package is: a choice that
    // does not outlive the session beats a choice that cannot be made.
    console.error("could not persist the appearance preferences", error);
  }
}

/**
 * Record whatever theme is on the document now, for the titlebar's button.
 *
 * The button's own handler belongs to the host — `toggleNextDesignTheme`
 * (`apps/desktop/src/design.ts`) writes classic's preference and re-asserts
 * ui-next's `data-theme` — and this package cannot import it. So the titlebar
 * calls this straight afterwards and the resulting attribute is what gets
 * stored: not a mode this package guessed at, the value the host actually put
 * on the root. Synchronous, and it must stay called after the toggle rather
 * than before.
 */
export function rememberTheme(storage: Storage = settingsStorage): void {
  remember({ theme: readRootTheme() }, storage);
}
