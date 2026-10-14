# @srelens/core

The service layer shared by every srelens frontend: the transports (Tauri IPC
on the desktop, HTTP and WebSocket on the web host), the typed wrappers over
backend capabilities, Kubernetes verdicts and formatting, settings storage,
notifications. The new design (`packages/ui-next`) and classic
(`apps/desktop/src`) both build on it. The component kit (`packages/ui-kit`)
deliberately does not: a design system that knows about capabilities is not
reusable, and `tokens-only.test.ts` fails on any `@srelens/core` import in the
kit.

This file records the package's boundary, so the next module added here does
not have to infer the rule from the imports.

## What belongs here

A module belongs in core if it would still make sense with a different UI in
front of it. If it renders, holds view state, or knows what a tab, a screen or
a dialog looks like, it belongs in the UI package that draws it.

## No React behind the main entry point

`@srelens/core` is consumed by more than one UI, so it must not reach React.
The package has two kinds of entry point:

| Import | Holds | React? |
|---|---|---|
| `@srelens/core` | the barrel, `src/index.ts` | never, at any depth |
| `@srelens/core/react` | `src/react.ts`: the two hooks that need React, `useAccess` (`lib/access.ts`) and `useNamespaceOptions` | yes |

The narrower subpaths `./transport`, `./platform`, `./webTransport`,
`./wsClient` and `./lib/*` are views onto the same modules. `./lib/*` reaches
every module, the two hook modules included, so import those hooks through
`./react` rather than by their `lib/` path.

`src/no-react.test.ts` enforces this. Its tests:

- **No React from the barrel, at any depth.** It walks the real import graph
  from `index.ts`. A module is only safe if nothing it reaches imports React,
  directly or through a package that requires it. A direct-import check was
  not enough: a pure mapper imported from `access.ts` once pulled React into
  the barrel transitively.
- **No declared runtime dependency that requires React.** `notify.ts` once
  imported `sonner`, a React library, and leaked React through an external
  package rather than a relative import.
- **Nothing imported that is not declared.** A phantom dependency resolves
  through hoisting locally and fails in the Docker build.
- **The two hooks stay out of the barrel.**
- **React is still reachable behind `./react`.** This guards the first test:
  if the hooks stopped importing React, it would pass for the wrong reason.

## Consumed as source

`package.json` exports `.ts` files directly, and the package has no build
step. Every consumer compiles it with its own toolchain. A build would add
latency and a `dist/` to keep in sync, for nothing.

## One coverage gate across the packages

The coverage floors live in the root `vitest.config.ts`, which runs every
package as one project set (`apps/desktop`, `packages/core`, `packages/ui-kit`,
`packages/ui-next`). They were calibrated when every frontend module lived in
`apps/desktop`, and when core was split out, each half measured alone sat below
them. So CI runs
`pnpm test` from the root, not `pnpm -r test`, which skips the root and would
report green with no gate at all. The comment on that step in
`.github/workflows/ci.yml` is what holds the choice.

`apps/desktop/src/coverage-config.test.ts` asserts the floors are in the place
vitest reads them, and that every package with a `vitest.config.ts` is in the
root project list. A package missing from that list goes untested while the
run still reports green: `packages/ui-next` once did.

## What moving code here broke, and the guards since

Extracting this package (#311) broke four things outside the moved code. Each
now has a guard. A module moving between packages can hit the same four:

| What broke | Guard |
|---|---|
| The CI coverage gate: recursive `pnpm -r test` skips the root config, so it ran with no floors | `coverage-config.test.ts`, above, and the comment on CI's test step |
| A Rust test reading a moved JSON asset: the capability catalog is now `src/lib/capability-catalog.json` | `capability_catalog_json_is_in_sync` (`crates/registry/src/lib.rs`), which reads it from its new path. Regenerate with `UPDATE_CATALOG=1 cargo test -p srelens-registry` |
| The shadcn CLI's aliases: `components.json` still pointed `cn` at `@/lib/utils` after it moved | `apps/desktop/src/components-config.test.ts`, which checks that `utils`, `components`, `ui` and the tailwind entry point at files and directories that exist |
| The Docker build: the image builds the frontend, so every workspace member's manifest and sources must be copied in | `apps/desktop/src/dockerfile-workspace.test.ts`, which checks each package under `packages/` is copied before `pnpm install` and before the build |

Rust tests also read other files under `src/lib/` by path, such as the
extension stream and log-request JSON in `crates/registry/src/extensions/`.
Moving or renaming one of those breaks them in the same way.
