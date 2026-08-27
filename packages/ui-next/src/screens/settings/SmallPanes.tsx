import { isApplePlatform } from "@srelens/core";
import { Button, Panel } from "@srelens/ui-kit";
import { openTab } from "../../lib/tabsStore";
import { hint, type WindowAction } from "../../lib/shortcuts";

/**
 * §23's three small panes: `Accessibility`, `Shortcuts` and `Clusters`.
 *
 * **§J.1 has nothing to say about any of them.** The mock's "Invented data
 * fields" appendix (`docs/superpowers/specs/mock-full-design.md`) is grouped by
 * what a field would take to ship — a new store, an APM backend, an indexing
 * layer — and every row in it is Kubernetes- or agent-shaped: incident ids,
 * run records, audit verdicts. None of the three subjects here are data at
 * all. Accessibility and Shortcuts are read off things that already exist
 * (the stylesheet, `lib/shortcuts.ts`); Clusters deliberately holds none.
 * Checking J.1 was still worth doing — a sibling task found it silent on its
 * own pane too, and "the appendix doesn't mention it" is only reassuring once
 * you have looked.
 */

/**
 * `Shortcuts` — every row's chord comes from {@link hint}, which reads the
 * same `BINDINGS` the window actually listens on
 * (`packages/ui-next/src/lib/shortcuts.ts`). This file names no key: it
 * supplies a label and an action type, and `hint` supplies the glyphs. A
 * binding that changes, or is rebound to a different chord, changes what
 * renders here without anyone touching this file — the alternative, a table
 * of glyphs typed out by hand, is wrong the moment `BINDINGS` moves and stays
 * wrong until someone happens to notice.
 *
 * The list of rows is every variant of `WindowAction["type"]` the module
 * declares, in the order the window's own dispatch reads naturally (console,
 * then the tab actions, then zoom) — not §23's ten-row table, which mixes in
 * chords this pane has no source for (`/` in the console, `↑ ↓`, `⌘⏎`, `esc`)
 * and omits one this app does bind (`reopen-tab`, `⌘⇧T`). A row this pane
 * cannot get from `hint` is a row it does not draw.
 */
const WINDOW_ACTIONS: ReadonlyArray<{ type: WindowAction["type"]; label: string }> = [
  { type: "console", label: "Open the console" },
  // First after the console, because §25's surface is the one on this list a
  // reader might need in a hurry.
  { type: "lock", label: "Lock the workspace" },
  { type: "new-tab", label: "New tab" },
  { type: "close-tab", label: "Close tab" },
  { type: "reopen-tab", label: "Reopen the last closed tab" },
  { type: "prev-tab", label: "Previous tab" },
  { type: "next-tab", label: "Next tab" },
  { type: "select-tab", label: "Jump to a tab" },
  { type: "zoom-in", label: "Zoom in" },
  { type: "zoom-out", label: "Zoom out" },
  { type: "zoom-reset", label: "Reset zoom" },
];

export function ShortcutsPane() {
  const apple = isApplePlatform();
  return (
    <div className="flex flex-col gap-4">
      <Panel title="Keyboard">
        <div role="list" aria-label="Keyboard shortcuts">
          {WINDOW_ACTIONS.map(({ type, label }) => (
            <div
              key={type}
              data-testid="shortcut-row"
              data-action-type={type}
              role="listitem"
              // A label beside a chord: `min-w-0 flex-1` on the label lets it
              // shrink and wrap instead of pushing the chord off the edge,
              // which is what an un-shrinkable flex child does under
              // `min-width: auto` — the bug that has cost this migration eight
              // defects, and jsdom sees none of them.
              className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1 border-b border-rule py-1.5 last:border-b-0"
            >
              <span className="min-w-0 flex-1 basis-40 text-[0.75rem] text-ink">{label}</span>
              <kbd className="kbd shrink-0" data-testid="shortcut-keys">
                {hint(type, apple)}
              </kbd>
            </div>
          ))}
        </div>
      </Panel>
    </div>
  );
}

/**
 * `Accessibility` — §23's `Motion and focus`, `Contrast` and `Screen readers`
 * sections, cut down to what srelens actually backs.
 *
 * **No `Reduce motion` switch.** §23 draws one; this pane does not, because
 * there is nothing behind it to flip. The stylesheet already has both halves
 * of the mechanism a switch like this would need —
 * `@media (prefers-reduced-motion: reduce)` and a `[data-motion="reduced"]`
 * block that does the same thing for an app-level override
 * (`packages/ui-kit/src/styles/kit.css`) — but nothing in this codebase ever
 * writes `data-motion`, and there is no stored preference for it anywhere:
 * no key in `settingsStorage`, no field in core, no reducer. That is the
 * fifth `data-*` axis this migration has found drawn in the stylesheet with
 * no writer behind it. A switch here would look identical to the real ones on
 * the Appearance pane — same track, same `role="switch"` — while persisting
 * nothing, which is worse than the absence: it tells a reader who flips it
 * that srelens remembered a choice it did not. So this says what actually
 * happens today (the OS setting, and only the OS setting) instead of drawing
 * a control for a preference that does not exist. Reported with this task.
 *
 * **No `Always show focus ring` or `Underline links` switches either.**
 * `[data-focus="always"]` and `[data-underline="on"]` are the same story —
 * real selectors, no writer — and outside what this task was asked to build;
 * §23's `Motion and focus` section is left as one control short of the mock
 * for the same reason as above, not three.
 *
 * **The Contrast and Screen readers paragraphs are static copy**, not reads of
 * anything: the High contrast theme and the ARIA behaviour they describe are
 * both already real and already tested (`AppearancePane.test.tsx`; the tab
 * list and console live region elsewhere in this package), so there is
 * nothing here to wire up beyond a link to where the theme is actually
 * chosen.
 */
export function AccessibilityPane() {
  return (
    <div className="flex flex-col gap-4">
      <Panel title="Motion">
        <p data-testid="reduce-motion-note" className="text-[0.75rem] leading-relaxed text-muted">
          <strong className="text-ink">Reduce motion</strong> stops the live pulse, the flow
          animation on the topology graph, and every transition. srelens keeps no preference of its
          own for this yet, so it only follows your system&apos;s reduce-motion setting — there is no
          switch here to turn it on independently of your OS.
        </p>
      </Panel>

      <Panel title="Contrast">
        <p className="text-[0.75rem] leading-relaxed text-muted">
          The <strong className="text-ink">High contrast</strong> theme raises every text pair above
          7:1, thickens rules to a visible grey, and drops the tinted washes behind badges. Colour
          never carries meaning on its own here: status shows a dot and a word, severity shows a
          label, and the topology graph marks the failing path with a dashed stroke as well as
          colour.
        </p>
        <p className="mt-2 text-[0.75rem] leading-relaxed text-muted">
          Pick it from the Appearance pane, under Theme.
        </p>
      </Panel>

      <Panel title="Screen readers">
        <p className="text-[0.75rem] leading-relaxed text-muted">
          Tabs expose <code className="code">role=&quot;tab&quot;</code> with selected state; the
          console announces agent replies through a live region.
        </p>
        <p className="mt-2 text-[0.75rem] leading-relaxed text-muted">
          Every icon-only control carries a label, switches report{" "}
          <code className="code">aria-checked</code>, and the confirmation gate is a modal dialog
          that traps escape.
        </p>
      </Panel>
    </div>
  );
}

/**
 * `Clusters` — a sentence and a link, per spec decision 3.
 *
 * **`/connections` already is the cluster-source screen** (PR #366): it lists
 * the kubeconfig files srelens reads, names each context's credential kind,
 * and probes reachability. A second panel here that lists contexts, or a
 * control that adds a kubeconfig, would be two places managing the same
 * sources — the exact duplication the Connections spec rejected when it
 * declined to duplicate the cluster rail. §23 itself calls this section a
 * stub: `"Clusters (and any unimplemented panel)"`. This pane draws no table
 * and no add-a-kubeconfig control; it says where the real one lives and sends
 * the reader there.
 */
export function ClustersPane() {
  return (
    <div className="flex flex-col gap-4">
      <Panel title="Clusters">
        <p className="text-[0.75rem] leading-relaxed text-muted">
          Which kubeconfig files srelens reads, what each context&apos;s credential kind is, and
          whether it can be reached all live on Connections — this pane does not keep a second copy
          of that list.
        </p>
        <div className="mt-3">
          <Button variant="secondary" onClick={() => openTab("/connections")}>
            Open Connections
          </Button>
        </div>
      </Panel>
    </div>
  );
}
