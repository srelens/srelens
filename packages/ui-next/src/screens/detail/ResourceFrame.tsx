import { useId, type ReactNode } from "react";
import { Breadcrumb, Button, KV, Stat, Tabs, type StatProps, type TabItem } from "@srelens/ui-kit";
import { Icons } from "../../lib/icons";
import { CopyNameButton } from "./CopyNameButton";
import { Section } from "./Section";
import type { DetailFact } from "./facts";

/**
 * The detail frame, on its own: what a resource's peek and full tab look like
 * with no opinion on where the resource came from.
 *
 * A module of its own because two families of detail draw it. The built-in
 * views read a `K8sObject` through `detailData`; an app's reads through the
 * host's `extensions.resource`. Both views import the app slots, and the app
 * detail is exported beside them, so this file imports nothing from
 * `extensions/` — that is what keeps the app view from a cycle. (#1001)
 */

/** One tile of the strip. Absent facts are omitted, never drawn as em dashes. */
export type Tile = StatProps & { key: string };

/**
 * The design's other header affordance: promote this subject to a tab of its
 * own. Outlined and labelled, beside the close rather than instead of it —
 * the mock draws two separate controls, and a peek that could only be left by
 * closing it is a peek the reader has to re-find.
 */
export function OpenTabButton({ onClick, disabled }: { onClick: () => void; disabled?: boolean }) {
  const Glyph = Icons.openTab;
  return (
    <Button type="button" variant="outline" size="xs" disabled={disabled} onClick={onClick}>
      {/* The word is the accessible name; the glyph only decorates it. */}
      <Glyph size={12} aria-hidden="true" />
      Open tab
    </Button>
  );
}

/**
 * THE PEEK'S OWN FACT LAYOUT: a label column and a value column, read down.
 *
 * The mock's two-column list, in a pane about 352px wide — the label in muted
 * ink, the value beside it, one fact per line. The full tab reads the very
 * same facts across three columns of label-above-value and builds that itself;
 * this screen has never seen that layout and cannot be changed by it. What the
 * two share is the LIST (`detailFacts`), not a line of its markup. (#331)
 *
 * Untitled, because the design heads the first block of a detail with nothing:
 * the header above has already given the name, the kind and the namespace. An
 * untitled block also cannot fold, which is what keeps a detail from opening
 * showing nothing at all.
 *
 * A subject with no facts draws no block: an empty section still has its
 * padding and still rules against whatever follows it.
 */
export function PeekFacts({ facts }: { facts: DetailFact[] }) {
  if (facts.length === 0) return null;
  return (
    // Named, so a test can hold this screen to drawing the WHOLE derived list
    // rather than a fact or two off it — a screen that quietly dropped three
    // of them would otherwise look exactly right. A class rather than a
    // wrapper: the block has to stay a direct sibling of what follows it, or
    // `.section + .section` draws no hairline against it.
    <Section className="fact-list">
      {facts.map((fact) => (
        <KV key={fact.label} k={fact.label} v={fact.value} mono={fact.mono} />
      ))}
    </Section>
  );
}

/**
 * THE FULL TAB'S OWN FACT LAYOUT: three columns of label-above-value, each
 * pair ruled off beneath, filling the width of a page.
 *
 * The mock's grid, built here, by the screen that draws it. The peek reads the
 * very same facts down one column of label-beside-value rows and builds that
 * itself. What the two share is the LIST (`detailFacts`), so they cannot
 * disagree about what a subject says; what neither shares is a line of the
 * other's markup, so a change to this grid changes nothing in the peek.
 *
 * It replaces `FactGrid` — a kit wrapper that took the peek's rendered rows
 * and restyled them into these columns. That wrapper had to describe a layout
 * in terms of children it did not build (`.factgrid .section > :not(.kv)`,
 * then a second rule for tables, and a reviewer's note that a third kind of
 * child was coming), and it made the tab's layout a property of the peek's
 * DOM. `KV`'s own `stacked` form is what a row looks like here; the grid
 * around them is this screen's. (#331)
 *
 * Untitled, like the peek's list and for the same reason: the header above has
 * already named the subject, and an untitled block cannot fold, so Overview
 * can never open showing nothing.
 */
export function TabFacts({ facts }: { facts: DetailFact[] }) {
  if (facts.length === 0) return null;
  return (
    <Section>
      <div data-slot="fact-grid" className="grid grid-cols-3 gap-x-6">
        {facts.map((fact) => (
          <KV key={fact.label} stacked k={fact.label} v={fact.value} mono={fact.mono} />
        ))}
      </div>
    </Section>
  );
}

/**
 * Labels and Annotations side by side, as the full tab closes a subject.
 *
 * Two columns, and each wrapped, so neither section is the other's adjacent
 * sibling — `.section + .section` would otherwise rule down the middle of the
 * row instead of across it. The same wrapper hides the row from whatever block
 * comes after it, so the row draws that rule itself.
 */
export function TabMetadata({ labels, annotations }: { labels: ReactNode; annotations: ReactNode }) {
  return (
    <div data-slot="metadata-pair" className="border-t border-rule-strong grid grid-cols-2 [&+.section]:border-t [&+.section]:border-rule-strong">
      <div>{labels}</div>
      <div className="border-l border-rule-strong">{annotations}</div>
    </div>
  );
}

export interface ResourceTabFrameProps {
  name: string;
  /** Where the trail starts: the cluster, in the words the reader knows it by. */
  cluster: string;
  namespace: string | null;
  kind: string;
  /** The subject's action row, on the header's line. None until it has loaded. */
  actions?: ReactNode;
  tabs: TabItem[];
  active: string;
  onTabChange: (id: string) => void;
  tiles: Tile[];
  /** The active pane. */
  children: ReactNode;
}

/**
 * The resource FULL TAB's chrome: one subject filling a tab of its own, headed
 * by its name and the trail that locates it, its actions on the same line, its
 * tabs, a strip of figures beneath, then the pane the host hands in.
 */
export function ResourceTabFrame({
  name,
  cluster,
  namespace,
  kind,
  actions,
  tabs,
  active,
  onTabChange,
  tiles,
  children,
}: ResourceTabFrameProps) {
  const headingId = useId();
  const activeLabel = tabs.find((t) => t.id === active)?.label;

  return (
    <section
      aria-labelledby={headingId}
      data-slot="resource-tab"
      className="flex min-h-0 min-w-0 flex-1 flex-col"
    >
      <header className="border-b border-rule-strong flex items-center gap-3 px-3 py-2">
        <div className="name-row flex min-w-0 items-baseline gap-2.5">
          {/* An `h1`: this is a page, and the tab strip above it is chrome.
              The peek's subject is an `h2` because a peek sits inside a
              screen that already has one. */}
          <h1 id={headingId} className="truncate text-[1.25rem] font-semibold">
            {name}
          </h1>
          {/* The peek's own control, in the same place: directly after the
              name, shown when this row is hovered or focused (`.name-row`). */}
          <span data-slot="tab-name-action" className="name-action self-center">
            <CopyNameButton name={name} />
          </span>
          {/* Cluster, namespace, kind — the design's small-caps trail. A
              cluster-scoped subject has no namespace to place, so the trail is
              two steps rather than a step reading "—". */}
          <Breadcrumb
            className="shrink-0"
            parts={namespace ? [cluster, namespace, kind] : [cluster, kind]}
          />
        </div>
        <span className="flex-1" />
        <div data-slot="tab-actions" className="flex shrink-0 items-center gap-1.5">
          {actions}
        </div>
      </header>

      <div className="border-b border-rule-strong px-3">
        <Tabs
          tabs={tabs}
          active={active}
          onChange={onTabChange}
          label="Resource views"
          variant="underline"
        />
      </div>

      {/* Between the strip and the panel rather than inside it: these are the
          subject's vital signs, as worth having beside its manifest as beside
          its facts. `.stat + .stat` draws the vertical rules the design puts
          between the tiles; the row sizes them, which is what `Stat`'s own
          comment asks a caller to do. */}
      {tiles.length > 0 && (
        <div data-slot="metric-strip" className="border-b border-rule-strong flex">
          {tiles.map(({ key, ...tile }) => (
            <Stat key={key} {...tile} className="flex-1" />
          ))}
        </div>
      )}

      <div
        className="pane-body"
        role="tabpanel"
        // Named rather than pointed at: `Tabs` puts no ids on its buttons, so
        // `aria-labelledby` would have nothing to reference.
        aria-label={activeLabel}
        // A scrolling region has to be reachable, or its content is unreadable
        // to anyone driving the page from the keyboard.
        tabIndex={0}
      >
        {children}
      </div>
    </section>
  );
}
