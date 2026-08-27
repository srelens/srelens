import { useEffect, useState, useSyncExternalStore } from "react";
import {
  UI_SCALE,
  applyUiScale,
  getUiScale,
  isApplePlatform,
  isTauri,
  setUiScale,
  settingsStorage,
} from "@srelens/core";
import { Button, Panel } from "@srelens/ui-kit";
import { hint as chordHint } from "../../lib/shortcuts";
import type { Storage } from "../../lib/tabsPersist";

/**
 * §23's `Appearance` pane: which theme the window wears, which accent, how big
 * the interface is, and how tightly the rows sit — plus the way back to the
 * classic design, which §23 does not draw.
 *
 * **§J.1 does not cover any of this, and that is the point of checking.** The
 * mock's "Invented data fields" appendix lists only data a Kubernetes API
 * would have to return — incidents, metrics, object counts, provider
 * metadata. Appearance appears nowhere in §J at all, so §J.1 neither blesses
 * nor condemns §23's five themes, five accents and seven zoom steps. The check
 * that decides them is the stylesheet, and it was made against
 * `packages/ui-kit/src/styles/tokens.css`:
 *
 * - **All five themes are real.** `[data-theme="dark"]`, `"paper"`,
 *   `"midnight"` and `"contrast"` each have a full token block, and `light` is
 *   the bare `:root` — which is why picking Light REMOVES the attribute rather
 *   than writing `data-theme="light"`, a selector no rule matches.
 * - **All five accents are real, as tokens.** §23 prints them as hex literals
 *   (`#4b3bd6`, `#1f5fd0`, `#0d7068`, `#9a5c05`, `#ad2456`); every one of those
 *   values is already in `[data-accent="…"]`, with a second variant for the
 *   dark grounds. So this file names no colour: each swatch carries the
 *   `data-accent` (and `data-theme`) the token block is keyed on and paints
 *   itself with `--accent` resolved locally. That is also the only way a
 *   swatch can show the DARK variant of an accent on a dark ground, which a
 *   hex literal could never do.
 * - **§23's zoom scale is not what the app supports.** It lists
 *   `80 90 100 110 125 140 160`. `UI_SCALE` in `packages/core/src/lib/uiScale.ts`
 *   is `{ MIN: 80, MAX: 150, STEP: 10 }`, so `160%` clamps to 150 — a segment
 *   that silently gives you something else — and `125%` is off the step grid
 *   the Cmd/Ctrl +/- chords walk, so landing on it would leave the next
 *   keystroke moving to a value this control cannot show as selected. The
 *   steps are therefore DERIVED from `UI_SCALE`, which keeps the segmented
 *   control and the keyboard talking about the same ladder.
 * - **§23's density hints claim a text size density does not change.**
 *   `12px text`, `13px text`, `14px text` — but `[data-density]` sets
 *   `--row-h`, `--pad-y` and `--pane-head-h` and no font size anywhere. So the
 *   hint here says what density actually moves, and points at Interface zoom
 *   for the thing §23 promised.
 *
 * **The pane reads the document, not its own store.** `data-theme` has three
 * writers already — boot (`applyNextDesignTheme`), the titlebar's theme button
 * (`toggleNextDesignTheme`) and now this pane — so a pane that trusted what it
 * last saved would sit there showing Midnight over a document that is plainly
 * dark. A `MutationObserver` on the root is the same trick `Terminals.tsx`
 * already uses for the one other thing in this app that has to know the theme.
 *
 * **The boot half is {@link applyStoredAppearance}, and it is wired.**
 * `apps/desktop/src/main.tsx` calls it after its own `applyNextDesignTheme()`
 * and before `createRoot`, so a chosen theme, accent or density is back on the
 * root before the first paint of the next launch. That ordering is not
 * arbitrary and neither is what the function writes — see its own doc.
 */

/* ------------------------------------------------------------------ *
 * step-11 scaffolding
 *
 * §23 draws NO design toggle. That is not an omission to be corrected: the
 * mock is drawn as of **step 11** of the migration plan, by which point the
 * classic design, this toggle and the Placeholder screen have all been
 * deleted. Every piece of migration scaffolding is absent from it by
 * construction.
 *
 * Step 8 requires the opposite — the toggle must ship in BOTH designs until
 * that last step — so this pane deliberately carries a control the design does
 * not show. It lives in Appearance because that is where
 * `apps/desktop/src/components/AppearanceSettingsSection.tsx` puts it in
 * classic, so a reader switching designs finds it in the same place in both.
 *
 * WHAT DELETES IT: step 11 of
 * `docs/superpowers/plans/2026-08-20-new-design-migration-*` — the step that
 * removes `apps/desktop/src/design.ts`, `AppearanceSettingsSection.tsx` and
 * `packages/ui-next/src/shell/Placeholder.tsx`. When `design.ts` goes, the
 * `Design` panel below and the `ported` / `onSwitchToClassic` props go with it,
 * and this pane matches §23 exactly. Not before: deleting it against the mock
 * takes the only way back to a working design with it.
 * ------------------------------------------------------------------ */

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
const BARE = { theme: "light", accent: "violet", density: "default" } as const;

interface Appearance {
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
function writeAxis(attribute: string, value: string, bare: string): void {
  const root = document.documentElement;
  if (value === bare) root.removeAttribute(attribute);
  else root.setAttribute(attribute, value);
}

function readRootTheme(): ThemeId {
  return asTheme(document.documentElement.getAttribute("data-theme")) ?? BARE.theme;
}

function readRootAccent(): AccentId {
  return asAccent(document.documentElement.getAttribute("data-accent")) ?? BARE.accent;
}

function readRootDensity(): DensityId {
  return asDensity(document.documentElement.getAttribute("data-density")) ?? BARE.density;
}

/**
 * Watch the three attributes this pane and its two co-writers share.
 *
 * One observer per subscriber, torn down with it. The alternative — a
 * module-level observer — would outlive the pane and keep the document under
 * observation for the rest of the session for nothing.
 */
function subscribeToRoot(onChange: () => void): () => void {
  const observer = new MutationObserver(onChange);
  observer.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ["data-theme", "data-accent", "data-density"],
  });
  return () => observer.disconnect();
}

function readStored(storage: Storage): Partial<Appearance> {
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
 * preference standing, and a document written by {@link remember} always
 * carries all three axes, so a reader who HAS chosen still gets all three back.
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
 * Remember one axis, keeping the other two as they are on the document.
 *
 * Read-modify-write against the document rather than against a cached copy:
 * the titlebar's theme button can have moved `data-theme` since this pane
 * mounted, and a blind spread would write the stale value back over it.
 */
function remember(patch: Partial<Appearance>, storage: Storage = settingsStorage): void {
  const next: Appearance = {
    theme: readRootTheme(),
    accent: readRootAccent(),
    density: readRootDensity(),
    ...patch,
  };
  try {
    storage.setItem(APPEARANCE_KEY, JSON.stringify(next));
  } catch (error) {
    // Best-effort, as every other preference in this package is: a choice that
    // does not outlive the session beats a choice that cannot be made.
    console.error("could not persist the appearance preferences", error);
  }
}

/**
 * The pixel size of the body text at a given zoom, for §23's hint.
 *
 * Read off the document rather than repeated from `html { font-size: 16px }` in
 * the kit's stylesheet, so the sentence cannot outlive the rule it describes.
 * Native webview zoom does not change a computed CSS pixel, so the effective
 * size on screen is the computed one scaled by the percentage.
 */
function bodyPixels(percent: number): number {
  let base = 16;
  try {
    const computed = Number.parseFloat(getComputedStyle(document.documentElement).fontSize);
    if (Number.isFinite(computed) && computed > 0) base = computed;
  } catch {
    // No stylesheet attached — a unit test, or a first paint.
  }
  return Math.round((base * percent) / 100);
}

/** The current row height, when a stylesheet is attached to say. */
function rowHeight(): string {
  try {
    return getComputedStyle(document.documentElement).getPropertyValue("--row-h").trim();
  } catch {
    return "";
  }
}

export interface AppearancePaneProps {
  /**
   * Display names of the screens that exist in the new design — step-11
   * scaffolding, passed in rather than imported.
   *
   * **ui-next cannot reach `apps/desktop/src/design`.** This package depends on
   * `@srelens/core` and `@srelens/ui-kit` and nothing else; `apps/desktop`
   * depends on THIS package, so an import the other way is a cycle across a
   * package boundary with no path to follow. The host injects instead, which is
   * the path `NextApp` already offers: `main.tsx` passes
   * `PORTED_SCREENS.map((s) => s.name)` as `ported` and a closure over
   * `switchDesign("classic")` as `onExit`, and `Placeholder` consumes exactly
   * those two — "passed in rather than imported, so the kit's gallery and this
   * package's tests do not depend on `apps/desktop`", as it says itself.
   */
  ported: readonly string[];
  /**
   * Leave the new design. Step-11 scaffolding.
   *
   * A callback rather than a call into the design module for the reason above,
   * and it must stay one: switching back has to work when this screen is the
   * thing being left. `NextApp`'s `leave` is what this is for — it saves the
   * handoff, calls `switchDesign`, and renders a refusal at the window root,
   * because the classic toast host is not in this tree.
   */
  onSwitchToClassic: () => void;
}

export function AppearancePane({ ported, onSwitchToClassic }: AppearancePaneProps) {
  const theme = useSyncExternalStore(subscribeToRoot, readRootTheme);
  const accent = useSyncExternalStore(subscribeToRoot, readRootAccent);
  const density = useSyncExternalStore(subscribeToRoot, readRootDensity);

  // Zoom has no change notification: the Cmd/Ctrl chords and the titlebar's
  // three buttons both go through `Chrome`'s `zoom`, which writes core's
  // setting and asks the webview to scale. Native zoom does move the layout
  // viewport, though, so a resize is the one signal that reaches here.
  const [scale, setScale] = useState(getUiScale);
  useEffect(() => {
    const reread = () => setScale(getUiScale());
    window.addEventListener("resize", reread);
    return () => window.removeEventListener("resize", reread);
  }, []);

  // Desktop only, for the reason `Chrome` hides its zoom buttons on the web:
  // `applyUiScale` asks the webview to zoom, there is no webview to ask in a
  // browser, and the browser's own zoom already does this. A control that
  // quietly does nothing is worse than a sentence saying where the control is.
  const desktop = isTauri();
  const apple = isApplePlatform();
  const rows = rowHeight();

  function pickTheme(id: ThemeId) {
    writeAxis("data-theme", id, BARE.theme);
    remember({ theme: id });
  }

  function pickAccent(id: AccentId) {
    writeAxis("data-accent", id, BARE.accent);
    remember({ accent: id });
  }

  function pickDensity(id: DensityId) {
    writeAxis("data-density", id, BARE.density);
    remember({ density: id });
  }

  function pickScale(percent: number) {
    // `setUiScale` clamps and returns what it stored, so what is applied and
    // what is shown are both the stored value rather than the asked-for one.
    const stored = setUiScale(percent);
    setScale(stored);
    applyUiScale(stored);
  }

  return (
    <div className="flex flex-col gap-4">
      <Panel title="Theme">
        {/*
          `flex flex-wrap` with a fixed basis on each card, not a five-column
          row: the content pane sits beside a 196px nav rail, and five cards in
          a row do not fit it at any window width worth supporting. Every card
          is `shrink-0` so the labels cannot be squeezed to nothing — a flex
          child with nothing stopping it shrinking is where `min-width: auto`
          has cost this migration eight defects, and jsdom sees none of them.
        */}
        <div className="flex flex-wrap gap-2" role="radiogroup" aria-label="Theme">
          {THEMES.map((option) => (
            <label
              key={option.id}
              data-testid="theme-card"
              data-theme-id={option.id}
              className="w-[9.5rem] max-w-full shrink-0 cursor-pointer rounded-[var(--radius-tile)] border border-rule p-2 hover:border-rule-strong has-[:checked]:border-accent has-[:checked]:bg-accent-wash has-[:focus-visible]:[outline:2px_solid_var(--accent)]"
            >
              {/* Visually hidden rather than absent, as in the kit's own
                  segmented control: the label IS the card, and the input under
                  it is what the browser gives arrow-key navigation and the
                  announced checked state to. */}
              <input
                type="radio"
                name="next-theme"
                className="sr-only"
                checked={theme === option.id}
                onChange={() => pickTheme(option.id)}
              />
              {/*
                The preview wears the theme it offers, so every colour in it
                comes from that theme's own tokens and this file names none.

                Ground plus ink, not §23's "two-tone" ground plus surface:
                under `contrast` those two are the same white, so a
                canvas/surface swatch would draw a blank tile for the one theme
                whose entire subject is contrast.
              */}
              <span
                aria-hidden="true"
                data-theme={option.id === BARE.theme ? undefined : option.id}
                className="block rounded-[5px] border border-rule bg-canvas px-1.5 py-2"
              >
                <span className="block h-1.5 w-3/5 rounded-full bg-ink" />
                <span className="mt-1 block h-1.5 w-2/5 rounded-full bg-muted" />
              </span>
              <span data-testid="theme-label" className="mt-2 block text-[0.75rem] font-medium text-ink">
                {option.label}
              </span>
              <span className="mt-0.5 block text-[0.6875rem] leading-snug text-muted">
                {option.hint}
              </span>
            </label>
          ))}
        </div>
      </Panel>

      <Panel title="Accent">
        <div className="flex flex-wrap gap-2" role="radiogroup" aria-label="Accent">
          {ACCENTS.map((option) => (
            <label
              key={option.id}
              data-testid="accent-card"
              className="flex shrink-0 cursor-pointer items-center gap-1.5 rounded-[var(--radius-tile)] border border-rule px-2 py-1.5 hover:border-rule-strong has-[:checked]:border-accent has-[:checked]:bg-accent-wash has-[:focus-visible]:[outline:2px_solid_var(--accent)]"
            >
              <input
                type="radio"
                name="next-accent"
                className="sr-only"
                checked={accent === option.id}
                onChange={() => pickAccent(option.id)}
              />
              {/*
                Both attributes, on the swatch itself. The accent's dark
                variants are compound selectors — `[data-theme="dark"][data-accent="blue"]`
                — so a swatch carrying only `data-accent` would show the light
                blue on a dark ground, which is the wrong colour for the thing
                it is offering.
              */}
              <span
                aria-hidden="true"
                data-testid="accent-swatch"
                data-theme={theme === BARE.theme ? undefined : theme}
                data-accent={option.id === BARE.accent ? undefined : option.id}
                className="block h-3.5 w-3.5 shrink-0 rounded-full bg-accent"
              />
              <span data-testid="accent-label" className="text-[0.75rem] text-ink">
                {option.label}
              </span>
            </label>
          ))}
        </div>
      </Panel>

      <Panel title="Text size and density">
        {desktop ? (
          <div className="flex flex-wrap items-center gap-x-3 gap-y-1">
            <span className="min-w-0 flex-1 basis-40">
              <span className="block text-[0.75rem] font-medium text-ink">Interface zoom</span>
              <span data-testid="zoom-hint" className="block text-[0.6875rem] leading-snug text-muted">
                {/* The chords come from `lib/shortcuts.ts`, so rebinding zoom
                    cannot leave this sentence naming a key nothing answers. */}
                Scales the whole window. {chordHint("zoom-in", apple)} and{" "}
                {chordHint("zoom-out", apple)} anywhere. Currently {bodyPixels(scale)}px body text.
              </span>
            </span>
            {/*
              `data-variant="segmented"` and `min-w-0`: eight steps is wider
              than this pane at any nav-rail layout, and the kit's variant makes
              the control carry its own overflow instead of pushing the pane
              sideways. Without the `min-w-0` the flex item's `min-width: auto`
              refuses to shrink and the `max-width: 100%` inside never bites.
            */}
            <span className="seg min-w-0 shrink" data-variant="segmented" role="radiogroup" aria-label="Interface zoom">
              {ZOOM_STEPS.map((percent) => (
                <label
                  key={percent}
                  className="seg-btn cursor-pointer has-[:focus-visible]:[outline:2px_solid_var(--accent)]"
                  data-on={scale === percent}
                >
                  <input
                    type="radio"
                    name="next-zoom"
                    className="sr-only"
                    checked={scale === percent}
                    onChange={() => pickScale(percent)}
                  />
                  <span data-testid="zoom-label">{percent}%</span>
                </label>
              ))}
            </span>
          </div>
        ) : (
          <p className="text-[0.75rem] leading-relaxed text-muted">
            Your browser&apos;s own zoom scales srelens here, so there is nothing for this pane to
            set — {chordHint("zoom-in", apple)} and {chordHint("zoom-out", apple)} work as they do on
            any page. Currently {bodyPixels(UI_SCALE.DEFAULT)}px body text at the browser&apos;s
            default zoom.
          </p>
        )}

        <div className="mt-3 flex flex-wrap items-center gap-x-3 gap-y-1">
          <span className="min-w-0 flex-1 basis-40">
            <span className="block text-[0.75rem] font-medium text-ink">Density</span>
            {/*
              §23's hints for this row are `12px text`, `13px text` and
              `14px text`. The `[data-density]` blocks set `--row-h`, `--pad-y`
              and `--pane-head-h` and no font size at all, so that copy would
              claim a change srelens does not make; this says what density does
              move, and where the text size actually lives.
            */}
            <span data-testid="density-hint" className="block text-[0.6875rem] leading-snug text-muted">
              How tall a row is and how much padding it carries{rows === "" ? "" : ` — ${rows} rows now`}.
              Text size does not change; Interface zoom is what scales that.
            </span>
          </span>
          {/* The variant here too, though three segments usually fit: with the
              row wrapped, the control gets its own line and `Comfortable` can
              still be wider than a narrow content pane. Carrying its own
              overflow keeps that from becoming a horizontal scroll on the
              whole page — which is what an un-shrinkable flex child does, and
              jsdom shows none of it. */}
          <span className="seg min-w-0 shrink" data-variant="segmented" role="radiogroup" aria-label="Density">
            {DENSITIES.map((option) => (
              <label
                key={option.id}
                className="seg-btn cursor-pointer has-[:focus-visible]:[outline:2px_solid_var(--accent)]"
                data-on={density === option.id}
              >
                <input
                  type="radio"
                  name="next-density"
                  className="sr-only"
                  checked={density === option.id}
                  onChange={() => pickDensity(option.id)}
                />
                <span data-testid="density-label">{option.label}</span>
              </label>
            ))}
          </span>
        </div>
      </Panel>

      {/*
        ─────────────────────────────────────────────────────────────────
        STEP-11 SCAFFOLDING. §23 draws no `Design` panel; see the note at
        the top of this file for why that is not an omission to correct.
        Deleted by migration step 11, together with `apps/desktop/src/design.ts`
        and `AppearanceSettingsSection.tsx`, and not before.
        ─────────────────────────────────────────────────────────────────
      */}
      <Panel title="Design">
        <p className="text-[0.75rem] leading-relaxed text-muted">
          You are in the new design. It is <strong className="text-ink">in progress</strong>: some
          screens are not there yet, and the ones that are may still change. Switching reloads the
          window, and the classic design has this same choice in the same place.
        </p>
        {ported.length > 0 && (
          <>
            <p className="mt-2 text-[0.75rem] text-muted">In the new design so far:</p>
            <ul className="mt-1 list-inside list-disc text-[0.75rem] text-muted">
              {ported.map((name) => (
                <li key={name} data-testid="ported-screen">
                  {name}
                </li>
              ))}
            </ul>
          </>
        )}
        <div className="mt-3 flex flex-wrap items-center gap-2">
          <Button variant="secondary" onClick={onSwitchToClassic}>
            Switch to the classic design
          </Button>
        </div>
      </Panel>
    </div>
  );
}
