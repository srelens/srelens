# Changelog

## Unreleased

Release notes are otherwise generated from commit subjects when `release.yml` publishes a
release: its Features and Fixes lists. This file holds what a subject cannot say, such as
what to do when upgrading and which features are previews. Issue and pull request numbers
are in srelens/srelens.

### Extension platform

What changed in extensions since v0.15.0 (2026-09-20).

#### Upgrading from 0.15.0

v0.15.0 supports extension API 0.1 only. This release supports API 0.3 to 0.6 and does not
support 0.1 or 0.2. After the upgrade, an installed app that requires API 0.1 is
quarantined: it is disabled, not removed, and **Settings → Apps** shows the reason. That
includes Flux and Argo CD.

Reinstall Flux and Argo CD from **Settings → Apps → Catalog**. The reinstall lifts the
quarantine and keeps each app's cluster selection and its **Allow plain HTTP to this
computer** choice. A saved setting is kept only if the new release still declares it, and
the current Flux and Argo CD releases declare none, so settings saved under 0.15.0 are
dropped. Until an Updates view exists (srelens/srelens#563), srelens does not look for new
releases or offer them, so the reinstall is by hand. The details are in
[migration.md](docs/extensions/migration.md#upgrading-from-0150).

#### Preview

Executable apps, which run a program they ship as a sandboxed sidecar, are a preview. So is
the API they need, API 0.6, until it is frozen as 1.0 (srelens/srelens#582). Where they
run:

- Windows: out of the box.
- Linux: out of the box on a systemd desktop with Landlock. The deb, rpm, AppImage and AUR
  packages ship the launcher `srelens-sandbox-launch`, and srelens asks your systemd user
  manager for a delegated cgroup. On systemd before 252 (Ubuntu 22.04) and the RHEL 9
  family, delegate the `cpu` controller first
  ([how](docs/extensions/manifest.md#where-executable-apps-run)). Without systemd,
  `SRELENS_SANDBOX_LAUNCHER` and `SRELENS_SANDBOX_CGROUP_ROOT` name them. Where a piece is
  missing, the app installs, but srelens refuses to start its sidecar and says what is
  missing.
- macOS: not yet. srelens refuses to start any sidecar until its memory and CPU watchdog has
  been checked with Seatbelt on a macOS 27 Mac.
- The web host: it refuses to install an executable app.

An executable app also needs a verified publisher, or the off-by-default setting **Allow
unsigned apps to modify clusters and run code**. What the platform does not yet protect is
listed in [security.md](docs/extensions/security.md#not-yet-protected).

#### Added

- Declared, host-enforced actions: primitives (srelens/srelens#665), preconditions
  (srelens/srelens#668), bulk actions (srelens/srelens#669), workload restart and node
  cordon (srelens/srelens#673), and Flux and Argo CD actions in their manifests
  (srelens/srelens#674).
- One host confirmation for UI and MCP writes, with impact levels (srelens/srelens#664,
  srelens/srelens#661). Every write is audited (srelens/srelens#660). An update shows its
  access changes before it applies (srelens/srelens#681).
- A setting, off by default, **Allow unsigned apps to modify clusters and run code**. An
  unsigned app that declares write actions needs it (srelens/srelens#677), and so does one
  with a `k8s.exec` binding (srelens/srelens#746) and every unsigned executable app,
  whether or not it writes (srelens/srelens#751). An app signed by a verified publisher
  does not.
- Declarative UI: table columns (srelens/srelens#682), detail panels (srelens/srelens#683),
  status resolvers and badges (srelens/srelens#685), dashboard cards (srelens/srelens#686),
  commands (srelens/srelens#689), links (srelens/srelens#690, srelens/srelens#740), typed
  settings (srelens/srelens#691), secret settings kept in the desktop's keychain-backed
  vault (srelens/srelens#710), the component catalog (srelens/srelens#678), bindings that
  accept several served CRD versions (srelens/srelens#692) and a timeseries chart
  (srelens/srelens#696).
- Live data and reach: the stream contract, with cancellation owned by the view
  (srelens/srelens#697), watches on bound kinds (srelens/srelens#706), `network.http` with
  host allowlists (srelens/srelens#720), and logs, exec and port-forward for apps
  (srelens/srelens#746).
- Distribution: `.srelens-extension` packages with whole-package signatures
  (srelens/srelens#741), and a signed catalog with publisher delegation
  (srelens/srelens#745).
- Executable apps: the sidecar supervisor and JSON-RPC protocol (srelens/srelens#743),
  broker callbacks and a data directory (srelens/srelens#750), the `executable` app kind,
  which starts a sidecar on its first operation call (srelens/srelens#751), the Extension
  Inspector with per-app logs and metrics (srelens/srelens#749), the protocol package and
  its JSON Schema (srelens/srelens#756), the Rust SDK (srelens/srelens#765) and the Go SDK
  (srelens/srelens#771). The sandbox feasibility spike behind them is
  srelens/srelens#698.
- MCP: every installed app's readers and declared actions, and an executable app's
  operations, are MCP tools named `plugin/<id>/<name>`, and the servers send
  `tools/list_changed` as apps change (srelens/srelens#751).
- Sidecar limits: the Inspector shows a sidecar's memory on Windows
  (srelens/srelens#777). On macOS, a host-enforced memory and CPU watchdog is built
  (srelens/srelens#781). It is weaker than the kernel limits on Windows and Linux, because
  it bounds sustained use and not a burst between its readings, and macOS still refuses
  every sidecar until it has been checked with Seatbelt on a macOS 27 Mac.
- Rust SDK: a test that a stream handler that panics ends its stream with one error and the
  sidecar keeps serving, and a settable log level, `Sidecar::log_level`
  (srelens/srelens#782). Host calls are checked before they are sent, and capped at the
  host's `maxConcurrentRequests` (srelens/srelens#783).
- Web: a per-user Apps inventory with a shared read-only catalog (srelens/srelens#717), and
  an operator extension policy (srelens/srelens#742).
- Performance budget tests (srelens/srelens#716), and the threat model records the parser
  fuzzing (srelens/srelens#718).

#### Changed

- API lines 0.3 to 0.6 are supported. 0.1 and 0.2 are retired, and their installed apps are
  quarantined until replaced. The retirement window of at least two srelens minor releases
  applies from API 1.0, so a line before 1.0 can be retired sooner, as these two were
  ([specification.md](docs/extensions/specification.md#versioning)).
- Flux and Argo CD actions moved from core into app manifests (srelens/srelens#674). Fields
  first added to 0.3 moved to a new 0.4 line (srelens/srelens#715).
- App logos come from packages, not the bundle (srelens/srelens#741). Flux and Argo CD show
  their initials until their releases ship as packages.
- The sidecar protocol crate's JSON Schema generation, `schema()` and the `JsonSchema`
  derives, is now an optional `schema` feature that is off by default. A sidecar built on
  the Rust SDK no longer builds schemars. Code that calls `schema()` must turn the feature
  on (srelens/srelens#784).

#### Fixed

- App pages route by context key (srelens/srelens#719) and find their cluster by its pinned
  ID (srelens/srelens#726).
- Streams end with their window and are scoped to the window that owns them
  (srelens/srelens#732, srelens/srelens#736, srelens/srelens#752).
- The Rust SDK answers `health`, `activate` and `deactivate` while its handlers block, and
  writes nothing after the `shutdown` answer (srelens/srelens#785).

### Other fixes

Not specific to extensions. The generated release notes list the rest.

- On **All namespaces**, two objects with the same name in different namespaces no longer
  show as one row, and deleting one no longer removes the other's row: typed watches are
  keyed by namespace and name (srelens/srelens#694).
- The log line reader is bounded, and a line that is not UTF-8 no longer ends the follow
  (srelens/srelens#748).
