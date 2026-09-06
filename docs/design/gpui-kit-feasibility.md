# Moving ui-next to GPUI Kit — a feasibility

**Verdict: not as a migration, yes as a bounded experiment.** The technical
fit is better than it looks from the outside, and the project cost is worse.
The blocker is not GPUI Kit; it is that srelens would be carrying three user
interfaces instead of two, and that the browser build gives back the one
boundary whose removal is the whole financial case.

What follows is measured rather than estimated. Every number in it came from
this repository or from the GPUI Kit skills, both read on 2026-09-06.

---

## 1. What we would be moving

| | files | lines (no tests) |
| --- | ---: | ---: |
| `packages/ui-next` | 131 | 43,197 |
| `packages/ui-kit` | 89 | 13,786 |
| `packages/core` (TypeScript service layer) | 95 | 11,375 |
| `apps/desktop/src` (classic, frozen but shipping) | — | 23,871 |
| **frontend total** | | **92,229** |

And the part that is easy to forget:

| | files | lines |
| --- | ---: | ---: |
| `packages/ui-next` tests | 111 | 49,074 |

Those tests are not filler. Nearly every one of them names the defect it
exists to prevent — a stale namespace picker, a dialog that followed the
cluster rail, a diff that printed a whole document to show one line. They are
the project's memory. They do not port; they would be rewritten, and the
reasoning in them is what would be at risk.

Against that, the Rust already in the tree:

| crate | lines |
| --- | ---: |
| `kube` | 25,902 |
| `mcp` | 8,348 |
| `server` | 7,784 |
| `registry` | 2,317 |
| `streams` | 2,697 |
| `llm` | 2,129 |
| `agent` | 1,845 |
| `capability` | 244 |
| `apps/desktop/src-tauri` | 10,514 |
| **total** | **61,780** |

## 2. The strongest argument for it

**The service layer is already Rust, and `packages/core` exists only to reach
it.** All 11,375 lines of it are wrappers that marshal a call across the Tauri
IPC boundary into `crates/kube` and back, plus the types to describe what
crosses. In a GPUI application that boundary does not exist: a screen calls
the capability directly.

So `packages/core` is not ported. It is **deleted** — on the desktop. That
is the one place in this exercise where the work goes down rather than up, and
it is a real 11k lines. It also removes a whole class of bug this codebase has
actually had — the `api_version` / `apiVersion` mismatch that killed schema
autocomplete in both designs for every release was a defect that can only
exist because there is a JSON boundary in the middle. Direct calls are typed
end to end.

**The browser build does not get this.** See §4: a GPUI app compiled to
WebAssembly runs in the browser, but `crates/kube` does not — the kube
client, kubeconfigs, exec plugins and TLS all stay on the server. So the
browser build needs an RPC client to `crates/server`, which is `core`
again, in Rust. It would be smaller than 11k lines and it could share types
with the server crate rather than hand-mirroring them, which is a genuine
improvement over today. But the boundary is back, and with it the class of
bug. The deletion is a desktop win only.

The second argument is performance, and it is not theoretical here. The
topology screen was reworked twice for lag, and the resource lists are
virtualized by hand. GPUI Kit ships `DataTable` and `VirtualList` built for
"hundreds of thousands of rows", GPU-composited. That is squarely the shape of
this app.

## 3. What GPUI Kit actually covers

Read from the installed skills rather than the marketing page.

**Covered, and well:** Button, Input, Select, Combobox, Checkbox, Switch,
Radio, Icon, Dialog, AlertDialog, Notification, Tabs, Tooltip, Form, a
searchable virtualized List, DataTable with resizable and sortable columns,
DockArea with draggable tabs, charts, Markdown rendering, and a rope-backed
code editor with Tree-sitter highlighting and LSP diagnostics.

That last one matters more than it sounds. The manifest editor is one of the
larger things in ui-next and it is CodeMirror plus our own lint, completion
and diff plumbing. GPUI Kit's editor arrives with the highlighting and
diagnostics already attached.

**Two assumptions I had going in were wrong**, and both are worth correcting
because they would have changed the verdict in the wrong direction:

- *Testing.* GPUI has a real framework, documented across 741 lines:
  `#[gpui_kit::test]`, `TestAppContext`, `VisualTestContext`, and a
  single-threaded deterministic executor. Not Testing Library, but not a gap.
- *Accessibility.* The coding guides direct you to test "through the
  accessibility tree by role, label, value, enabled state, focus, and
  selection". There is a tree, and it reaches the OS: on Windows the window
  reports itself to UI Automation as framework `AccessKit`, and the first
  screen (§7) exposed its two pickers as ComboBoxes and its Refresh as a
  Button. Two things measured against it, both worth knowing: the pickers
  arrived with **empty names** until the app set `accessibility_label` —
  names are the application's job, as they are in HTML — and the
  `DataTable` **did not appear in the tree at all**. The pickers also
  advertise UI Automation's `ExpandCollapse` pattern, and calling it left
  them collapsed with no option ever entering the tree — declared, not
  honoured. A list screen whose list is invisible to a screen reader, behind
  a picker a screen reader cannot open, is not shippable to the readers this
  codebase's 7:1 promise was made for. Whether those are gaps in this
  toolkit version or in AccessKit's Windows adapter is the first thing the
  experiment should run down, before any second screen.

## 4. What has no answer

**No terminal.** `Terminals.tsx` and the pod shell are xterm. The word
"terminal" appears exactly once across all 27 installed skill files, and it is
about punctuation. Zed has an alacritty-backed terminal, but it is not part of
`gpui-kit`. This is build-it-or-vendor-it, and it is not small.

**Web mode is possible, and it is the least proven path.** An earlier draft
of this document called GPUI native-only. That was wrong, and the correction
was sitting in the installed skills: the coding guides list "macOS / Windows /
Linux / wasm" as the platform axis and say to "preserve platform/wasm
differences when an API is not portable". GPUI compiles to `wasm32` and
renders through WebGPU in the browser, and gpui-component's own gallery ships
that way.

Three things temper it, all worth carrying into the experiment rather than
arguing about:

- *It is the quiet target, and it is a fork's.* Upstream GPUI — the one Zed
  ships — does not run in a browser. Zed's own discussion of it has "no work
  on wasm support" in mid-2024, lists "Zed on the web" as a milestone beyond
  1.0, and by February 2026 the maintainers were redirecting the question to
  Discord. The browser target that gpui-kit exercises therefore lives in
  Longbridge's line of GPUI, not Zed's. That is why the gpui-component README
  says "one Rust codebase to macOS, Windows, and Linux" and never mentions the
  browser, and why the skills treat wasm as a set of differences to work
  around rather than a headline. It also means the browser path depends on
  one company keeping a divergence from upstream alive. The one concrete
  rough edge I found — `std::time::Instant` is unimplemented on `wasm32`, so
  scroll axis-locking had to be disabled in the browser to stop a panic — is
  the kind of thing a quiet target has more of.
- *WebGPU narrows reach.* Today's web mode is a React app: it opens in
  anything. A WebGPU build needs a browser that has WebGPU on, which is not a
  given in locked-down enterprise fleets or older Safari. For a tool aimed at
  operators on whatever machine is in front of them, that is a real regression
  to weigh against the rendering speed.
- *The RPC boundary returns* (§2). `crates/server` at 7,784 lines stays,
  and the browser build needs a typed client for it. The desktop's strongest
  argument does not carry across.

So browser access is not the blocker it looked like. It is the part of the
experiment most likely to surprise, and it has to be in the experiment.

**A third interface.** The classic-to-ui-next migration is *not finished*:
`design.ts` lists 19 ported routes and `apps/desktop/src` is still 23,871
lines of shipping React. Adding GPUI now means three UIs in one repository,
each needing the same bug fixed three times. This project already has a
written scar from exactly that failure mode at a smaller scale — the shell
pieces `Resources` and `Workloads` duplicated, where "a batch of fixes landed
on one that never reached the other".

**The design system is ours.** `ui-kit` is 13,786 lines built on Radix and
cmdk with a token system of five themes, five accents and three densities,
pinned by contrast tests. GPUI Kit brings its own components *and its own
normative design guides*, which are a requirement rather than inspiration.
Adopting it is adopting somebody else's design language, or fighting it.

## 5. Cost

Roughly 57,000 lines of UI to re-express (`ui-next` plus `ui-kit`), minus what
GPUI Kit's components absorb, plus a terminal, plus rewriting 49,000 lines of
tests into a different framework, plus running two or three frontends during
the transition. Rust UI code is not more compact than JSX.

There is no honest version of this that is a quarter of work. It is the
largest thing this repository would have ever done, larger than the
classic-to-ui-next migration currently in flight and unfinished.

## 6. Recommendation

**Do not start a migration. Do one screen, on purpose, to get real numbers.**

The resource list is the right subject. It is the spine of the app, it is
where the performance complaints live, it exercises `DataTable`, virtual
scrolling, the namespace picker, the row menu, and it calls `crates/kube`
directly — so it also proves or disproves the "delete `packages/core`" claim,
which is the whole financial case.

Build it as its own binary against the existing crates, **and build the
same screen for `wasm32` against `crates/server`**, because the browser
build is where this toolkit is least proven and where the desktop's best
argument stops applying. Do not wire either into the Tauri app, do not put
them behind a feature flag in the shipping product, and do not port a second
screen until the first has answered:

1. How many lines is the list screen in GPUI versus its 43k-line neighbourhood
   in ui-next?
2. Does calling `crates/kube` directly actually delete the `core` wrapper, or
   does an equivalent adapter grow back?
3. Do 5,000 pods scroll better than they do today, measured, not felt?
4. What does the accessibility tree expose, checked with a real screen reader
   on Windows and macOS?
5. How much of `ui-kit`'s token system survives GPUI Kit's own design guides?
6. Does the `wasm32` build open, scroll and render the list in the browsers
   the web mode is actually used from — measured on WebGPU availability,
   bundle size and first paint against today's React build?
7. How big is the Rust RPC client the browser build needs, and can it share
   types with `crates/server` instead of mirroring them?

If the answers are good, the migration is a real option and there is a number
to put on it. If they are not, the cost of finding out was one screen, twice.

**One thing to settle first, because it is not an engineering question:**
whether the classic design gets retired before another interface starts.
Three shipping UIs is the failure mode this repository has already met at a
smaller scale.

---

## Appendix: the skills

Installed at `.claude/skills/`, from
`longbridge/gpui-component` on GitHub:

- `gpui-kit/` — SKILL.md plus 24 references covering components, GPUI
  mechanics (actions, async, contexts, elements, entities, events, focus,
  globals, layout) and testing.
- `gpui-kit-design-guides/` — SKILL.md plus the normative design guide.

27 files, 376 KB. GPUI Kit is Apache-2.0, by Longbridge, built on Zed's GPUI;
version 0.6.0 at the time of writing.
