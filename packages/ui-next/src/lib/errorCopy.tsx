import type { ReactNode } from "react";
import { describeError } from "@srelens/core";
import { Alert, ErrorState, RawError, type Tone } from "@srelens/ui-kit";

/**
 * The new design's error vocabulary: one way to turn a backend refusal into
 * something a person can act on, and one place that decides what happens to
 * the string the cluster actually sent.
 *
 * **Why this exists at all.** `describeError` has classified cluster failures
 * since classic — exec-auth, timeouts, DNS, 401, 403, TLS — and it is
 * platform-aware, because the remedy for an exec plugin that cannot run is a
 * different sentence on the desktop than in the web container. Every one of
 * its call sites was in `apps/desktop`. This package had none, so every screen
 * here printed whatever Rust said: the cluster overview's Fleet rail was
 * showing `ApiError: Unauthorized (Status { metadata: Some(ListMeta { … })` at
 * the reader, in a column 286px wide.
 *
 * **The contextual title stays; the DETAIL is what `describeError` replaces.**
 * This is the one deliberate departure from classic's call sites, which pass
 * `title={describeError(e).title}`. Classic's error cards had no title of
 * their own to lose. This package's do — "Could not list pods on prod-eu",
 * "Could not load secret/db-creds's manifest" — and that half of the message
 * is the half `describeError` can never know: it is handed a string, not a
 * screen. "Not authorized" is also not a fact the detail withholds; the first
 * sentence under it says "The cluster rejected your credentials" in words.
 * Dropping the context to gain the classification would trade a fact for a
 * paraphrase.
 *
 * It also keeps a copy this package writes ITSELF from being overwritten.
 * `redactSecretManifest` fails closed with hand-written sentences — "This
 * Secret's manifest is not shown, because it could not be redacted: it uses
 * YAML aliases" — which are deliberately not cluster errors and match none of
 * `describeError`'s branches. They come back through the generic case, so a
 * screen passing `friendly.title` would title the Secret pane's most careful
 * refusal "Something went wrong". With the contextual title kept, that message
 * arrives exactly as written.
 *
 * **Nothing is dropped.** `describeError` keeps the original in `raw` for
 * precisely this, and every surface below offers it — folded away behind a
 * word, never in a `title` attribute. See {@link RawError} for why that
 * distinction is not a detail.
 */

/** A classified failure, plus the one rule about when its original is worth offering. */
export interface FriendlyCopy {
  title: string;
  detail: string;
  /**
   * The original message — `undefined` when it is the same string as `detail`,
   * which is what the generic case returns. Showing it twice reads as two
   * problems, and a disclosure that opens onto the line above it is a click
   * spent learning nothing.
   */
  raw: string | undefined;
}

/** {@link describeError}, with the "is the original worth offering" rule applied. */
export function friendly(error: unknown): FriendlyCopy {
  const { title, detail, raw } = describeError(error);
  return { title, detail, raw: raw === detail ? undefined : raw };
}

/**
 * Several failures at once, as one piece of copy.
 *
 * The overview's fan-out sections are the shape this is for: `Not ready`
 * checks six kinds and any subset of them can refuse, and `Stale` collects a
 * reason from every loader that stopped refreshing. Deduplicated, because one
 * expired token refuses all six and "your credentials expired" said six times
 * is not six problems.
 */
export function summarise(errors: string[]): { detail: string; raw: string | undefined } {
  const copies = errors.filter((e) => e !== "").map(friendly);
  const details = [...new Set(copies.map((c) => c.detail))];
  // The originals are kept apart by a blank line rather than the separator the
  // sentences use: each one is a struct that already contains punctuation, and
  // running two together makes a third thing that is neither.
  const raws = [...new Set(copies.map((c) => c.raw).filter((r): r is string => r !== undefined))];
  const detail = details.join(" · ");
  const raw = raws.join("\n\n");
  return { detail, raw: raw === "" || raw === detail ? undefined : raw };
}

export interface FailureStateProps {
  /**
   * What failed, in this screen's own words. Left out, the classification's
   * own headline is used — which is what a surface with no context to add
   * should do, and what classic's call sites all do.
   */
  title?: ReactNode;
  error: unknown;
  onRetry?: () => void;
  retryLabel?: string;
  action?: { label: string; onClick: () => void };
  className?: string;
}

/** A content area whose load failed, said in words the reader can act on. */
export function FailureState({ title, error, ...rest }: FailureStateProps) {
  const copy = friendly(error);
  return <ErrorState title={title ?? copy.title} detail={copy.detail} raw={copy.raw} {...rest} />;
}

export interface FailureAlertProps {
  /**
   * The banner's own headline — "These pods are stale", "Namespaces could not
   * be listed". Required, and never taken from the classification: these
   * banners sit over rows that are still on screen, and the sentence that
   * makes them worth reading is about the ROWS, not about the failure.
   */
  title: ReactNode;
  error: unknown;
  /** `warn` by default: a banner over surviving rows is a warning, not a stop. */
  tone?: Tone;
  className?: string;
}

/**
 * A warning over content that is still there — a stale list, a kind that could
 * not be checked, a namespace picker that may be short.
 *
 * Deliberately NOT an error state: the rows above and below it are real, and
 * replacing them with a card would throw away the only information the reader
 * has. The title is the caller's for the same reason — those sentences were
 * written on purpose and this only changes what sits under them.
 */
export function FailureAlert({ title, error, tone = "warn", className }: FailureAlertProps) {
  const copy = friendly(error);
  return (
    <Alert tone={tone} title={title} className={className}>
      {copy.detail}
      <RawError text={copy.raw ?? ""} className="mt-1" />
    </Alert>
  );
}

/**
 * A failure with room for a word and nothing more — the overview's Fleet rows
 * (286px), a per-kind count that could not be taken, a cluster in the rail.
 *
 * The headline alone: `Not authorized`, `Can't reach the cluster`. A paragraph
 * here would push the nine clusters that answered off the bottom of the rail,
 * which is exactly what the struct was doing. The original is still one click
 * away, and closed it costs a single quiet word.
 */
/**
 * A failure in a place that has room for the sentence but no room for chrome —
 * the line under a confirm dialog's kubectl preview, where the dialog itself
 * has already said which action was refused.
 *
 * The detail rather than the headline, because this surface is wide: "Not
 * authorized" under a Drain confirmation says less than "The cluster rejected
 * your credentials — refresh your kubeconfig credentials and try again", and
 * there is space for the second.
 */
export function FailureLine({ error, className }: { error: unknown; className?: string }) {
  const copy = friendly(error);
  return (
    <div className={className}>
      {copy.detail}
      <RawError text={copy.raw ?? ""} className="mt-0.5" />
    </div>
  );
}

export function FailureWord({
  error,
  lead,
  className,
}: {
  error: unknown;
  /** What the row is about, when the word alone would not say — "Could not count Pod: ". */
  lead?: ReactNode;
  className?: string;
}) {
  // `describeError` rather than `friendly`, and the difference matters here.
  // `friendly` withholds the original when it is the same string as the
  // DETAIL, which is what the generic case returns — the right rule for every
  // surface that prints the detail. This one prints the TITLE, so for an
  // unclassified failure the title is "Something went wrong" and the message
  // is only in `raw`: applying that rule here would leave a row saying
  // "Something went wrong" and nothing else, which is less than the row said
  // before any of this. The original is always offered.
  const { title, raw } = describeError(error);
  // A `div` rather than a `span`: `details` is flow content, and a disclosure
  // inside phrasing markup is a shape the parser rewrites underneath you.
  return (
    <div className={className}>
      {lead}
      {title}
      <RawError text={raw} className="mt-0.5" />
    </div>
  );
}
