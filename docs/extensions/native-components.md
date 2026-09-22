# Native component vocabulary, version 1

The host renders nine data-only components using `@srelens/ui-kit`. The contract
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
