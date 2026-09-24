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
   quotas, cancellation and teardown. Refuse executable extensions wherever no
   sandbox backend exists for isolation, which is the filesystem, network and process
   restrictions. Windows, Linux and macOS each have one. Memory and CPU quotas are
   kernel-enforced on Windows and Linux, and host-enforced by the supervisor on macOS,
   a weaker guarantee accepted for macOS only (see
   [the macOS decision](#decision-macos-limits-are-host-enforced)). No renderer bridge
   is planned: contributions use host components. What each OS's sandbox can enforce
   is recorded under
   [Sandbox backends for executable extensions](#sandbox-backends-for-executable-extensions).
4. Extend signed distribution from official releases to third-party publishers,
   with key rotation, update verification, permission-diff consent and
   revocation. Unsigned local manifests stay outside reserved namespaces.

There is no Lens API shim, Node compatibility host or third-party package runtime.
Extensions must target the srelens contract. Retired archive inventory entries are
excluded on read and removed on the next successful inventory save, preserving
native installations and settings.

## Sandbox backends for executable extensions

Status:

- **Accepted, 2026-09-24:**
  [the macOS decision](#decision-macos-limits-are-host-enforced). The executable SDK
  milestone includes macOS, with host-enforced limits there.
- **Proposed:** everything else here, from the feasibility spike
  [#571](https://github.com/srelens/srelens/issues/571) (recorded 2026-09-24). That
  covers the backend per OS and the open questions.

Nothing here ships yet. The supervisor that will use these backends is
[#572](https://github.com/srelens/srelens/issues/572), under
[#521](https://github.com/srelens/srelens/issues/521).

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

A "must be denied" check passes only when three things hold:

1. **The sidecar is alive.** Every check first pings it, and a denial counts only when
   the probe itself answers that the operation failed. A backend that stops the sidecar
   from starting, or crashes it, cannot pass.
2. **A host-side positive control succeeded.** Before asking the sidecar, the host does
   the same operation itself: it reads the same file, writes to the same directory,
   connects to the same address, resolves the same name, or starts the same program. If
   the host cannot do it either, a failure inside the sandbox says nothing about the
   sandbox. The check then fails as INCONCLUSIVE and never passes.
3. **The error is a sandbox's refusal for that operation** (`Denial::accepts` in
   `spikes/sidecar-sandbox/src/lib.rs`), not just any error:
   - a file refusal is `PermissionDenied`, `NotFound` or `ReadOnlyFilesystem`;
   - a connection refusal is `PermissionDenied`, `TimedOut`, `ConnectionRefused` or an
     unreachable network or host;
   - a DNS refusal is a resolver failure. It has no distinctive code on either OS, so the
     control carries the weight;
   - a process refusal is `EPERM` on Unix and error 1816 (the Job Object's process
     quota) on Windows. An `EACCES` from a filesystem layer does not count;
   - a memory refusal is the allocation failing.

   `ConnectionRefused`, `TimedOut` and "unreachable" count only because the host's own
   connection to the same address succeeded moments before.

These rules caught three false passes:

- **LPAC's network checks.** The probe had crashed (below).
- **A child process under Landlock.** It looked refused because opening `/dev/null` for
  the child's stdio had been denied.
- **All three network checks with no network**, found in review. Without the positive
  controls, all three passed with no sandbox at all when the network was unavailable.
  The run that showed it used `SPIKE_BACKEND=none` under `unshare --net`, and the
  unreachable network looked like a denial. With the controls, the same run fails all
  three as INCONCLUSIVE.

The recorded runs used for the matrix were re-run under the controls and the error-kind
rule. No cell changed. Every enforced network cell had a working host network behind it,
and no enforced cell was actually inconclusive.

With no sandbox, every "must be denied" check fails because the operation succeeds, on
both Windows and Linux. The recorded runs are in the pull request for #571.

### Support matrix

**Enforced** means the check passed. **Partial** means the restriction held, but not in
the way the check requires. **Not provided** means the facility does not cover it, and
the check failed. **Unverified** means it was not run.

Windows was run on Windows 11 Pro 10.0.26200 (x64, 24 logical processors), from an
unelevated process at medium integrity. Linux was run
in Docker Desktop 29.7.2 on the WSL2 kernel `6.6.87.2-microsoft-standard-WSL2`, which
reports Landlock ABI 3, with cgroup v2, Debian bookworm and bubblewrap 0.8.0. The
maintainer reproduced both Linux runs on the same kernel with the same results:

- **Privileged:** `landlock+seccomp+cgroup` 11/11. bubblewrap 8/11: files and network
  were denied through namespaces (`NotFound`, `ConnectionRefused`,
  `NetworkUnreachable`), with no process, memory or CPU limit. The cgroup backend
  OOM-killed the sidecar (`SIGKILL`, `oom_kill 1`) and held it to 0.26 CPUs.
- **Unprivileged:** Landlock 5/11 and seccomp 6/11, identical to the privileged runs.

macOS was run by the maintainer on macOS 27.0 (26A428), arm64, kernel 27.0.0, with
rustc 1.98.1. The first run, at `c7ff2875`, never got the probe started: all 11 tests
failed at the liveness ping with `SIGABRT`, and the only denial logged was
`deny(1) file-read-data /`. The profile was revised to allow what startup needed:

- `file-read-data` on `/` itself (a listing of the top-level names);
- `file-map-executable` for the probe and the system libraries;
- the system logging preferences.

The second run, at `02190671`, used `SEATBELT_TRACE=1` and exited 0:

- **Baseline:** 2 passed, 9 failed, as expected.
- **`seatbelt`, all checks:** 9 passed, 2 failed.
  - Checks 1 to 4 and 7 were enforced. The denials were `EPERM` for file reads and
    writes outside the scratch directory, for both TCP connects and for the child
    process, and a resolver failure for DNS.
  - Memory and CPU were not provided. `setrlimit` refused `RLIMIT_DATA` and `RLIMIT_AS`
    with `EINVAL`, so 512 MiB was allocated against the 128 MiB limit. The probe used
    1.99 CPUs against 0.25: `RLIMIT_CPU` was set, but it is a budget, not a rate.
- **`seatbelt`, claimed checks only:** 9/9.
- **The trace** showed the profile also denying operations that neither startup nor
  stdio needed, and that stay denied: `file-read-data /dev/dtracehelper`,
  `network-outbound /private/var/run/syslog`, `system-info net.link.addr`, `/etc/hosts`,
  the networkd preferences, and the `notification_center` and `logd` lookups. It showed
  the denials the checks rely on as well. DNS failed at
  `network-outbound /private/var/run/mDNSResponder`, the resolver's Unix socket, and not
  at a `mach-lookup` as the profile's first comments assumed.

| Backend | 1 Read | 2 Write | 3 Network and DNS | 4 Child process | 5 Memory | 6 CPU | 7 Stdio JSON-RPC |
|---|---|---|---|---|---|---|---|
| **Windows** AppContainer, no capabilities | Enforced (error 5) | Enforced | Enforced: loopback dropped (5 s timeout; the host's own connect succeeded), internet `WSAEACCES`, DNS `WSAHOST_NOT_FOUND` | Not provided | Not provided | Not provided | Works |
| **Windows** Job Object | Not provided | Not provided | Not provided | Enforced (error 1816, active-process limit 1) | Enforced: allocation refused, sidecar keeps running | Enforced: 0.28 CPUs under a hard cap | Works |
| **Windows** AppContainer + Job Object | Enforced | Enforced | Enforced | Enforced | Enforced | Enforced: 0.25 CPUs | Works |
| **Windows** LPAC + Job Object | Enforced | Enforced | Partial: no connection is made, but `WSAStartup` fails (10107) and Rust's standard library panics, so the sidecar dies on its first socket call | Enforced | Enforced | Enforced | Works |
| **Linux** Landlock (ABI 3) | Enforced (`EACCES`) | Enforced | Not provided: TCP rules need ABI 4 and this kernel has 3; UDP, and so DNS, needs ABI 10 | Not provided | Not provided | Not provided | Works |
| **Linux** seccomp filter | Not provided | Not provided | Enforced (`EPERM`; DNS fails) | Enforced (`EPERM`) | Not provided | Not provided | Works |
| **Linux** cgroup v2 | Not provided | Not provided | Not provided | Not provided | Enforced: OOM-killed (`SIGKILL`, `oom_kill 1`), host relaunches | Enforced: 0.25 CPUs | Works |
| **Linux** Landlock + seccomp + cgroup v2 | Enforced | Enforced | Enforced | Enforced | Enforced | Enforced: 0.26 CPUs | Works |
| **Linux** bubblewrap, all namespaces unshared | Enforced (paths not mounted, `ENOENT`) | Enforced | Enforced (network namespace) | Not provided | Not provided | Not provided | Works |
| **Linux** Landlock with TCP rules (ABI ≥ 4, kernel ≥ 6.7) | Unverified | Unverified | Unverified | — | — | — | Unverified |
| **macOS** `seatbelt`: `sandbox-exec` (deprecated) with `src/seatbelt.sb`, plus rlimits. Verified on macOS 27.0 arm64 only | Enforced (`EPERM`) | Enforced (`EPERM` outside, allowed inside) | Enforced: TCP `EPERM`, DNS resolver failure (the mDNSResponder socket is denied) | Enforced (`EPERM`) | **Not provided by the kernel (observed):** `setrlimit` refuses `RLIMIT_DATA` and `RLIMIT_AS` with `EINVAL`, and 512 MiB was allocated against a 128 MiB limit. Host-enforced by the planned watchdog (next row) | **Not provided by the kernel (observed):** 1.99 CPUs against 0.25. `RLIMIT_CPU` is accepted, but it is a lifetime budget, not a rate. Host-enforced by the planned watchdog (next row) | Works |
| **macOS** supervisor watchdog ([#713](https://github.com/srelens/srelens/issues/713), decided, not built) | — | — | — | — | Host-enforced, weaker than the kernel: bounds sustained use, while a burst can exceed the limit between samples. Not built or measured | Host-enforced, weaker than the kernel: throttles with `SIGSTOP`/`SIGCONT` or kills after a sample shows the overrun. Not built or measured | — |
| **macOS** `seatbelt` on Intel, or on macOS before 27 | Unverified | Unverified | Unverified | Unverified | Unverified | Unverified | Unverified |
| **macOS** App Sandbox helper | Not built (follow-up) | Not built | Not built | Not built | Not provided (research) | Not provided (research) | Not built |

The Landlock-only and seccomp-only rows gave the same results in an unprivileged
container (Docker's default security settings, no `--security-opt`) as in a
privileged one. `--privileged` was needed only for a writable `/sys/fs/cgroup` and for
the user namespaces bubblewrap creates.

### How each backend was built

**Windows.** `CreateAppContainerProfile` registers the container for the current user
and returns its SID. A SID from `DeriveAppContainerSidFromAppContainerName` alone was
refused by `CreateProcessW` with `ERROR_FILE_NOT_FOUND`, so a supervisor must register a
profile and delete it (`DeleteAppContainerProfile`) on uninstall. Concurrent
`CreateAppContainerProfile` calls for a profile that does not exist yet failed with
`0x8007000A`. That happened in three runs out of three when the parallel tests started
on a fresh profile, so the harness serializes creation, and a supervisor must too. The container is given
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
applies a Landlock ruleset that targets **ABI 5**: the filesystem rights of ABIs 1 to 5,
and TCP bind and connect (ABI 4). The ruleset is best-effort, so an older kernel
enforces the subset it knows. It does not handle what later ABIs added: ABI 6 scoping of
abstract Unix sockets and signals, ABI 9 `RESOLVE_UNIX` (connecting to a pathname Unix
socket), and ABI 10 UDP. The `landlock` crate it uses, 0.4.7, knows ABIs up to 9 and
has no UDP rights. The ruleset grants the scratch directory, read and execute on the
sidecar binary, read and execute under `/usr` for the dynamic loader and libc, and read
of the few `/etc` files libc's resolver opens. It then installs a seccomp filter that
fails `socket` for any family but `AF_UNIX`, `io_uring_setup`, `fork`, `vfork` and
`clone` without `CLONE_THREAD` with `EPERM`, and `clone3` with `ENOSYS` so that libc
falls back to an inspectable `clone`. Threads keep working and new processes do not.
The code is in `spikes/sidecar-sandbox/src/linux.rs`.

**macOS (verified on macOS 27.0 arm64).** The `seatbelt` backend starts the same launcher. The
launcher sets `RLIMIT_DATA` and `RLIMIT_AS` to the memory limit and `RLIMIT_CPU` to a
60-second budget, and prints whether macOS accepted each one. It then `exec`s
`/usr/bin/sandbox-exec -f seatbelt.sb -D PROBE=… -D SCRATCH=…` on the probe. The profile
is its own file, `spikes/sidecar-sandbox/src/seatbelt.sb`:

- `(deny default)`, allowing `process-exec` of the probe only, with no `process-fork`;
- reads of the probe, the system libraries and the dyld shared cache;
- `file-map-executable` for the same;
- `file-read-data` on `/` itself, which the first run showed startup needs;
- the system logging preferences;
- metadata reads everywhere, so that paths resolve;
- `sysctl-read`, and signals to itself;
- reads and writes under the scratch directory.

Everything else is denied: network (including the mDNSResponder socket, which is how
DNS was denied) and every `mach-lookup`. The paths are canonicalized first, because
Seatbelt matches resolved paths and `/var` and `/tmp` are symlinks on macOS. The profile
still depends on `sandbox-exec`, which Apple documents as deprecated.

The CPU budget is deliberately larger than check 6 can spend. `RLIMIT_CPU` kills a
process once a total is used up, which is not a rate limit. A small budget would pass
check 6 by stopping the sidecar, and that would report a guarantee macOS does not give.

`run-macos.sh` runs the baseline and `seatbelt`. Its exit status separates two
failures:

- **2:** the probe could not start under the profile, so nothing is known yet about
  Seatbelt. The script then prints the sandbox denials from that run, and the
  harness's own liveness failure carries the recent denials too.
- **1:** the probe ran and a claimed check (1 to 4, 7) failed.

`SEATBELT_TRACE=1`, which also runs automatically after an exit-2 run, runs the probe
once under the profile with `(deny default)` replaced by allow-and-report. The log then
lists, in one run, every operation the profile does not yet allow.
`test-run-macos.sh` tests the exit-status logic on any POSIX shell. Both Mac runs are
described above.

The research:

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
- Memory has no enforced per-process limit. `RLIMIT_AS` and `RLIMIT_DATA` are reported
  to be accepted and then not applied
  ([report](https://github.com/MaximumTrainer/llm-cad/issues/38)). On macOS 27.0 arm64
  it was worse, **as observed**: `setrlimit` refused both, at 128 MiB, with `EINVAL`.
  There is no cgroup equivalent. The fallback is for the supervisor to watch the
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
- **macOS: Seatbelt for isolation, plus a host-side watchdog for limits.** The
  `seatbelt` backend enforced checks 1 to 4 and 7 on macOS 27.0 arm64. It rests on the
  deprecated `sandbox-exec`, or on the private `sandbox_init_with_parameters`. No kernel
  facility found limits a process's memory or CPU rate: `setrlimit` refuses the memory
  limits, and `RLIMIT_CPU` is only a budget. Memory and CPU are therefore host-enforced,
  as the decision below records.

### Decision: macOS limits are host-enforced

Status: **Accepted**, 2026-09-24, by the maintainer
([#521](https://github.com/srelens/srelens/issues/521#issuecomment-5823655882)).

**The executable SDK milestone includes macOS.** On macOS only, the supervisor enforces
the memory and CPU limits with a host-side watchdog, documented as a weaker guarantee
than the kernel enforcement on Windows and Linux. The watchdog is
[#713](https://github.com/srelens/srelens/issues/713), a sub-issue of #521 that depends
on #572. It is not built yet.

**Why:**

- **Isolation holds on all three OSes.** Filesystem, network, DNS and process
  restrictions were verified on Windows (AppContainer), on Linux (Landlock and seccomp)
  and on macOS 27.0 arm64 (Seatbelt).
- **macOS lacks kernel resource limits.** It has nothing like a Job Object or a cgroup.
  `setrlimit` refuses the memory limits (observed), and `RLIMIT_CPU` is a lifetime
  budget, not a rate.

**The macOS guarantee, stated plainly:**

- **How it works.** The watchdog samples the sidecar's memory and CPU use (for example
  with `proc_pid_rusage`). Past a limit, it throttles the sidecar (`SIGSTOP`/`SIGCONT`)
  or kills it.
- **What it bounds.** *Sustained* use, not every instant.
- **What it does not bound.** A burst between two samples can exceed the limit,
  including an allocation large enough to take memory from the rest of the system
  before it is seen.
- **How it differs.** On Windows and Linux the kernel refuses or kills at the limit
  itself.
- **Exit criterion.** #521's "Runs under CPU and memory limits" is met on macOS in this
  weaker sense, and the product documentation must say so.

Considered and not chosen:

1. **Start on Windows and Linux and refuse executables on macOS.** The spike recommended
   this; it would have kept every platform at kernel enforcement, at the cost of no
   executables on macOS.
2. **Isolation only on macOS, with no limits.** Rejected: a sidecar could exhaust the
   machine's memory or CPU, which the SDK promises to prevent.

### What the spike did not establish

- **Real desktops.** Linux ran in a privileged Docker container as root. A desktop
  supervisor runs as the user and needs a cgroup subtree that systemd delegates to it
  (for example a `systemd-run --user --scope` unit). Which controllers are delegated,
  and whether `cpu` is among them on the distributions srelens supports, was not
  checked.
- **Landlock TCP rules** (ABI 4, kernel 6.7 and later) could not be exercised. Docker
  Desktop's WSL2 kernel has ABI 3.
- **Architectures.** Only x86-64 was run. The spike compiles for arm64 Linux. There,
  the seccomp filter leaves out `fork` and `vfork`, which arm64 does not have, and it has
  not been run. That is what Docker on an Apple-silicon Mac would run. Windows on Arm
  was not run.
- **Unix sockets.** The seccomp filter here still allows `AF_UNIX`, and the launcher's
  Landlock ruleset (ABI 5) does not handle ABI 9 `RESOLVE_UNIX`, whatever the kernel
  supports. A sidecar may therefore reach host sockets such as the D-Bus session bus.
  This is outside the seven checks and was not tested. The sidecar's stdio is pipes, so
  a production filter can deny `AF_UNIX` too.
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
- **macOS: more Macs.** Run
  `SEATBELT_TRACE=1 sh spikes/sidecar-sandbox/run-macos.sh` (see
  [the spike's README](../../spikes/sidecar-sandbox/README.md)) on an Intel Mac and on
  macOS versions before 27, which have not been run. Try narrowing the global
  metadata-read rule. Decide whether `sandbox_init_with_parameters` (private) is
  acceptable in place of the deprecated `sandbox-exec`.
- **macOS: the host-side watchdog** ([#713](https://github.com/srelens/srelens/issues/713),
  as decided above). Build the supervisor's sampler, and its throttle
  (`SIGSTOP`/`SIGCONT`) or kill past the limit. Measure how far a burst overshoots
  between samples, and document that bound for users.
- **macOS: an App Sandbox helper variant.** Not built: it needs code signing with
  entitlements. An ad-hoc signature (`codesign -s - --entitlements …`, no developer
  account) may be enough for a local test. It would still have to answer whether an
  App-Sandboxed third-party binary can be given a per-extension scratch directory, and
  whether it can be started other than as srelens's child. Re-signing a third-party
  binary also replaces its publisher's signature.
- **Linux on a desktop.** Run the checks as an ordinary user on current Ubuntu and
  Fedora with a kernel of 6.7 or later. Cover the Landlock TCP rules, a
  systemd-delegated cgroup and unprivileged user namespaces (for bubblewrap).
- **Windows.** Run on Windows 10 (the job-list attribute needs Windows 10 or later), on
  Arm64, and under enterprise policy.

### Open questions

- On Windows or Linux, should a missing limit layer refuse the extension or allow it with
  a warning? An example is a Linux desktop with no delegated cgroup. The macOS case is
  decided above; this one is not.
- One AppContainer profile per extension, or one per install? Where is the profile
  deleted if srelens is uninstalled with extensions still installed?
- Can the host-side broker callbacks (#573) stay on stdio, so that no backend has to
  open even loopback networking? Loopback is denied under the recommended Windows and
  Linux backends.
- What are the Landlock ABI floor and target? ABI 1 already covers checks 1 and 2 when
  seccomp covers the network. The spike targets ABI 5, which leaves two gaps that a
  newer target would close on newer kernels. Both tie to the Unix-socket item under
  [What the spike did not establish](#what-the-spike-did-not-establish).
  - **Unix sockets:** ABI 6 scopes abstract Unix sockets and signals, and ABI 9
    `RESOLVE_UNIX` restricts connecting to pathname sockets. Until then, seccomp must
    deny `AF_UNIX`.
  - **UDP:** ABI 10 adds UDP rights. The `landlock` crate used here has none, so DNS
    and other UDP rest on seccomp alone.

  Should the supervisor target the newest ABI its Landlock library knows, and require a
  minimum for a sidecar to run at all?

### Running the spike

It is its own Cargo workspace, like `fuzz/`, so the root workspace and CI do not build
it. [Its README](../../spikes/sidecar-sandbox/README.md) has the prerequisites, the
commands for Windows, for Linux in Docker and for macOS, how long each takes, and what
to send back. In short, from the repository root:

```bash
# Linux, in Docker (writes spikes/sidecar-sandbox/results/results-linux.txt)
docker run --rm --privileged -v "$PWD/spikes/sidecar-sandbox:/spike" rust:1-bookworm sh /spike/run-linux.sh
# macOS, with the trace (writes spikes/sidecar-sandbox/results/results-macos.txt)
SEATBELT_TRACE=1 sh spikes/sidecar-sandbox/run-macos.sh
# Windows or any OS: this OS's recommended or candidate backend
cd spikes/sidecar-sandbox && cargo test
```

On Linux outside Docker, `cargo test` defaults to `landlock+seccomp+cgroup`. Its cgroup
layer needs a writable, delegated cgroup v2 directory, which `SPIKE_CGROUP_ROOT` names.
An ordinary user has none under `/sys/fs/cgroup`, so without one the checks fail at
start, with a message saying so. `run-linux.sh` exits non-zero if
`landlock+seccomp+cgroup` fails. `run-macos.sh` exits 1 if `seatbelt` fails a check it
claims, and 2 if the probe could not start under the profile.

`SPIKE_BACKEND` takes:

- **Windows:** `none`, `appcontainer`, `job`, `appcontainer+job` and `lpac+job`;
- **Linux:** `none`, `landlock`, `seccomp`, `cgroup`, `landlock+seccomp+cgroup` and
  `bwrap`;
- **macOS:** `none` and `seatbelt`.
