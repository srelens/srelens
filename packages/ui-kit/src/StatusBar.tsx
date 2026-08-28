import type { ReactNode } from "react";
import { cx } from "./cx";
import { filled } from "./slot";
import { Spinner } from "./Spinner";
import { toneColor, type Tone } from "./tone";
import type { IconComponent } from "./IconButton";

export interface StatusSegment {
  /** Identifies the readout in the list; must be unique within the strip. */
  id: string;
  /**
   * What the readout says, in words.
   *
   * A string rather than a node, and required rather than optional, because it
   * is doing two jobs at once: it is the readout, and it is the accessible name
   * of the button when there is one. Neither an icon-only segment nor a segment
   * that speaks only in colour can be expressed here, which is the point.
   */
  label: string;
  /** A quieter suffix — a count of retries, a qualifier. Never the meaning. */
  detail?: ReactNode;
  /** Tints the readout. Semantic, so it follows the theme; see {@link Tone}. */
  tone?: Tone;
  /** A filled dot before the label, in the segment's tone. */
  dot?: boolean;
  /** Animates the dot, for a readout that is live rather than merely current. */
  pulse?: boolean;
  /** A glyph before the label. Decorative; the label still says everything. */
  icon?: IconComponent;
  /** Something is running behind this readout — indexing, reconnecting. */
  busy?: boolean;
  /** Hover text, for the segment whose label is too short to explain itself. */
  title?: string;
  /** Makes the segment pressable. Left off, it is a readout and nothing more. */
  onSelect?: () => void;
}

export interface StatusBarProps {
  /** Readouts from the leading edge. */
  segments: StatusSegment[];
  /** Readouts pinned to the trailing edge, pushed there by a growing gap. */
  end?: StatusSegment[];
  /** Names the strip for assistive technology. */
  label?: string;
}

/**
 * The strip along the bottom of the window: a row of small readouts — which
 * cluster, how many port-forwards, whether the watch is live — several of which
 * are also the way in to what they report on.
 *
 * The mock's version read six stores directly and hard-coded what it could not
 * read. None of that survives the move: the kit may not know what a cluster or
 * a port-forward is, so every readout arrives as a segment and this component
 * is left owning the three things that are actually about the strip — what a
 * segment looks like, which of them can be pressed, and how a segment says what
 * it means.
 *
 * That last one is the reason the port is not a copy. The mock said several
 * things in colour and nothing else: a `style={{ color: "var(--warn)" }}` on
 * the segment, a dot filled straight from a value in the data. A colour-blind
 * user sees an ordinary readout; a screen reader sees one too. So colour here
 * is a {@link Tone} over a required text `label`, the dot is hidden from
 * assistive technology as the second channel it is, and a segment that is busy
 * says so with `aria-busy` rather than only by spinning. The dot also loses the
 * mock's per-cluster colour: an arbitrary value out of the data cannot be
 * themed, and a design system that accepts one has no palette.
 *
 * A named group rather than `role="status"`: this strip is not an
 * announcement. A live region here would read out every count, every reconnect
 * and every indexing tick as it happened. (#320)
 */
export function StatusBar({ segments, end, label = "Status" }: StatusBarProps) {
  return (
    <div className="statusbar" role="group" aria-label={label}>
      {segments.map((segment) => (
        <Segment key={segment.id} segment={segment} />
      ))}
      {/* A length check rather than `filled`, which the kit's optional slots
          use: this is a list of data, not a node, and an empty one is the only
          way it can be empty. Absent a trailing group there is no gap either —
          with nothing on the right, the readouts simply run from the left as
          they already do, and a spacer would only be a stretched hole. */}
      {end !== undefined && end.length > 0 && <span className="flex-1" aria-hidden="true" />}
      {end?.map((segment) => (
        <Segment key={segment.id} segment={segment} />
      ))}
    </div>
  );
}

/** One readout: a button when it leads somewhere, plain text when it does not. */
function Segment({ segment }: { segment: StatusSegment }) {
  const { label, detail, tone, dot, pulse, icon: Icon, busy, title, onSelect } = segment;

  const body = (
    <>
      {dot && (
        <span
          data-dot
          // The dot restates the tone, and the tone restates the label. Hidden,
          // because a screen reader reading it would get a bullet and no news.
          aria-hidden="true"
          className={cx("h-1.5 w-1.5 shrink-0 rounded-full", pulse && "live-dot")}
          // Neutral rather than nothing when no tone was given: an invisible
          // dot still takes its space and its gap, so the row would look
          // wrongly padded rather than un-dotted.
          style={{ background: toneColor(tone ?? "muted") }}
        />
      )}
      {busy ? (
        // The kit's Spinner names itself, and a second name on a readout that
        // already has one reads as "Loading indexing 1 284". The word is
        // `aria-busy` on the segment; this is only the picture of it.
        <Spinner aria-hidden="true" className="size-3 shrink-0" />
      ) : (
        Icon && (
          // Hidden by the slot rather than by asking the icon to hide itself:
          // the icon is the caller's component, and one that drops the
          // `aria-hidden` it is handed would put a nameless graphic inside the
          // button's name. (#320)
          <span aria-hidden="true" className="flex shrink-0 items-center">
            <Icon size={11} />
          </span>
        )
      )}
      {label}
      {filled(detail) && <span className="opacity-60">{detail}</span>}
    </>
  );

  // Tinting the whole segment rather than the label alone, the way the mock
  // did: the dot, the icon and the detail all belong to the same readout, and
  // `currentColor` is what the inlined glyphs draw themselves in.
  const style = tone ? { color: toneColor(tone) } : undefined;

  if (onSelect) {
    return (
      // Explicitly type="button". These stand in the window's chrome, which can
      // sit inside a form, and a button without a type submits it — a status
      // strip is the last place anyone would look for a lost form.
      <button
        type="button"
        className="status-seg"
        style={style}
        title={title}
        aria-busy={busy || undefined}
        onClick={onSelect}
      >
        {body}
      </button>
    );
  }

  return (
    <span className="status-seg" style={style} title={title} aria-busy={busy || undefined}>
      {body}
    </span>
  );
}
