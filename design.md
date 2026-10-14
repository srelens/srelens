# srelens UI design contract and visibility audit

Status: first implementation pass, 2026-09-10. Baseline: `dev` at `f0e0ec0e`.
Scope: the new desktop/web UI (`packages/ui-next` and `packages/ui-kit`).
Classic UI and the TUI have separate implementations; parity is a later review,
not a claim that this audit has tested them.

## Purpose

Make operational information readable at normal zoom without losing the compact,
edge-to-edge workspace. A reader must be able to identify the active cluster,
read a resource name, distinguish a failure from an empty result, and locate the
next action without hovering everything or increasing font weight everywhere.

This is the working design contract. Each follow-up should update its findings,
verification evidence, and status. A token calculation is evidence about a colour
pair, not proof that every screen is accessible.

## Theme and mode contract

### Cluster overview refinements (2026-09-10)

- Overview information belongs to the current cluster. Node-version counts
  reuse its loaded node list; no Fleet queries run against other contexts.
- Node actions reserve their intrinsic width so the overflow menu stays on
  the same row as Cordon/Uncordon and Drain.
- The node list displays CPU usage in cores, with up to three decimal places
  below one core and two above it; sorting still uses raw millicores.
- The overview has one title strip. Server version remains in Control plane.
  Compact tab hints retain the full title alongside context/detail so truncated
  resource names remain readable, with each identifier horizontally scrollable.
- Context labels in resource headers, tab hints and the assistant prompt prefer
  the saved short name, falling back to the display name. Requests and stored
  tab identities continue to use the original kubeconfig context name.
- The collapsed assistant prompt hides overflow within its one-line strip;
  focus opens the full editor, which retains scrolling for longer prompts.

Verified in a local browser harness with long node names, mixed versions,
sub-core CPU usage, dark theme, and tab hover. This complements component tests;
it does not replace native webview validation.

| Axis | Supported values | Contract |
| --- | --- | --- |
| Theme | Light, Paper, Dark, Midnight, High contrast | Same information hierarchy and interaction model in every theme. |
| Accent | Violet, Blue, Teal, Amber, Rose | Selection/action identity only; never redefine severity or cluster identity. |
| Density | Compact, Default, Comfortable | Change spacing and row height, never shrink the text scale. |
| System appearance | OS light/dark when no explicit theme overrides it | Resolve through the existing appearance owner, with explicit user choice winning. |
| Zoom | Existing application scale controls | Keep controls reachable; test 100%, 125%, and 200%, including a 960px window. |
| Motion | Normal and reduced motion | Reduced motion removes unnecessary movement; information and progress remain visible. |
| Platform | Desktop webview and web host | Verify native behavior separately: Chrome cannot prove a Tauri interaction works. |

There are 25 theme/accent combinations and 75 theme/accent/density combinations.
High contrast must retain its stronger text floor after changing accent.
Appearance preferences continue using the existing backend persistence. This
work introduces no browser-only settings or parallel theme store.

## Measured baseline

Ratios below are the lowest value across the five neutral surfaces at 100%
opacity, calculated from the committed CSS colours. These are source measurements,
not measurements of antialiased pixels or of every rendered component.

| Theme | Faint text | Muted text | Warning text | Strong divider |
| --- | ---: | ---: | ---: | ---: |
| Light | 2.39:1 | 4.64:1 | 3.93:1 | 1.22:1 |
| Paper | 2.52:1 | 4.74:1 | 3.73:1 | 1.27:1 |
| Dark | 3.04:1 | 5.11:1 | 7.69:1 | 1.36:1 |
| Midnight | 3.08:1 | 5.35:1 | 8.44:1 | 1.33:1 |
| High contrast | 7.20:1 | 9.70:1 | 8.03:1 | 18.76:1 |

Additional failures:

- High-contrast faint text fell to 6.56:1 on the violet selection wash. Blue,
  teal, amber and rose accents inherited ordinary light-theme colours, falling
  below the advertised 7:1 text floor.
- `.btn:hover` set the text to `--ink`; `.btn-accent:hover` did not restore
  `--accent-ink`. On a dark accent background this can make the label unreadable.
  Its brightness filter also changed the tested colour pair.
- Tab context and status-bar text were 9px, uppercase and widely tracked.
  Table headings and several grouping labels were 10px. Weight alone cannot
  compensate for that size and low contrast.
- Dividers are deliberately subtle, but the same `--rule` is also used by
  `TextInput` and `Select` as their control boundary. Those are different jobs.
- `TextInput` and `Select` carry `outline-none`; their actual computed keyboard
  focus treatment needs inspection against the base focus rule and CSS layers.
- Source inventory found eight 9px utilities in the kit and eight explicit 10px
  utilities in ui-next. These need contextual review rather than a global replace.
- Identifier wrapping remains in detail, logs, Helm, topology and MCP settings.
  The tab tooltip is fixed already; other cases need content-level review.

## Colour and visibility rules

1. Normal operational text, including placeholders, secondary text and tooltips:
   at least **4.5:1** against its actual background. The high-contrast theme targets
   **7:1** for the tested text roles. Do not round a failing ratio up to a pass.
2. Test selected, hovered, raised, sunk and error surfaces—not just white and black.
   Opacity and colour mixing change the effective pair and need separate checks.
3. Essential control boundaries, icons and state indicators target **3:1** against
   adjacent colours. Decorative hairlines may stay subtle. Introduce a dedicated
   control-boundary role rather than darkening every panel divider.
4. `--ink` carries primary data; `--ink-soft` supports labels; `--ink-muted` carries
   secondary information; `--ink-faint` remains readable when used as text.
   Faint must not mean unavailable. Disabled controls are a separate state.
5. Status has both colour and language: “Unreachable”, “Pending”, “Failed”,
   “Connected”. Never use a dot or tint as the only signal.
6. A primary button always uses its paired accent ink, including hover. An accent
   override must preserve text and focus contrast in the selected theme.

These thresholds follow [WCAG text contrast](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)
and [non-text contrast](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html).
They are design targets, not a declaration of whole-application WCAG conformance.

## Typography and weight

The new desktop design uses a 1.1 native scale at the displayed 100% baseline.
Its 80–150% choices scale relative to that baseline. Classic keeps its original
1.0 baseline and existing zoom preference. Each design stores its own displayed
percentage in backend settings. CSS sizes below are before native zoom.

Use the existing system UI face for navigation and resource data. Use monospace
for manifests, code and commands where alignment helps. Resource-table values,
including node names, namespaces and images, share the UI font; an identifier
is not a reason to switch typefaces. Keep these values single-line. Do not
uppercase cluster names or identifiers: preserve their case and spelling.

| Role | Size at 100% | Weight | Use |
| --- | --- | --- | --- |
| Metadata | 11px / `--text-meta` | 400; 500 for compact navigation | Context tags, timestamps, table headings, status readouts. |
| Label | 12px / `--text-label` | 500 | Form labels, buttons, compact navigation. |
| Body/data | 13px / `--text-body` | 400 | Resource rows, values, normal operational content. |
| Prose | 14–15px | 400 | Explanations, release notes, onboarding. |
| Section title | 13–16px | 600 | Pane and section hierarchy. |
| Page title | 20–22px | 600 | Home and major page introductions. |

Use `--weight-regular`, `--weight-medium` and `--weight-semibold` for new shared
rules. Existing equivalent component rules can migrate incrementally. Reserve
700 for a demonstrated need, not a general visibility fix. Keep tabular figures
for changing metrics. Prefer sentence case; short section markers can be uppercase
with modest tracking. Text below 11px requires a documented exception (for example,
a tiny identity mark accompanied by a full adjacent name).

Machine text stays on one line and scrolls in a bounded region. Prose wraps.
Truncated navigation labels expose their full identity on focus and hover.

## Layout and interaction

- Edge-to-edge regions, hairline separators, independent scrolling, no page-level
  card gutters. Prose receives reading padding; an editor or table fills its pane.
- Keep density independent of font size and zoom. Existing rows are 22/27/29px;
  changing those is a separate layout decision with realistic row counts.
- Keep search and primary actions visible at 960px. Toolbars may wrap; fixed
  controls must not be pushed outside the window.
- Selected, hovered, focused, disabled, busy and invalid are distinct states.
  Keyboard focus must be visible even on an already-selected item.
- Do not reveal essential actions only on pointer hover. Provide a keyboard path
  and expose the action while its row/control has focus.
- Aim for 24×24px interactive targets; dense controls need adequate spacing or
  an equivalent larger control. Audit actual hit areas before claiming compliance
  with [WCAG target size](https://www.w3.org/WAI/WCAG22/Understanding/target-size-minimum.html).
- Loading, empty, unavailable and failed are different states. Preserve the cause
  and offer retry for failures; never report absence when a request failed.
- Destructive actions retain the existing consent and cluster-identity safeguards.

## Screen audit and work queue

“Inventoried” means the screen family and relevant shared styles were identified.
It does not mean all screen code, live-cluster states or native interactions were
exercised. Only the first-pass shared components have rendered evidence below.

| Area | Findings / next verification | Status |
| --- | --- | --- |
| Shared tokens and primary buttons | Full theme/accent text pairs; hover colour inheritance. | Fixed in first pass; automated coverage. |
| Tabs and status bar | 9px machine labels, heavy tracking, faint metadata. Verify overflow at minimum width. | Shared type scale updated; 75-combination component matrix checked. |
| Tables and lists | Header/group label legibility; selected-row text and live numbers. | Headings and missing selected-row highlight fixed. End-column resize handle produces a small horizontal scroll range in the fixture; per-screen audit remains. |
| Forms and settings | TextInput/Select focus suppression confirmed in Chrome; weak boundaries; long MCP addresses remain. | Shared input/select/picker boundaries and input/select focus fixed. Settings screen audit next. |
| Home and connections | Name/context/status hierarchy; 16+ clusters; narrow width and unavailable state. | Inventoried; rendered screen audit pending. |
| Overview and metrics | Caption size, chart axis/legend contrast, zero/unknown/loading distinction. | Inventoried; chart/state audit pending. |
| Resource detail and editor | Identifier wrapping; secret reveal/read-only states; diff context and schema errors. | Inventoried; content and focus audit pending. |
| Logs, terminals and application log | Machine line overflow, timestamp hierarchy, selection, search marks, terminal palette. | Inventoried; stream and terminal audit pending. |
| Helm and port forwards | Operation status labels, failure/readiness distinctions, command/resource identifiers. | Inventoried; operation-state audit pending. |
| Topology | Small node labels, dimmed edges/labels, selected/hovered graph contrast. | Inventoried; dedicated graph audit pending. |
| Agent and console | Streaming text, citations, tool status, code blocks, disabled send state. | Inventoried; streaming audit pending. |
| Menus, dialogs, pickers, drawers | Focus visibility, selected options, scroll affordances and return focus. | Shared components available for visual review; full keyboard pass pending. |
| Identity marks and shell chrome | Small glyph/badge exceptions, disconnected identity, active-cluster indication. | Separate identity palette retained; hit-area audit pending. |
| Motion, system mode and zoom | OS changes, reduced motion, 125%/200% scale and density combinations. | Verification backlog. |
| Classic UI and TUI | Separate token and rendering systems. | Out of this implementation pass; parity inventory later. |

## First implementation pass

- Adjust neutral secondary/faint inks without changing surface colours or accent
  identities. Improve light/paper warning and success text.
- Preserve high-contrast accent choice with dedicated darker accent colours.
- Retain accent ink on primary-button hover; replace the contrast-changing filter
  with an inset ink outline so hover stays visible without changing text contrast.
- Add metadata/body/label and weight tokens; apply metadata scale to tab context,
  status readouts, eyebrows and table/group headings. Preserve machine-label case.
- Give form inputs/selects/pickers a separate `--control-line` with at least 3:1
  contrast on neutral surfaces; remove the utility suppressing input/select focus.
  Give placeholders explicit theme ink instead of inherited half-transparent text.
- Match the table selection rule to the `data-state="selected"` attribute the
  component actually renders, retaining compatibility with `data-selected`.
- Add tests for 59 text pairs in each of 25 combinations (1,475 comparisons),
  form boundaries, primary hover ink, metadata roles and selected-row styling.

## Verification and completion criteria

Run `pnpm exec vitest run packages/ui-kit/src/theme-contrast.test.ts` for the
colour/type contract, plus the full coverage suite, typecheck and production build.
The test resolves root, theme and accent declarations from the stylesheet and
asserts that every expected token exists. It does not model arbitrary transparency,
OS forced colours or every descendant utility override.

For each follow-up, use actual components with representative content and record:

- Themes/accent/density/zoom and viewport tested.
- Normal, hover, keyboard focus, selected, disabled, busy, empty and error states.
- Before/after screenshots or measured computed styles and element bounds.
- Desktop-webview verification for native interactions, separately from browsers.
- Remaining limitations, without marking untested screens complete.

A screen is complete when readable content, distinguishable states, keyboard
access, bounded scrolling and realistic narrow-window behavior have all been
verified. Passing token tests alone does not complete a screen.


## First-pass measured result and visual evidence

| Theme | Faint text after (neutral surfaces) | Muted text after | Control boundary minimum |
| --- | ---: | ---: | ---: |
| Light | 4.79:1 | 5.66:1 | 3.21:1 |
| Paper | 4.74:1 | 5.63:1 | 3.42:1 |
| Dark | 5.19:1 | 7.06:1 | 3.63:1 |
| Midnight | 6.09:1 | 7.68:1 | 4.24:1 |
| High contrast | 8.04:1 | 9.70:1 | 6.26:1 |

The stronger text targets also pass on the tested selected/wash backgrounds;
the neutral-surface minima above do not replace that broader test matrix.

A local Vite fixture rendered the real TabStrip, Table, Button, Badge, Field,
TextInput, Select and StatusBar components with sample data. Chrome checked all
75 theme/accent/density combinations at 1280×800: metadata computed to 11px in
all combinations, and there was no horizontal document overflow. At 960px, the
fixture retained its tab controls at 100%, 125% and 200% CSS zoom. This is a
shared-component layout check, not a full-app zoom or native-webview certification.

The focus check used keyboard modality: TextInput and Select changed from
`outline-style: none` to a solid 2px outline. Primary-button hover retained its
paired ink and no brightness filter. Review screenshots alongside the computed
checks; neither substitutes for testing a real screen with real content.

| Mode | Before | After |
| --- | --- | --- |
| Light | [Baseline](docs/design/before-light.png) | [Updated](docs/design/after-light.png) |
| Dark | [Baseline](docs/design/before-dark.png) | [Updated](docs/design/after-dark.png) |
| Paper | — | [Updated](docs/design/after-paper.png) |
| Midnight | — | [Updated](docs/design/after-midnight.png) |
| High contrast | — | [Updated](docs/design/after-contrast.png) |

[Keyboard focus after the fix](docs/design/focus-light.png) ·
[Primary-button hover](docs/design/hover-light.png).
Before images use the baseline token/component styles. After images also wrap
the sample table in a scrolling region, as application screens do; that fixture
correction is not a change to the Table component's layout contract.

Next batch: take the Home, Settings and Connections screens through the same
state/viewport matrix, then charts and stream-heavy screens. Keep their audit
rows open until that rendered work is done.


Validation recorded for this pass:

- Full frontend coverage run: 386 files, 6,288 tests passed; 90.82% line coverage.
- Final focused component/contrast tests also pass after the placeholder and hover
  refinements made following that full run.
- `pnpm typecheck`, production `pnpm build`, and `cargo test --workspace --offline`
  passed.
- The shared component styles were inspected in the component fixture. Native
  desktop gestures and full-screen state matrices remain explicitly unverified.
