# Moving ui-next to GPUI Kit — a feasibility

**Verdict: not as a migration, yes as a bounded experiment.** The technical
fit is better than it looks from the outside, and the project cost is worse.
The blocker is not GPUI Kit; it is that srelens would be carrying three user
interfaces instead of two, and would lose the browser.

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

So `packages/core` is not ported. It is **deleted**. That is the one place in
this exercise where the work goes down rather than up, and it is a real 11k
lines. It also removes a whole class of bug this codebase has actually had —
the `api_version` / `apiVersion` mismatch that killed schema autocomplete in
both designs for every release was a defect that can only exist because there
is a JSON boundary in the middle. Direct calls are typed end to end.

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
  selection". There is a semantic tree. Given how much this codebase has
  invested in aria roles, live regions and a 7:1 contrast promise, an absent
  one would have been disqualifying on its own.

## 4. What has no answer

**No terminal.** `Terminals.tsx` and the pod shell are xterm. The word
"terminal" appears exactly once across all 27 installed skill files, and it is
about punctuation. Zed has an alacritty-backed terminal, but it is not part of
`gpui-kit`. This is build-it-or-vendor-it, and it is not small.

**Web mode dies.** `crates/server` is 7,784 lines that exist to serve this
frontend into a browser, with a Docker image and `docs/WEB.md` behind it. GPUI
is native only — macOS, Windows, Linux. Moving ui-next to GPUI means either
dropping browser access as a product capability, or keeping the React frontend
alive forever to serve it. That is a product decision, not an engineering one,
and it should be made before any code is written.

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

Build it as its own binary against the existing crates. Do not wire it into
the Tauri app, do not put it behind a feature flag in the shipping product,
and do not port a second screen until the first has answered:

1. How many lines is the list screen in GPUI versus its 43k-line neighbourhood
   in ui-next?
2. Does calling `crates/kube` directly actually delete the `core` wrapper, or
   does an equivalent adapter grow back?
3. Do 5,000 pods scroll better than they do today, measured, not felt?
4. What does the accessibility tree expose, checked with a real screen reader
   on Windows and macOS?
5. How much of `ui-kit`'s token system survives GPUI Kit's own design guides?

If the answers are good, the migration is a real option and there is a number
to put on it. If they are not, the cost of finding out was one screen.

**Two things to settle first, because they are not engineering questions:**
whether browser access is expendable, and whether the classic design gets
retired before another interface starts. If browser access stays, this is a
second frontend forever, and the answer is no regardless of how good the
prototype looks.

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
