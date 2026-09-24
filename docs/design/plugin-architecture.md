# srelens extension architecture

Status: native-only direction, updated 2026-09-14. This supersedes the earlier
proposal for a Freelens/OpenLens compatibility runtime. The delivery plan and its
decisions are tracked in [#163](https://github.com/srelens/srelens/issues/163).

## Supported contract

Extensions target the versioned srelens manifest and capability broker in
`crates/plugin-host`. The host declares a set of supported extension API versions and
serves each manifest under the highest one its `srelensApiVersion` range matches; the
versioning and compatibility rules are in the
[extension API specification](../extensions/specification.md). The desktop application renders declarative pages,
dashboards, resource details, navigation groups and actions with host components
in both classic and new designs. No third-party JavaScript executes in the host.
Native Flux and Argo CD manifests are the reference integrations.

The application installs declarative manifests from the catalog, where official
releases are signature-verified, or as local manifests after an explicit
permission review. There is no developer mode. IDs under `org.srelens.` are
reserved for signed srelens releases. See [Extensions](../EXTENSIONS.md) for
installation, examples, supported fields and current limitations. The executable
SDK, third-party publisher signing, key rotation and revocation described below
are future work, not shipped capabilities.

## Data and lifecycle

The backend owns installation, permissions, enabled state, revisions and settings.
It writes the inventory atomically under a cross-process lock. No extension
preference is persisted in browser storage. Every read validates the current
installation and revision; disable, removal and update revoke stale readers.
Already admitted reads may finish. Every load also re-verifies each app's
manifest and stored signature proof. An app that fails is quarantined and
disabled on its own, without affecting the others.

Views pin cluster identity in their routes and broker calls. Contributions use
host theme, density, navigation and searchable namespace controls. Failed reads
retain their error and retry action; they never become empty-cluster claims.

The application exposes the `extensions.*` capabilities through the shared
registry; install, configuration and GitOps actions are consent-gated.
The declarative broker inherits core capability annotations and binds fixed
resource selectors. Callers cannot override those selectors or obtain ambient
network, filesystem, process, kubeconfig or credential access.

The developer broker can register `plugin/<id>/<operation>` capabilities for MCP.
The app uses the revision-checked read facade; dynamic MCP discovery remains
separate lifecycle work. Multi-user web installations remain disabled until
per-user inventory and lifecycle isolation are implemented.

## Native platform roadmap

1. Keep the declarative manifest and host-rendered UI as the default authoring
   model. Prove additions through the Flux and Argo CD reference extensions.
2. Add resource workflows through explicit broker operations with inherited
   mutation annotations and confirmation; do not bypass host consent.
3. Design a native SDK for supervised, sandboxed JSON-RPC sidecars. Require
   quotas, cancellation and teardown; refuse executable extensions on unsupported
   sandbox backends. No renderer bridge is planned: contributions use host
   components. What each OS's sandbox can enforce is recorded under
   [Sandbox backends for executable extensions](#sandbox-backends-for-executable-extensions-proposed).
4. Extend signed distribution from official releases to third-party publishers,
   with key rotation, update verification, permission-diff consent and
   revocation. Unsigned local manifests stay outside reserved namespaces.

There is no Lens API shim, Node compatibility host or third-party package runtime.
Extensions must target the srelens contract. Retired archive inventory entries are
excluded on read and removed on the next successful inventory save, preserving
native installations and settings.

## Sandbox backends for executable extensions (Proposed)

Status: **Proposed**. These are the findings of the feasibility spike
[#571](https://github.com/srelens/srelens/issues/571), recorded 2026-09-24, for the
decisions still open in [#521](https://github.com/srelens/srelens/issues/521). Nothing
here ships. The supervisor that would use these backends is
[#572](https://github.com/srelens/srelens/issues/572).

The spike asked one question: can each desktop OS's own sandbox facility enforce the
restrictions an executable extension must run under, on ordinary operations? It is a
feasibility check, not a security review. It does not attempt escapes, and a backend
that passes here has not yet been hardened against a sidecar that tries to break out.
That review belongs to #572 (see [Follow-up work](#follow-up-work)).

### What was checked

A probe sidecar (`spikes/sidecar-sandbox/src/bin/probe.rs`) speaks newline-delimited
JSON-RPC 2.0 on stdin/stdout and performs one ordinary operation per request. The host
harness (`spikes/sidecar-sandbox/src/lib.rs`) starts it under one backend, and
`spikes/sidecar-sandbox/tests/checks.rs` asserts the outcome. The configured limits are
128 MiB of memory and 0.25 of a CPU.

| # | Check | Required outcome |
|---|---|---|
| 1 | Read a stand-in `~/.kube/config`, and a file outside the granted scratch directory | Denied |
| 2 | Write inside the scratch directory; write outside it | Allowed; denied |
| 3 | TCP connect to a listener on the host's loopback, TCP connect to `1.1.1.1:443`, resolve `example.com` | Denied |
| 4 | Start a child process (the probe re-executing itself) | Denied |
| 5 | Allocate and touch 512 MiB | Refused, or the sidecar is stopped; the host keeps running and can start another |
| 6 | Burn two threads for 3 s | Throttled to at most 1.5× the limit, or stopped |
| 7 | 50 JSON-RPC round trips over stdin/stdout | Works |

Every check first pings the sidecar. A denial counts only when the probe itself answers
that the operation failed, so a backend that stops the sidecar from starting, or crashes
it, cannot pass a "must be denied" check. That rule caught two false passes during the
spike: LPAC's network checks, where the probe had crashed (below), and a child process
that looked refused under Landlock because opening `/dev/null` for its stdio had been
denied.

With no sandbox, every "must be denied" check fails because the operation succeeds, on
both Windows and Linux. The recorded runs are in the pull request for #571.

### Support matrix

**Enforced** means the check passed. **Partial** means the restriction held, but not in
the way the check requires. **Not provided** means the facility does not cover it, and
the check failed. **Unverified** means it was not run.

Windows was run on Windows 11 Pro 10.0.26200 (x64, 24 logical processors), from an
unelevated process at medium integrity. Linux was run
in Docker Desktop 29.7.2 on the WSL2 kernel `6.6.87.2-microsoft-standard-WSL2`, which
reports Landlock ABI 3, with cgroup v2, Debian bookworm and bubblewrap 0.8.0. macOS was
researched only, not run.

| Backend | 1 Read | 2 Write | 3 Network and DNS | 4 Child process | 5 Memory | 6 CPU | 7 Stdio JSON-RPC |
|---|---|---|---|---|---|---|---|
| **Windows** AppContainer, no capabilities | Enforced (error 5) | Enforced | Enforced: loopback dropped (5 s timeout), internet `WSAEACCES`, DNS `WSAHOST_NOT_FOUND` | Not provided | Not provided | Not provided | Works |
| **Windows** Job Object | Not provided | Not provided | Not provided | Enforced (error 1816, active-process limit 1) | Enforced: allocation refused, sidecar keeps running | Enforced: 0.28 CPUs under a hard cap | Works |
| **Windows** AppContainer + Job Object | Enforced | Enforced | Enforced | Enforced | Enforced | Enforced: 0.25 CPUs | Works |
| **Windows** LPAC + Job Object | Enforced | Enforced | Partial: no connection is made, but `WSAStartup` fails (10107) and Rust's standard library panics, so the sidecar dies on its first socket call | Enforced | Enforced | Enforced | Works |
| **Linux** Landlock (ABI 3) | Enforced (`EACCES`) | Enforced | Not provided: TCP rules need ABI 4 and this kernel has 3; UDP, and so DNS, needs ABI 10 | Not provided | Not provided | Not provided | Works |
| **Linux** seccomp filter | Not provided | Not provided | Enforced (`EPERM`; DNS fails) | Enforced (`EPERM`) | Not provided | Not provided | Works |
| **Linux** cgroup v2 | Not provided | Not provided | Not provided | Not provided | Enforced: OOM-killed (`SIGKILL`, `oom_kill 1`), host relaunches | Enforced: 0.25 CPUs | Works |
| **Linux** Landlock + seccomp + cgroup v2 | Enforced | Enforced | Enforced | Enforced | Enforced | Enforced: 0.26 CPUs | Works |
| **Linux** bubblewrap, all namespaces unshared | Enforced (paths not mounted, `ENOENT`) | Enforced | Enforced (network namespace) | Not provided | Not provided | Not provided | Works |
| **Linux** Landlock with TCP rules (ABI ≥ 4, kernel ≥ 6.7) | Unverified | Unverified | Unverified | — | — | — | Unverified |
| **macOS** Seatbelt profile (`sandbox-exec` or `sandbox_init_with_parameters`) | Unverified | Unverified | Unverified | Unverified | Not provided (research) | Not provided (research) | Unverified |
| **macOS** App Sandbox helper | Unverified | Unverified | Unverified | Unverified | Not provided (research) | Not provided (research) | Unverified |

The Landlock-only and seccomp-only rows gave the same results in an unprivileged
container (Docker's default security settings, no `--security-opt`) as in a
privileged one. `--privileged` was needed only for a writable `/sys/fs/cgroup` and for
the user namespaces bubblewrap creates.

### How each backend was built

**Windows.** `CreateAppContainerProfile` registers the container for the current user
and returns its SID. A SID from `DeriveAppContainerSidFromAppContainerName` alone was
refused by `CreateProcessW` with `ERROR_FILE_NOT_FOUND`, so a supervisor must register a
profile and delete it (`DeleteAppContainerProfile`) on uninstall. The container is given
no capabilities, so it has no `internetClient`. Its SID is granted read and execute on
the sidecar binary and full control of the scratch directory. It has no other grants.
`CreateProcessW` takes `STARTUPINFOEXW` with three attributes:
`PROC_THREAD_ATTRIBUTE_SECURITY_CAPABILITIES`, `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (only
the two stdio pipe ends are inherited) and `PROC_THREAD_ATTRIBUTE_JOB_LIST`. With the
job list, the process is inside the job from its first instruction, with no window in
which it runs unlimited. The job sets `JOB_OBJECT_LIMIT_ACTIVE_PROCESS` = 1,
`JOB_OBJECT_LIMIT_PROCESS_MEMORY`, `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` and a hard
`JOBOBJECT_CPU_RATE_CONTROL_INFORMATION` cap. The cap is a share of the whole machine,
so a limit in CPUs is divided by the processor count. Inherited anonymous pipes worked
across the AppContainer boundary. The code is in `spikes/sidecar-sandbox/src/windows.rs`.

**Linux.** A small launcher, part of the host (`spikes/sidecar-sandbox/src/bin/sandbox-launch.rs`),
confines itself and then `exec`s the sidecar, which inherits every layer. It joins a
cgroup the host created with `memory.max`, `memory.swap.max = 0` and `cpu.max`. It
applies a Landlock ruleset that handles every right the kernel's ABI knows, in
best-effort mode. The ruleset grants the scratch directory, read and execute on the
sidecar binary, read and execute under `/usr` for the dynamic loader and libc, and read
of the few `/etc` files libc's resolver opens. It then installs a seccomp filter that
fails `socket` for any family but `AF_UNIX`, `io_uring_setup`, `fork`, `vfork` and
`clone` without `CLONE_THREAD` with `EPERM`, and `clone3` with `ENOSYS` so that libc
falls back to an inspectable `clone`. Threads keep working and new processes do not.
The code is in `spikes/sidecar-sandbox/src/linux.rs`.

**macOS (research only).**

- Seatbelt profiles in SBPL can deny file reads and writes by path, `network-outbound`,
  `process-fork` and `process-exec`, so checks 1 to 4 look expressible. But
  `sandbox-exec` is documented as deprecated, and its man page points to the App
  Sandbox instead ([sandbox-exec(1)](https://manp.gs/mac/1/sandbox-exec)).
  `sandbox_init_with_parameters` is private and undocumented, though Chromium applies
  its deny-by-default helper profiles with it
  ([Chromium Mac sandbox design](https://chromium.googlesource.com/chromium/src/+/HEAD/sandbox/mac/seatbelt_sandbox_design.md)).
- The App Sandbox does not fit a third-party sidecar started by srelens. A helper tool
  run by a sandboxed app inherits that app's sandbox through
  `com.apple.security.inherit`
  ([Apple: Embedding a command-line tool in a sandboxed app](https://developer.apple.com/documentation/xcode/embedding-a-helper-tool-in-a-sandboxed-app)),
  and "a process is not allowed to change its sandbox"
  ([Apple Developer Forums: Resolving App Sandbox Inheritance Problems](https://developer.apple.com/forums/thread/706390)).
  srelens itself is not sandboxed, because it reads kubeconfigs and talks to clusters.
  Its child therefore has no sandbox to inherit. A sidecar with its own App Sandbox
  would have to be signed with its own entitlements and started some other way than as
  srelens's child. Whether that can work for arbitrary third-party binaries is open.
- Memory has no enforced per-process limit. `RLIMIT_AS` and `RLIMIT_DATA` are accepted
  and then not applied ([report](https://github.com/MaximumTrainer/llm-cad/issues/38)),
  and there is no cgroup equivalent. The fallback is for the supervisor to watch the
  sidecar's footprint and kill it, which is detection after the fact, not a limit.
- CPU has no hard cap. `RLIMIT_CPU` stops a process after a total amount of CPU time.
  `taskpolicy` QoS clamps deprioritise a process but do not bound it
  ([taskpolicy](https://ss64.com/mac/taskpolicy.html)). Duty-cycling with
  `SIGSTOP`/`SIGCONT` is the only throttle.

### Recommended implementation per OS

- **Windows: AppContainer + Job Object**, both applied at `CreateProcessW` through the
  attribute list, as in the spike. Use a plain AppContainer, not LPAC: LPAC denied
  nothing more on the seven checks, and it turns a denied socket into a crash in any
  sidecar built with Rust's standard library. Register one profile per installed
  extension and delete it on uninstall. Grant only that extension's own data directory.
- **Linux: Landlock + seccomp + cgroup v2**, applied by a trusted launcher before
  `exec`. Each layer covers what the others do not, and all three were needed to pass.
  Treat seccomp as mandatory even on Landlock ABI 4 or later: Landlock's TCP rules do
  not cover UDP, and so do not cover DNS, before ABI 10. Refuse executables when the
  kernel has no Landlock, when a seccomp filter cannot be installed, or when there is no
  delegated cgroup. The ADR's rule applies to each layer, not just to the first.
  bubblewrap is not recommended as the primary backend. It gave filesystem and network
  isolation, but not process, memory or CPU limits, so seccomp and cgroups are still
  needed beside it. It also depends on unprivileged user namespaces, which some
  distributions restrict (not tested here).
- **macOS: no backend yet.** Under the ADR's rule, executables are refused on macOS
  until a Mac follow-up verifies a Seatbelt profile for checks 1 to 4 and 7, and a
  memory and CPU strategy is accepted. None exists today that enforces a limit.

### Recommendation on the SDK milestone (for decision)

This is a recommendation, not a decision; #521 owns the decision.

**Recommended:** the executable SDK milestone starts on **Windows and Linux**, with
macOS refusing executable extensions under the ADR rule. Both Windows and Linux passed
all seven checks with facilities that are documented, need no administrator rights once
cgroups are delegated, and were observed working. macOS cannot currently enforce the
memory or CPU limits the #521 exit criteria require ("Runs under CPU and memory
limits"). Its filesystem and network backend rests on a deprecated tool or a private
API, and none of it was run.

**The alternative** is to hold the milestone until macOS has a verified backend. That
keeps the platform uniform, but it puts every executable extension behind the item with
the most unknowns, and the exit criteria may be unreachable there without relaxing the
limits requirement.

### What the spike did not establish

- **Real desktops.** Linux ran in a privileged Docker container as root. A desktop
  supervisor runs as the user and needs a cgroup subtree that systemd delegates to it
  (for example a `systemd-run --user --scope` unit). Which controllers are delegated,
  and whether `cpu` is among them on the distributions srelens supports, was not
  checked.
- **Landlock TCP rules** (ABI 4, kernel 6.7 and later) could not be exercised. Docker
  Desktop's WSL2 kernel has ABI 3.
- **Architectures.** Only x86-64 was run. The seccomp filter names `fork` and `vfork`,
  which do not exist on arm64, and it was not built for arm64. Windows on Arm was not
  run.
- **Unix sockets.** The seccomp filter here still allows `AF_UNIX`, and Landlock below
  ABI 9 does not restrict connecting to a pathname socket. A sidecar may therefore
  reach host sockets such as the D-Bus session bus. This is outside the seven checks
  and was not tested. The sidecar's stdio is pipes, so a production filter can deny
  `AF_UNIX` too.
- **Memory-limit behaviour differs by OS.** On Windows the allocation fails and the
  sidecar lives on. On Linux the kernel kills the sidecar. SDKs and the supervisor must
  handle both.
- **The seccomp filter is a deny-list.** A production filter should be an allow-list.

### Follow-up work

- **#572: an escape-hardening security review of the real supervisor.** The spike tried
  only ordinary operations. Before release, the supervisor needs adversarial review of
  at least: handle and file-descriptor inheritance, symlink and hard-link tricks inside
  the scratch directory, Unix and abstract sockets, `io_uring` and other syscall
  surface the deny-list misses, signals and `ptrace` against sibling processes, named
  objects and other IPC reachable from an AppContainer, and the launcher's own window
  before its layers are applied.
- **#572: a sandbox conformance suite.** `spikes/sidecar-sandbox/tests/checks.rs` is
  meant to seed it: the same seven checks, run against each production backend in CI on
  Windows and Linux runners.
- **macOS, on a Mac.** Add a `seatbelt` backend to the spike and run the checks. Write a
  deny-by-default SBPL profile that allows the dyld shared cache and system libraries,
  reads and writes in the scratch directory only, `process-exec` of the sidecar only,
  and denies `network*` and `process-fork`. Confirm DNS is refused, which includes the
  `mach-lookup` path to mDNSResponder. Check that `sandbox-exec` still exists on the
  current macOS, and decide whether `sandbox_init_with_parameters` is acceptable to
  depend on. Measure whether `RLIMIT_AS` or `RLIMIT_DATA` limit anything (expected:
  no). Measure a supervisor that polls the footprint and kills, `RLIMIT_CPU`, and
  `SIGSTOP`/`SIGCONT` duty-cycling. Determine whether a separately signed,
  App-Sandboxed third-party helper can be started other than as srelens's child. Run on
  both Apple silicon and Intel.
- **Linux on a desktop.** Run the checks as an ordinary user on current Ubuntu and
  Fedora with a kernel of 6.7 or later. Cover the Landlock TCP rules, a
  systemd-delegated cgroup and unprivileged user namespaces (for bubblewrap).
- **Windows.** Run on Windows 10 (the job-list attribute needs Windows 10 or later), on
  Arm64, and under enterprise policy.

### Open questions

- Should a missing limit refuse the extension, or allow it with a warning? The ADR
  refuses only when no sandbox backend exists at all.
- One AppContainer profile per extension, or one per install? Where is the profile
  deleted if srelens is uninstalled with extensions still installed?
- Can the host-side broker callbacks (#573) stay on stdio, so that no backend has to
  open even loopback networking? Loopback is denied under the recommended Windows and
  Linux backends.
- What is the Landlock ABI floor? ABI 1 already covers checks 1 and 2 when seccomp
  covers the network.

### Running the spike

It is its own Cargo workspace, like `fuzz/`, so the root workspace and CI do not build
it.

```bash
cd spikes/sidecar-sandbox
SPIKE_BACKEND=none cargo test -- --nocapture --test-threads=1   # the unsandboxed baseline: nine checks fail
cargo test                                                     # this OS's recommended backend
cargo run --bin cleanup                                        # Windows: delete the AppContainer profile
```

Linux backends, from the repository root:

```bash
docker run --rm --privileged -v "$PWD/spikes/sidecar-sandbox:/spike:ro" rust:1-bookworm sh /spike/run-linux.sh
```

`SPIKE_BACKEND` takes `none`, `appcontainer`, `job`, `appcontainer+job` and `lpac+job`
on Windows, and `none`, `landlock`, `seccomp`, `cgroup`, `landlock+seccomp+cgroup` and
`bwrap` on Linux.
