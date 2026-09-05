# Working in this repo

Read [`CONTRIBUTING.md`](CONTRIBUTING.md) and [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md)
first. They are the guide: setup, architecture, the capability registry, the
test-driven rule, the coverage floors. This file does not repeat them.

What is here is the short list of traps this codebase has actually sprung —
each one a bug that shipped, passed CI, and was found by a person looking at
the screen. If a rule below reads as obvious, it is only obvious in hindsight.

## Capabilities: the wire name is the caller's, not the struct's

A capability's `*In` struct is deserialized from the JSON that
`@srelens/core`'s wrapper sends. The wrapper writes `camelCase`. Rust writes
`snake_case`. **Any multi-word field must carry `#[serde(rename = "…")]`**
naming it exactly as the wrapper sends it — `ListEventsIn`,
`ListReplicaSetsIn` and `PodsForSelectorIn` each do.

`OpenApiSchemaIn` did not. Its field was `api_version`; every caller sends
`apiVersion`; so every call failed to deserialize and editor field
autocomplete never worked in either design, in any release. Nothing caught it
for two reasons, and both are rules:

- **Test with the caller's payload, not the struct's field names.** The e2e
  case sent `{"api_version": …}` — the struct's own spelling — so it agreed
  with the bug instead of catching it. An e2e case that hand-writes JSON is
  asserting a contract; write the contract the app actually speaks. Better
  still, add a unit test that deserializes the wrapper's payload and asserts
  the wrong spelling is rejected, so nothing drifts back.
- **Never collapse a failed call into a fact about the cluster.** See below.

Adding a capability? Follow the walkthrough in `docs/DEVELOPMENT.md`, and
before you finish, read your `*In` struct beside the `core` wrapper that calls
it, field by field.

## Say what you know, not what you guess

Two different facts must not render as one sentence:

- the call failed (refused, timed out, unreachable), and
- the cluster answered, and the thing genuinely is not there.

The schema pane rendered both as "The cluster has no schema for Secret" — a
confident, wrong claim about a built-in kind that certainly has one, and it
hid the bug above for as long as it stood. A failed call says what failed,
gives the reason through `describeError`, and offers a retry. An absence says
the cluster serves none.

The same rule caught an apply that came back with no documents and was toasted
as a success, and a dry-run diff whose failure was rendered as "No changes." —
false reassurance one click before Apply.

## The route is the identity

`openTab` dedupes by route **string**. Whatever distinguishes two things a
reader can have open at once has to be *in* the route, or the second one
silently focuses the first:

- `/edit/<cluster>/<kind>/<namespace>/<name>` — the cluster is in it because
  the editor pins the cluster it opened on. Without it, Edit on staging's
  `default/web` focused prod's draft and staging's could not be opened at all.
- A custom kind carries its group (`acme.io%2FDeployment`), because a CRD may
  legally reuse a built-in kind's name and resolving by name alone lands on
  the built-in — including for delete.
- Screens are keyed by route, **not** by the rail's cluster. Keying by
  `stableId` remounts the screen on a rail switch, which re-pins it, throws
  the draft away, and makes `useClusterGate` see `pinned === live` — so the
  write goes to the new cluster with no warning.

And: **a route with no screen registered in `lib/routes.ts` renders the
Placeholder.** That is the whole of "clicking Edit does nothing". A new screen
is not done until `SCREENS` (or a parse-based match in `screenFor`) names it,
and something in the app opens it — `/new` had a route, a screen, and no
button anywhere for weeks.

## Secrets

`k8s.getManifest` returns a Secret's values in the clear. Redact on arrival
with `redactSecretManifest`, fail closed, and keep the editor read-only until
the reader reveals — through `k8s.getSecret`, the consent-gated read. Apply
stays off while placeholders are shown, or applying writes the placeholders
over the values.

## UI

- **Don't mix a native `<select>` with the app's picker in one control
  cluster.** The kit's `Select` is a real native `<select>` and the new design
  uses it deliberately for short, fixed, app-defined option sets. But the
  browser draws its own dropdown for it, so beside a `Combobox` it reads as
  another application's control.
- **Any list of cluster-supplied values gets `Combobox` or `MultiSelect`.**
  Namespaces, contexts, containers, kinds: you do not know at design time
  whether it is four entries or four hundred, and a native dropdown over
  sixty namespaces is a wall with no search. `NamespacePicker` and
  `NamespaceChoice` in `screens/resourceShell.tsx` are the shared pair —
  reach for them rather than writing the loading state again, because that is
  the part that drifts (an empty dropdown reads as "this cluster has none").
- **Edge to edge.** The desktop layout is flush regions divided by
  hairlines, each scrolling on its own — no gutters, no floating cards, no
  page scroll. The rule is in `kit.css` under "panes" and it is easy to
  break by habit: the editor sat in `p-4` inside a rounded, bordered frame,
  which is a card in a design that has none. A component that IS a region
  fills it; a control inside one keeps its frame (that is what `CodeEditor`'s
  `flush` distinguishes). Prose still gets a gutter — a document does not.
- **Machine text does not wrap.** A `last-applied-configuration` annotation is
  one line of a thousand characters; `break-all` turns it into a
  paragraph-shaped block that hides the short lines around it. Let it run and
  scroll the block.
- **Do not render walls.** A manifest diff is almost entirely unchanged lines.
  Collapse long unchanged runs behind a counted, expandable gap and keep a few
  lines of context — but leave a run shorter than the marker that would
  replace it, and leave a no-change diff whole, because "unchanged" is an
  answer and an empty panel is not.
- Colour is never the only signal. A dot carries a word beside it.

## Verify by running it

Tests are the floor, not the ceiling. Every UI change in this repo that broke
in front of a user passed its suite first. Drive the screen — a Vite harness
with the `@srelens/core` wrappers stubbed is enough — and look at it.

Two examples from one branch: the Changes toggle stayed on "Hide changes"
after an apply, and the diff panel was unreadable at its column width. Both
had green tests. Neither was findable without looking.

## Windows

Three suites fail on Windows only and are unrelated to your change:
`SmallPanes.test.tsx` (ShortcutsPane) and `tailwind-sources.test.ts` (two
cases). `crates/kube/src/toolbox.rs` has a Unix-symlink test that will not
compile there. Do not "fix" them by changing what they assert; CI runs Linux.
