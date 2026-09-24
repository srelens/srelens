# Sidecar sandbox spike (#571)

This is a feasibility check. It asks whether each desktop OS's own sandbox can enforce the
restrictions an executable extension must run under, on ordinary operations. It is a
throwaway prototype, not the supervisor (that is #572), and it attempts no escapes. The
findings and the support matrix are in the ADR:
[Sandbox backends for executable extensions](../../docs/design/plugin-architecture.md#sandbox-backends-for-executable-extensions-proposed).

It is its own Cargo workspace, like `fuzz/`, so the root workspace and CI never build it.

## What runs

A probe sidecar (`src/bin/probe.rs`) speaks JSON-RPC over stdin/stdout. The host harness
(`src/lib.rs`) starts it under one backend, which `SPIKE_BACKEND` names, and
`tests/checks.rs` asserts seven checks:

| # | Check | Required |
|---|---|---|
| 1 | Read a stand-in `~/.kube/config`, and a file outside the scratch directory | denied |
| 2 | Write inside the scratch directory; write outside it | allowed; denied |
| 3 | TCP to the host's loopback, TCP to `1.1.1.1:443`, resolve `example.com` | denied |
| 4 | Start a child process | denied |
| 5 | Allocate 512 MiB against a 128 MiB limit | refused or stopped, and the host survives |
| 6 | Burn two threads for 3 s against a 0.25-CPU limit | throttled or stopped |
| 7 | 50 JSON-RPC round trips | work |

A "must be denied" check passes only in this case:

- the sidecar was alive first,
- the host itself could do the same operation (the positive control), and
- the probe answered with an error a sandbox produces for that operation (`Denial` in
  `src/lib.rs`).

If the host cannot do it either (no network or no DNS, for instance), the check fails as
`INCONCLUSIVE`, never passes.

| OS | Backends (`SPIKE_BACKEND`) | Recommended or candidate |
|---|---|---|
| Windows | `none`, `appcontainer`, `job`, `appcontainer+job`, `lpac+job` | `appcontainer+job` |
| Linux | `none`, `landlock`, `seccomp`, `cgroup`, `landlock+seccomp+cgroup`, `bwrap` | `landlock+seccomp+cgroup` |
| macOS | `none`, `seatbelt` | `seatbelt` (checks 1 to 4 and 7 verified on macOS 27.0 arm64; no memory or CPU limit) |

`none` is the unsandboxed baseline: nine checks fail there because the operation succeeds.
That is expected.

## Running it

Every command below starts from the repository root. Each run writes a results file to
`spikes/sidecar-sandbox/results/`, which git ignores.

### Linux, in Docker

Prerequisite: Docker. This works from Linux, macOS (Docker Desktop) or Windows (Docker
Desktop with WSL2).

```sh
docker run --rm --privileged -v "$PWD/spikes/sidecar-sandbox:/spike" rust:1-bookworm sh /spike/run-linux.sh
docker run --rm -e UNPRIVILEGED=1 -v "$PWD/spikes/sidecar-sandbox:/spike" rust:1-bookworm sh /spike/run-linux.sh
```

The same lines work in PowerShell.

- **Time:** the first run pulls `rust:1-bookworm` (about 1 GB). After that, each command
  takes about 3 to 5 minutes, mostly the build.
- **`--privileged`** is used for two things only: writing `/sys/fs/cgroup` (the `cgroup`
  backends) and creating user namespaces (`bwrap`). The second command runs only `none`,
  `landlock` and `seccomp` without it, to show those two layers need no privilege.
- **Output:** `results/results-linux.txt` and `results/results-linux-unprivileged.txt`.
- **Exit status:** non-zero if `landlock+seccomp+cgroup` fails any check. The other
  backends' failures are the matrix, not failures.
- **The kernel is the Docker VM's, not your host's.** The first lines of the results file
  name it, and each Landlock run prints the kernel's Landlock ABI. On an Apple-silicon Mac
  the container is arm64, which this spike has never run.

To run it on a Linux machine without Docker, as an ordinary user, the `cgroup` backends
need a delegated cgroup v2 directory with the `memory` and `cpu` controllers enabled.
Point `SPIKE_CGROUP_ROOT` at it, for example inside
`systemd-run --user --scope -p Delegate=yes`. Without one, they fail at start with a
message saying so. The other backends need nothing extra:

```sh
cd spikes/sidecar-sandbox
SPIKE_BACKEND=landlock cargo test --test checks -- --nocapture --test-threads=1
```

### macOS

Prerequisite: Rust through [rustup](https://rustup.rs) and `/usr/bin/sandbox-exec`, which
macOS still ships though it is deprecated.

```sh
SEATBELT_TRACE=1 sh spikes/sidecar-sandbox/run-macos.sh
```

- **Time:** about 2 to 4 minutes, mostly the first build.
- **What it prints first:** the macOS version, CPU architecture and whether `sandbox-exec`
  is present.
- **What it runs:**
  - the `none` baseline;
  - `seatbelt` across all seven checks (the matrix);
  - `seatbelt` again on only the checks it claims (1, 2, 3, 4, 7), which decide the exit
    status. Checks 5 and 6 are recorded, not claimed. The launcher prints whether each
    `setrlimit` call was accepted. On the first run (macOS 27.0 arm64), `RLIMIT_DATA` and
    `RLIMIT_AS` were refused with `EINVAL`.
- **With `SEATBELT_TRACE=1`, a trace.** It runs the probe once under the profile with
  `(deny default)` replaced by allow-and-report, then lists every operation the sandbox
  reported, with counts. Those are the operations the profile does not yet allow, found
  in one run even if the probe cannot start under the real profile. The trace also runs
  automatically when the probe could not start. The command without `SEATBELT_TRACE=1`
  runs the checks only.
- **Exit status:**

  | Code | Meaning |
  |---|---|
  | 0 | Every claimed check passed. |
  | 1 | The probe ran and a claimed check failed. |
  | 2 | The probe could not start under the profile, so nothing is known about Seatbelt yet. The script prints `probe could not start under the profile` and the sandbox's denials from that run. |

  `sh spikes/sidecar-sandbox/test-run-macos.sh` tests this logic on any POSIX shell.
- **Output:** `results/results-macos.txt`.

The profile is `src/seatbelt.sb`. Its first run never let the probe start
(`deny(1) file-read-data /`, then `SIGABRT`). The revised profile passed checks 1 to 4
and 7 on macOS 27.0 arm64; memory and CPU are not provided. If another Mac or macOS
version needs a change, make the narrowest one the trace shows, and send the diff back
with the results.

### Windows

Prerequisite: Rust through rustup (MSVC toolchain). No administrator rights are needed.

```powershell
cd spikes/sidecar-sandbox
mkdir -Force results | Out-Null
& { foreach ($b in 'none','appcontainer','job','appcontainer+job','lpac+job') {
  "===== SPIKE_BACKEND=$b"; $env:SPIKE_BACKEND = $b
  cargo test --test checks -- --nocapture --test-threads=1 2>&1 | ForEach-Object { "$_" }
} } | Tee-Object results/results-windows.txt
Remove-Item Env:SPIKE_BACKEND
cargo test                     # the recommended backend only: all 11 must pass
cargo run --bin cleanup        # delete the AppContainer profile the harness registered
```

- **Time:** about 1 minute to build, then about 40 seconds for all five backends.

## What the output looks like

Each operation prints one line: the backend, the check, and what came back.

```text
===== SPIKE_BACKEND=landlock+seccomp+cgroup
[landlock+seccomp+cgroup] read ~/.kube/config: Refused(Failure { message: "Permission denied (os error 13)", kind: "PermissionDenied", os: Some(13) })
[landlock+seccomp+cgroup] TCP connect to 1.1.1.1:443: Refused(Failure { message: "Operation not permitted (os error 1)", kind: "PermissionDenied", os: Some(1) })
[landlock+seccomp+cgroup] allocate 512 MiB (limit 128 MiB): Stopped("exited: signal: 9 (SIGKILL); cgroup memory.events oom_kill 1")
[landlock+seccomp+cgroup] used 0.25 CPUs
test result: ok. 11 passed; 0 failed; ...
```

- `Ok(...)` means the operation worked.
- `Refused(...)` means the probe reported that it failed, with the error kind and OS code.
- `Stopped(...)` means the sidecar was killed or exited.
- `Garbled(...)` means it wrote something that was not a reply, such as a panic.
- `INCONCLUSIVE` means the host itself could not do the operation, so the result says
  nothing about the sandbox.

## What to send back

- The results file or files from `results/`.
- For macOS, also any change you made to `src/seatbelt.sb`.

The macOS column of the ADR's matrix is verified on macOS 27.0 arm64 only. A run on an
Intel Mac, or on an older macOS, is still wanted.
