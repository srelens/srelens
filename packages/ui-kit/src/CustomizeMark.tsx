import { useId, useRef, useState } from "react";
import { Button } from "./Button";
import { Mark } from "./Mark";
import { cx } from "./cx";
import { Eyebrow } from "./Eyebrow";
import { Field } from "./Field";
import type { IconComponent } from "./IconButton";
import { filled } from "./slot";
import { Switch } from "./Switch";
import { TextInput } from "./TextInput";

/** Longer than the rail's label can show without truncating to nothing useful. */
const MAX_NAME = 28;
/** What {@link Mark} can draw inside its square. */
const MAX_SHORT = 3;
const DEFAULT_MAX_IMAGE_BYTES = 512_000;

/** A native colour input holds six hex digits and nothing else — not a token. */
const SIX_DIGIT = /^#[0-9a-fA-F]{6}$/;

const SIZES = ["sm", "md", "lg"] as const;

/*
 * Inline rather than an icon-set import: the kit takes no dependency on lucide,
 * and these are the only three glyphs it needs. The mock imported ImageUp,
 * RotateCcw and Trash2 from it. (#320)
 */
const glyphProps = { width: 12, height: 12, viewBox: "0 0 24 24", fill: "none", "aria-hidden": true } as const;

const UploadGlyph = () => (
  <svg {...glyphProps}>
    <path d="M12 16V4m0 0L7 9m5-5 5 5" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    <path d="M4 17v2a1 1 0 0 0 1 1h14a1 1 0 0 0 1-1v-2" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
  </svg>
);

const TrashGlyph = () => (
  <svg {...glyphProps}>
    <path d="M4 7h16M9 7V5h6v2m-8 0 1 13h8l1-13" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

const ResetGlyph = () => (
  <svg {...glyphProps}>
    <path d="M3 5v6h6" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
    <path d="M4 13a8 8 0 1 0 2-6l-3 4" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
  </svg>
);

/** How a mark looks. Everything {@link Mark} needs, and nothing else. */
export interface MarkAppearance {
  name: string;
  /** Up to three characters. Drawn as the mark, or ridden under a glyph or image. */
  short: string;
  color: string;
  mark: "text" | "icon" | "image";
  /** Which glyph, by id, out of the ones the caller offers. */
  icon?: string;
  /** The image itself, usually a data URL. */
  imageSrc?: string;
  /** Ride the short text under a glyph or image mark. */
  withText: boolean;
}

export interface CustomizeMarkProps {
  value: MarkAppearance;
  /** Every edit, whole. The editor keeps none of this itself. */
  onChange: (next: MarkAppearance) => void;
  /** The palette offered as swatches, each named: a hex read aloud names nothing. */
  colors?: Array<{ value: string; label: string }>;
  /** The glyphs offered. The kit ships none — the catalogue is the app's. */
  icons?: Array<{ id: string; label: string; icon: IconComponent }>;
  /** Offered only when given: puts the appearance back to whatever the app defaults to. */
  onReset?: () => void;
  /** How large an image the app is willing to keep. */
  maxImageBytes?: number;
  className?: string;
}

const kb = (bytes: number) => `${Math.round(bytes / 1024)} KB`;

/**
 * The editor for how one mark looks: its name, its short text, its colour, and
 * whether it draws as initials, a glyph or an image — previewed at every size
 * the rail uses while it is edited.
 *
 * The mock's version was welded to the app three ways over. It took a cluster
 * and read that cluster's override out of a module-level store, so the editor
 * could only ever edit the thing the store knew about; it wrote every keystroke
 * straight back into the store and into localStorage, so there was no way to
 * show an edit without committing it; and it drew its symbols from a map of
 * sixteen lucide icons keyed by preset id. None of that is presentation. What
 * is left is a controlled editor over the shape {@link Mark} draws —
 * `value` in, `onChange` out — with the palette and the symbol catalogue passed
 * in, the same move {@link MultiSelect} made when its one particular kind of
 * thing became `allLabel`. The dialog is gone too: this is the body, and
 * whoever opens it owns the frame and the way out of it. (#320)
 *
 * Every button here says `type="button"`. The kit's {@link Button} deliberately
 * leaves `type` alone, so inside a form a bare button submits it — and the mock
 * had six of them, including one that removed an image and one that reset the
 * whole thing.
 *
 * The swatches and the symbols are native radios rather than the mock's grids
 * of `aria-pressed` buttons, for the reason {@link Radio} gives: a shared `name`
 * buys one tab stop, arrow keys between the options and wrap-around at the ends,
 * correct in every browser, for the price of an attribute. Thirty-two buttons
 * would otherwise be thirty-two tab stops between the name field and the reset.
 * Their names come from the caller as words: the mock labelled a swatch
 * "Colour #b4342a" and a symbol "harddrive", which are a string of digits and an
 * unspaced identifier read out one letter at a time.
 *
 * Picking an image can fail in four ways and the mock covered two of them, both
 * silently to a screen reader. The wrong kind of file and one too large to keep
 * are reported as before but in a live region; a file that cannot be read at
 * all is reported at all, where the mock wired `onload` and left `onerror`
 * unhandled; and choosing the same file twice in a row now works, because the
 * input is cleared after each attempt — without that, a rejected file stays
 * selected and re-choosing it fires no change event at all.
 */
export function CustomizeMark({
  value,
  onChange,
  colors = [],
  icons = [],
  onReset,
  maxImageBytes = DEFAULT_MAX_IMAGE_BYTES,
  className,
}: CustomizeMarkProps) {
  const id = useId();
  const fileRef = useRef<HTMLInputElement>(null);
  const [error, setError] = useState<string | null>(null);

  const set = (patch: Partial<MarkAppearance>) => onChange({ ...value, ...patch });

  // The name is what the rail shows and what the mark falls back to naming
  // itself; the mock let it be emptied and said nothing.
  const nameError = value.name.trim() === "" ? "A display name is required." : undefined;

  const chosenGlyph = icons.find((option) => option.id === value.icon)?.icon;
  const previewImage = value.mark === "image" ? value.imageSrc : undefined;
  const previewIcon = value.mark === "icon" ? chosenGlyph : undefined;

  // A symbol mark with no symbols to choose from is a choice leading to an
  // empty grid, so it is not offered. The kit ships no icon set of its own.
  const marks: Array<{ id: MarkAppearance["mark"]; label: string }> = [
    { id: "text", label: "Text" },
    ...(icons.length > 0 ? [{ id: "icon" as const, label: "Symbol" }] : []),
    { id: "image", label: "Image" },
  ];

  const pickImage = (file: File | undefined, input: HTMLInputElement) => {
    // Cleared whatever happens: a rejected file stays selected otherwise, and
    // choosing it again fires no change event, so the second attempt does
    // nothing at all and looks like the button is broken.
    input.value = "";
    if (!file) return;
    if (!file.type.startsWith("image/")) {
      setError("That file is not an image.");
      return;
    }
    if (file.size > maxImageBytes) {
      setError(`Images must be under ${kb(maxImageBytes)} — this one is ${kb(file.size)}.`);
      return;
    }
    const reader = new FileReader();
    reader.onerror = () => setError("That image could not be read.");
    reader.onload = () => {
      setError(null);
      set({ mark: "image", imageSrc: String(reader.result) });
    };
    reader.readAsDataURL(file);
  };

  return (
    <div className={cx("min-w-0", className)}>
      {/* The preview rides along the top rather than owning a tall empty column. */}
      <div
        data-slot="preview"
        className="rule-b mb-2 flex items-center gap-3 px-3 py-2"
        style={{ background: "var(--surface-sunk)" }}
      >
        <div className="flex items-end gap-2">
          {SIZES.map((size) => (
            <Mark
              key={size}
              // Decorative: the name is written out beside them, and three
              // chips announcing the same name is that name three times.
              decorative
              size={size}
              active={size === "md"}
              name={value.name}
              short={value.short}
              color={value.color}
              icon={previewIcon}
              imageSrc={previewImage}
              withBadge={value.withText}
            />
          ))}
        </div>
        <div className="min-w-0">
          <div className="truncate text-[0.8125rem] font-semibold">{value.name}</div>
          {/* Which three: the sizes a rail draws a mark at. Unqualified, three
              squares beside each other read as three marks rather than as one
              mark at every size it is ever seen. */}
          <Eyebrow className="mt-px">preview at all three rail sizes</Eyebrow>
        </div>
      </div>

      <div className="grid grid-cols-[1fr_92px] gap-2 px-3">
        <Field label="Display name" error={nameError}>
          <TextInput
            value={value.name}
            invalid={nameError !== undefined}
            // Capped here rather than through `maxLength`, which a paste walks
            // straight past.
            onValueChange={(next) => set({ name: next.slice(0, MAX_NAME) })}
          />
        </Field>
        <Field label="Short text" hint={`Max ${MAX_SHORT}.`}>
          <TextInput
            value={value.short}
            onValueChange={(next) => set({ short: next.replace(/\s/g, "").slice(0, MAX_SHORT) })}
          />
        </Field>
      </div>

      <div className="flex items-center gap-2 px-3 py-1.5">
        <Eyebrow className="w-14 shrink-0">Colour</Eyebrow>
        <div className="flex flex-wrap items-center gap-1">
          {colors.length > 0 && (
            <div
              data-slot="swatches"
              role="radiogroup"
              aria-label="Colour"
              className="flex flex-wrap items-center gap-1"
            >
              {colors.map((swatch) => {
                // Compared in one case: the value can come back from storage or
                // from the custom picker in whichever case wrote it, and a
                // swatch that fails to match leaves the group with nothing
                // checked and no tab stop at all.
                const on = swatch.value.toLowerCase() === value.color.toLowerCase();
                return (
                  <input
                    key={swatch.value}
                    type="radio"
                    name={`${id}-color`}
                    aria-label={swatch.label}
                    checked={on}
                    onChange={() => set({ color: swatch.value })}
                    // The radio is the swatch: `appearance-none` leaves the
                    // element itself to be coloured, which keeps the native
                    // focus ring, the arrow keys and the announced role rather
                    // than rebuilding them around a hidden input.
                    className="h-[18px] w-[18px] cursor-pointer appearance-none rounded-[5px]"
                    style={{
                      background: swatch.value,
                      boxShadow: on ? "0 0 0 2px var(--surface), 0 0 0 3.5px var(--ink)" : undefined,
                    }}
                  />
                );
              })}
            </div>
          )}
          {/*
            The well is painted by CSS from the colour itself, with the native
            input laid over it at zero opacity.

            `<input type="color">` holds six hex digits and nothing else, and
            there is no way to put it in an "unset" state: HTML's value
            sanitization replaces any invalid value — the empty string included
            — with #000000. This asked for `""` and a comment said it would then
            show "its own default", which does not exist. A token is the primary
            case, not an edge: ui-next passes `var(--mark-*)` for all eleven
            swatches, so a reader who picked Green saw a green mark, a green
            swatch, the text `var(--mark-green)`, and a black well beside them.

            Painting the well instead of reading it hands the resolving to CSS,
            which already does it — a token, a named colour, a `color-mix()` —
            and keeps it resolved: the `--mark-*` tokens differ between light
            and dark, so a hex resolved once through `getComputedStyle` would go
            stale the moment the theme changed. The well cannot disagree with
            the mark this way, and the input keeps its role, its name, the OS
            picker and (through the wrapper) its focus ring. (#380 review)
          */}
          <span
            data-slot="custom-colour"
            className="relative ml-1 block h-[18px] w-6 shrink-0 rounded border has-[:focus-visible]:[outline:2px_solid_var(--accent)] has-[:focus-visible]:[outline-offset:1px]"
            style={{ background: value.color }}
          >
            <input
              type="color"
              aria-label="Custom colour"
              // Handed the colour only when it is one the control can hold, so
              // the picker opens on the current colour rather than on black.
              // Anything else stays empty, which the DOM then sanitizes to
              // black — that is the whole point of covering it: the value here
              // is only ever what the picker OPENS on, never what is shown.
              // (Written as empty rather than as the black it becomes because
              // the kit names no colour of its own; see `tokens-only.test.ts`.)
              value={SIX_DIGIT.test(value.color) ? value.color : ""}
              onChange={(event) => set({ color: event.target.value })}
              className="absolute inset-0 h-full w-full cursor-pointer appearance-none border-0 bg-transparent p-0 opacity-0"
            />
          </span>
          <span className="code text-[0.625rem] text-faint">{value.color}</span>
        </div>
      </div>

      <div className="flex items-center gap-2 px-3 py-1.5">
        <Eyebrow className="w-14 shrink-0">Mark</Eyebrow>
        <div className="seg" role="radiogroup" aria-label="Mark">
          {marks.map((option) => (
            <label
              key={option.id}
              className="seg-btn cursor-pointer has-[:focus-visible]:[outline:2px_solid_var(--accent)]"
              data-on={value.mark === option.id}
            >
              {/* Visually hidden rather than absent: the label is the segment,
                  and the input underneath is what the browser gives the arrow
                  keys and the announced state to. */}
              <input
                type="radio"
                name={`${id}-mark`}
                className="sr-only"
                checked={value.mark === option.id}
                onChange={() => set({ mark: option.id })}
              />
              {option.label}
            </label>
          ))}
        </div>
        {value.mark !== "text" && (
          <Switch
            on={value.withText}
            onChange={(on) => set({ withText: on })}
            label="Text on mark"
            className="ml-auto !w-auto"
          />
        )}
      </div>

      {value.mark === "icon" && icons.length > 0 && (
        <div
          data-slot="glyphs"
          role="radiogroup"
          aria-label="Symbol"
          className="flex flex-wrap gap-1 px-3 pb-1.5 pl-[4.5rem]"
        >
          {icons.map((option) => {
            const Glyph = option.icon;
            const on = option.id === value.icon;
            return (
              <label
                key={option.id}
                className="flex h-6 w-6 cursor-pointer items-center justify-center rounded-md border has-[:focus-visible]:[outline:2px_solid_var(--accent)]"
                style={{
                  borderColor: on ? value.color : "var(--rule)",
                  background: on ? "var(--accent-wash)" : "transparent",
                  color: on ? value.color : "var(--ink-muted)",
                }}
              >
                <input
                  type="radio"
                  name={`${id}-glyph`}
                  className="sr-only"
                  aria-label={option.label}
                  checked={on}
                  onChange={() => set({ icon: option.id })}
                />
                <Glyph size={13} aria-hidden="true" />
              </label>
            );
          })}
        </div>
      )}

      {value.mark === "image" && (
        <div className="px-3 pb-1.5 pl-[4.5rem]">
          <div className="flex flex-wrap items-center gap-1.5">
            <Button type="button" variant="secondary" size="sm" onClick={() => fileRef.current?.click()}>
              <UploadGlyph /> Choose image
            </Button>
            {/* `filled` rather than a null check: an image cleared elsewhere
                comes back as "" as often as it does undefined. */}
            {filled(value.imageSrc) && (
              <Button
                type="button"
                variant="danger"
                size="sm"
                // Back to the text mark, not just a cleared source: staying on
                // the image mark with nothing to show leaves an empty square.
                onClick={() => set({ mark: "text", imageSrc: undefined })}
              >
                <TrashGlyph /> Remove image
              </Button>
            )}
            <span className="text-[0.75rem] text-muted">Square, under {kb(maxImageBytes)}.</span>
            {/* Hidden, and driven by the button above, because the native
                control's own label cannot be styled. `display: none` is not
                focusable, so it is not a phantom tab stop either. */}
            <input
              ref={fileRef}
              type="file"
              accept="image/png,image/jpeg,image/svg+xml,image/webp"
              className="hidden"
              onChange={(event) => pickImage(event.target.files?.[0], event.target)}
            />
          </div>
          {error !== null && (
            // A live region: the mock's failure was a line of red text, which a
            // screen reader user never hears — and they are the ones who cannot
            // see that nothing happened.
            <p role="alert" className="mt-1 text-[0.75rem]" style={{ color: "var(--sev)" }}>
              {error}
            </p>
          )}
        </div>
      )}

      {onReset && (
        <div className="rule-t mt-1 flex justify-end px-3 py-2">
          <Button type="button" variant="secondary" size="sm" onClick={onReset}>
            <ResetGlyph /> Reset
          </Button>
        </div>
      )}
    </div>
  );
}
