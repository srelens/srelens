# Native component vocabulary, version 1

The host renders ten data-only components using `@srelens/ui-kit`. The contract
is [native-component.v1.json](../../schemas/native-component.v1.json); each
component has a named definition there. The vocabulary version is independent
of the extension manifest API (currently `0.3`). Unknown versions, component
names, fields, and invalid data produce a visible error; validation never
coerces or silently truncates input.

This is a rendering foundation, not a new manifest contribution point.
Existing extension contributions still name their authorized reader bindings.
The resource detail view projects its conditions into this vocabulary today.
Future resource panels and overview cards can use the same boundary after the
host has resolved and authorized their data bindings. A component payload
cannot fetch data, read Secrets, select a cluster, invoke capabilities, or grant
permissions. Authorization and redaction must happen before constructing it.

## Calling the host renderer

```tsx
import { NativeComponent } from "@srelens/ui-next/native-components";

<NativeComponent
  label="Conditions"
  payload={{
    version: 1,
    type: "Conditions",
    data: { items: [{ type: "Ready", status: "True", reason: "Available" }] },
  }}
  onRetry={reloadAuthorizedData}
/>
```

`payload` accepts `unknown` and is validated at render time with
`validateNativeComponent` from `@srelens/core`. The exported
`NativeComponentPayload` discriminated union describes valid host projections.
Hosts own the region label, retry handler, allocation of space, and request
lifecycle. Supply `state={{status:"loading"}}`,
`state={{status:"error",error}}`, or `state={{status:"empty"}}` while no ready
payload is available. Omit `state` for ready data. An error retains its cause
through `describeError`; successful empty collections and text say “No data
reported.” A null metric means unknown; numeric zero is a value.

The host can request `fill` for a Code component in a bounded pane. Extensions
cannot provide React nodes, markup, CSS, classes, event handlers, layout trees,
editor options, or JavaScript. Host themes and density apply automatically;
regions scroll independently and machine values do not wrap. Supply immutable
payloads: replacement collection identities reset expansion.

## Components

| Type | Data fields | Host presentation |
| --- | --- | --- |
| `KeyValue` | `items: [{label, value}]` | Kit key/value rows; values are string, finite number, boolean, or null. |
| `Badge` | `label`, `tone` | Kit semantic badge with visible text. |
| `Metric` | `label`, `value: number \| null`, optional `unit`, `description` | Kit metric tile. |
| `Conditions` | `items: [{type, status, reason?, message?, lastTransitionTime?, observedGeneration?}]` | Condition name, literal True/False/Unknown status, reason, message, timestamp and generation. True is not assumed healthy: a Degraded condition can be true. |
| `Events` | `items: [{type, reason, message, count?, time?}]` | Event type/reason/message, count (default 1), optional timestamp. |
| `Table` | `columns: [{key, label}]`, `rows: [[value, …]]` | Kit table; column keys must be unique and row widths must match. |
| `Timeline` | `items: [{time, title, detail?, tone?}]` | Ordered entries in supplied order, with explicit timestamps. |
| `Markdown` | `text` | Safe host Markdown subset described below. |
| `Code` | `text`, `language: "yaml" \| "none"` | Read-only, flush kit CodeEditor with Copy. No execution or editing. |
| `Timeseries` | `label`, `unit`, `range: {start, end}`, `times`, `series: [{name, values}]`, optional `thresholds: [{label, value, direction, tone}]` | Host-drawn line chart with a text legend, threshold list and a values table. See [Timeseries](#timeseries). |

Tones are `sev`, `warn`, `ok`, `info`, `accent`, and `muted`; they select host
semantic tokens, never extension-defined colors. Timestamps use the schema's
ISO-shaped timestamp syntax and display as supplied, without inventing relative
ages. Partial reads and backend truncation are facts owned by the embedding
host; retain their notices outside the component. An empty result must not be
used to hide a failed or incomplete read.

## Bounds

- Collections: at most 1,000 items. Tables: 1–20 columns and at most 1,000 rows.
- Labels: 1–256 characters; general strings and scalar values: 4,096; units: 64.
- Markdown: 16,384 characters; Code: 262,144. Larger content must use a separate
  host-controlled large-document surface; the catalog reports an error.
- Across a payload: at most 524,288 UTF-16 code units including property names,
  25,000 visited values, and eight nesting levels. Only plain JSON data is
  accepted: no accessors, custom prototypes, cycles or non-finite numbers.
- Lists mount 20 items initially. Counted buttons reveal at most 20 more per
  click and can collapse the list again. Markdown lists, blocks, and table rows
  use the same behavior. Markdown tables exceeding 20 columns fail visibly.

Schema bounds apply to individual values and arrays; aggregate traversal limits
and table key/row consistency are additionally enforced by the runtime validator.

## Timeseries

The chart primitive for metric data (#570). An app never ships chart code: it
supplies samples, and the host draws them with its own theme, layout and
accessibility. The shape is the one a metric provider (#569) fills from a range
query; until providers exist, a host projection supplies it the same way the
other components are supplied.

```json
{
  "version": 1,
  "type": "Timeseries",
  "data": {
    "label": "CPU usage",
    "unit": "cores",
    "range": { "start": 1790251200000, "end": 1790254800000 },
    "times": [1790251260000, 1790251320000, 1790251380000],
    "series": [
      { "name": "web-7d9f", "values": [0.21, null, 0.34] },
      { "name": "api-5c1b", "values": [0.10, 0.12, 0.61] }
    ],
    "thresholds": [{ "label": "Limit", "value": 0.5, "direction": "above", "tone": "sev" }]
  }
}
```

- **Time axis.** `range` is the time window the chart spans, and `times` the
  shared sample times, both integer epoch milliseconds. Times strictly increase
  and lie inside the range, which must start before it ends. The plot spans the
  whole range, so missing data at either end is visible rather than hidden by
  auto-fitting.
- **Series.** One to eight, with unique names. Each has exactly one value per
  time. `null` is a gap — no sample — and breaks the line; it is never drawn or
  counted as zero. A sample alone between gaps is drawn as its series' marker.
- **Units.** One of `number`, `percent` (0–100), `ratio` (0–1, shown as a
  percentage), `bytes` and `bytesPerSecond` (B, then binary prefixes KiB, MiB,
  GiB, TiB and PiB; `bytesPerSecond` adds `/s`),
  `seconds` (µs, ms, s, min, h), `cores` and `perSecond`. The host formats every
  value; an unknown unit or a format string is refused. One unit applies to the
  whole chart, which has a single value axis.
- **Thresholds.** At most four, with unique labels; `direction` is `above` or
  `below`, `tone` is `info`, `warn` or `sev`. A sample breaches a threshold when
  it is strictly beyond it. The value axis always includes zero and every
  threshold, and ends on its own gridlines.
- **Bounds.** At most 1,000 times per chart (the catalog's collection bound), so
  at most 8,000 samples. Anything larger, out of order, misaligned or non-finite
  is refused with a visible error, never truncated.

### What the host draws

The value-axis title names the unit (`CPU usage (cores)`). Series are coloured
from the theme's mark palette, light and dark, and each also has its own dash
pattern, repeated in its legend swatch, so none is told apart by colour alone.
A sample alone between gaps has no line to carry that pattern, so each series
also has a marker shape (circle, square, triangle, diamond, down-triangle,
plus, cross, asterisk, in series order), drawn at such samples and in the
legend swatch. Markers are unfilled outlines, so series with a sample at the
same time and value draw different shapes at one point and neither paints over
the other. Thresholds are dashed lines labelled in text with their
value.

Below the plot, as text:

- the legend: each series' name, latest, lowest and highest sample, and for each
  threshold it breaches, a badge such as “Above Limit: 12 of 60 samples”;
- the threshold list, with direction, value and severity in words;
- a **Show values table** button that reveals every sample as a table (time, then
  one column per series), paged 20 rows at a time like other collections. A gap
  reads “No sample”; a breaching value is followed by “· above Limit”.

Axis labels and legend figures round to four significant digits, and switch to
the locale's scientific notation (`1.5E9`) when the figure's magnitude is at
least 1e9 or below 1e-4, so an extreme value still fits the axis margin. The values
table and the threshold list print every digit of the value as sent (seconds
stay in seconds there rather than rounding to minutes), so a value that
breaches a threshold never reads as equal to it. Table rows show each sample's
time to the second whatever the range, and to the millisecond when any sample
has one, so no two rows read the same. The summary's range shows its exact
ends: seconds for ranges up to five minutes, and milliseconds too when either
end is off a whole second. Numbers and times use the reader's locale and time
zone. When the times shown (table rows, axis labels, the summary's range) span
more than one UTC offset, as across a daylight-saving fall-back where local
01:30 happens twice, every one of them also shows its offset (`01:30 GMT-4`,
`01:30 GMT-5`), axis labels that are dates (`Nov 1, GMT-4`) included.
Samples at the extremes of the double range are drawn without losing the axis:
its ends fall back to the samples themselves when rounding out would overflow.

### Time axis

Ticks fall on round times, never wherever the range happens to begin. The step
comes from a fixed ladder:

- 1, 2, 5, 10, 20, 50, 100, 200, 500 ms
- 1, 2, 5, 10, 15, 30 s
- 1, 2, 5, 10, 15, 30 min
- 1, 2, 3, 6, 12 h
- 1, 2, 7, 14 days
- 1, 2, 3, 6 months, on the 1st
- 1, 2 and 5 × 10ⁿ years, on 1 January

The host takes the smallest step whose labels fit: as many as the plot's width
holds at the length of the labels that step would really draw (offsets and
years included), but at most six on a wide plot and four in a narrow one. If that leaves fewer than two ticks, it takes the next finer step,
thinned evenly to as many ticks as fit while keeping its first and last.
Ticks are multiples of the step in the reader's local time, so an hour-step
axis reads 00:00, 06:00, 12:00 and a 15-minute one 14:15, 14:30. Hour and day
steps walk local calendar dates, so a daylight-saving change never shifts
them; each label is formatted from its tick's own instant.

A label's precision follows the step: milliseconds for sub-second steps,
seconds for second steps, hours and minutes for minute and hour steps, the date
for day steps, month and year for month steps, and the year for year steps. On
an hour-step axis a local midnight shows its date instead of 00:00, so a
multi-day axis says which day it is. Every label is its tick's true time at that
precision; none is rounded to a minute it isn't on.

The plot itself is one image to assistive technology, labelled with the axis
title, series count, time range and thresholds; the values table is its full
text equivalent.

### Downsampling

The plot draws at most 400 points per series. Up to that, every sample is
drawn. Above it, the samples are split into 200 contiguous buckets of equal
count and each bucket keeps its lowest and highest real sample, in time order
(min–max bucketing). No value is averaged or interpolated: every drawn point is
a sample the app sent, and each series' global minimum and maximum always
survive, so a one-sample spike is never smoothed away. A gap anywhere in a
bucket still breaks the line. When a chart is reduced it says so under the
legend; the legend's figures, breach counts and the values table always use
every sample.

### States

The host's `state` prop gives loading, error and no data exactly as for the
other components: a failed query shows its cause and a retry, never “No data
reported.”. A chart whose provider answered with no samples in range — no times,
or every value `null` — shows “No data reported.”.

## Markdown and external links

The renderer reuses the host's flat Markdown parser: headings, paragraphs,
bold, inline code, bullets, numbered lists, pipe tables and fenced code.
Unsupported syntax remains literal text. Raw HTML never enters the DOM;
images, scripts, styles, iframes and embedded HTML links are not supported.
Code fences use the same read-only CodeEditor. Inline links are recognized only
in plain text, without nested markup or image syntax.

Only normalized absolute HTTP(S) links without credentials, whitespace,
control characters, direction controls or backslashes are clickable. Other
schemes and relative links remain text. Visible external-link marks identify
navigation; a user click dispatches through `openNativeComponentLink` to the
native browser command on desktop or a new isolated browser tab on web. The
helper validates again at click time. Opening failures show their cause and a
retry action. Rendering or parsing a document never opens a link.
