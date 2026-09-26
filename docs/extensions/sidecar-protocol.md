# Sidecar protocol and supervisor

An executable app runs as a **sidecar**: a separate process that srelens starts inside
the operating system's sandbox, talks to over its stdin and stdout, and restarts when
it crashes. This page is the contract between srelens and a sidecar, and what the
supervisor does to keep one running.

Tracking: [#572](https://github.com/srelens/srelens/issues/572), part of
[#521](https://github.com/srelens/srelens/issues/521). The code is
`crates/plugin-host/src/sidecar/`.

**Status: nothing starts a sidecar yet.** The manifest has no executable kind: it still
accepts only `declarative`, and the unsigned-app policy for executables is to be
enforced when the kind is added ([specification.md](specification.md#unsigned-app-policy)).
Registering a sidecar's operations and wiring it into the app is
[#574](https://github.com/srelens/srelens/issues/574). What is not built is listed under
[Not yet](#not-yet).

## The wire

JSON-RPC 2.0, one message per line.

- **Framing.** Each message is one line of UTF-8 JSON ending in `\n`. `\r\n` is accepted.
  A line is at most 4 MiB (`MAX_MESSAGE_BYTES`). Batches are not part of the protocol.
- **stdout is protocol only.** Anything on it that is not a well-formed message stops the
  sidecar (see [Protocol violations](#protocol-violations)). **stderr is the log.**
- **Ids.** srelens numbers its requests 1, 2, 3 and so on. A sidecar may use any string or
  number as the id of a call it makes to srelens.
- **Params** are an object or an array, or absent.

## Lifecycle

```text
srelens                                   sidecar
   │ initialize {apiVersions, host, limits} ─▶
   │ ◀─ {apiVersion, sidecar}
   │ activate {} ─────────────────────────────▶
   │ ◀─ {}
   │        … requests, streams, health …
   │ deactivate {} ───────────────────────────▶
   │ ◀─ {}
   │ shutdown {} ─────────────────────────────▶
   │ ◀─ {}                               (exits)
   │ stdin closed
```

| Method | Direction | Params | Result |
|---|---|---|---|
| `initialize` | host → sidecar, first | `apiVersions`: every sidecar API version srelens speaks; `host`: `{name, version}`; `limits`: `{requestTimeoutMs, maxConcurrentRequests, maxStreams, memoryBytes, cpus}` | `apiVersion`: the one the sidecar chose; `sidecar`: `{name, version}`, optional |
| `activate` | host → sidecar, after `initialize` | `{}` | `{}` |
| `health` | host → sidecar, every 30 s | `{}` | `{}` |
| `deactivate` | host → sidecar, when stopping | `{}` | `{}` |
| `shutdown` | host → sidecar, last | `{}` | `{}`, then exit |

No app request is sent before `activate` has been answered, and none after
`deactivate`. A sidecar should also exit when its stdin closes: on Linux and macOS
that is how it learns srelens has gone.

### Version negotiation

The sidecar API has its own versions, listed in `SIDECAR_API_VERSIONS`
(`crates/plugin-host/src/sidecar/protocol.rs`). Today there is one, `0.1.0`. It is not
the extension API version (`SUPPORTED_API_VERSIONS`), because no manifest kind runs a
sidecar yet. Whether the two merge when the executable kind lands is open (#574).

`initialize` offers every version srelens speaks, and the sidecar answers with the one it
chose:

- **A version srelens offered:** the handshake continues under it.
- **One it did not offer, or none:** refused, with both sets named.
- **None in common:** the sidecar answers `initialize` with error `-32001` and, in
  `data.supported`, the versions it does speak. Also refused. The message names both
  sets and says to update the extension or srelens.

A refusal is not a crash. Starting the sidecar again would fail the same way, so the
supervisor does not retry (see [States](#states)).

## Requests

An app's request to its sidecar is an ordinary JSON-RPC request. The limits are host
policy (`Limits` in `crates/plugin-host/src/sidecar/limits.rs`):

| Limit | Default | When it is reached |
|---|---|---|
| Timeout | 30 s | The caller is told "The extension did not answer `scan` within 30 s", and the request is cancelled at the sidecar. |
| Requests in flight | 8 | The next one is refused at once, not queued: "The extension already has 8 requests in flight…". |
| Open streams | 5 | The next open is refused at once: "The extension already has 5 open streams…". |
| Memory | 256 MiB | Enforced by the sandbox backend (see [Sandbox](#sandbox)). |
| CPU | 1 CPU | Enforced like the memory. #572 names no default; this one is the supervisor's. |

- **Cancellation.** A caller that stops waiting cancels its request: srelens sends the
  notification `$/cancelRequest` `{"id": n}` and frees the request's place at once. The
  sidecar should stop work on it. An answer that arrives afterwards is dropped quietly;
  an answer to an id srelens never sent is a protocol violation.
- **Reserved methods.** An app request may not name a lifecycle method or anything under
  `$/` or `stream/`. Otherwise an app-facing path that forwarded method names could
  shut the sidecar down or forge stream frames.
- **Lifecycle calls are outside the limit** on requests in flight, so `health` and
  `shutdown` get through when every app request is stuck.

## Streams

For results that arrive over time. They follow the host's own stream rule
([streams.md](streams.md#frames)): any number of data frames, then exactly one terminal
frame.

| Message | Direction | Params |
|---|---|---|
| `stream/open` (request) | host → sidecar | `stream`: the id srelens chose; `method`, `params`: what to stream |
| `stream/data` | sidecar → host | `stream`, `data` |
| `stream/close` | sidecar → host | `stream`: it ended |
| `stream/error` | sidecar → host | `stream`, `message`: it failed |
| `stream/cancel` | host → sidecar | `stream`: stop sending it |

- The stream counts against the limit from its open until its terminal frame, or until
  srelens cancels it.
- A frame for a stream srelens never opened is a protocol violation. A frame for one that
  already ended or was cancelled is dropped.
- A stream may get up to 64 frames ahead of its reader. Past that, srelens stops it
  (`stream/cancel`) and its reader is told "The extension sent stream data faster than
  srelens read it". It is not buffered without bound.
- When the sidecar exits, every open stream fails with the reason.

## Calls from the sidecar

A sidecar may send requests to srelens. **Until the broker exists
([#573](https://github.com/srelens/srelens/issues/573)), every one is refused** with
`-32601` and "srelens does not take calls from sidecars yet". They stay on stdio, so no
sandbox has to open any network, loopback included. The ADR asks whether they can
("Open questions").

## Errors

| Code | Meaning |
|---|---|
| `-32700`, `-32600`, `-32601`, `-32602`, `-32603` | JSON-RPC 2.0's own |
| `-32001` | `initialize`: no API version in common |
| `-32800` | the answer to a request srelens cancelled (LSP's value). srelens ignores it. |

## Supervision

### States

`SidecarStatus` in `crates/plugin-host/src/sidecar/supervisor.rs`:

| State | Meaning | Leaves it |
|---|---|---|
| `Starting` | Being launched, initialized and activated. | on its own |
| `Running` | Serving requests under the negotiated API version. | on its own |
| `Restarting` | It exited unexpectedly; it is started again after the backoff delay. | on its own |
| `Disabled` | It exited unexpectedly once more than the backoff allows. | Restart |
| `Refused` | It cannot run here: no sandbox for this OS or a layer the machine lacks, limits nothing enforces, or no API version in common. | Restart |
| `Stopping`, `Stopped` | srelens stopped it. | Restart |

A request while the sidecar is not `Running` is refused at once with the state in words,
for example "The extension stopped unexpectedly and is restarting (attempt 2 of 3, in
5 s)".

### Restart backoff

An **unexpected exit** is any of these:

- the process crashes or exits on its own;
- it breaks the protocol;
- it fails a health check: no answer within 10 s, or an error;
- it does not come up: `initialize` or `activate` is refused or not answered in time.

After the first, srelens waits **1 s** and starts it again; after the second, **5 s**;
after the third, **30 s**. The fourth disables it:

> **Extension process exited unexpectedly**
>
> Restart · View logs · Disable

The last exit's reason goes with it, for example "it was killed by signal 6 (SIGABRT)",
or on Linux "it was stopped at its 256 MiB memory limit". A sidecar that ran for at least
**10 minutes** before exiting starts the sequence again at 1 s, so an occasional crash in
a long-lived sidecar does not add up to a disable. The 10 minutes are the supervisor's
choice; #572 does not name one. **Restart** starts it with a fresh sequence. **Disable**
is the host's: it stops the supervisor and disables the app in the inventory.

A crash never reaches srelens. Everything a sidecar does arrives as bytes on a pipe or as
an exit status. Nothing it writes can panic the host, no request waits past its deadline,
and when the process ends every caller still waiting is answered with the reason at once.

### Protocol violations

The supervisor stops a sidecar that writes a line that is not JSON, or not UTF-8, or is
longer than 4 MiB. It does the same for a message without `"jsonrpc": "2.0"`, for a
response with both or neither of `result` and `error`, for an answer to a request never
sent, for a stream frame for a stream never opened, and when the sidecar closes stdout
while still running. Once the framing is lost, no later answer can be trusted to belong
to its request. The stop counts as an unexpected exit.

### Stopping

`stop` sends `deactivate` and `shutdown`, 5 s each, closes stdin, and gives the process 5 s
to exit before killing it. Dropping the supervisor kills the sidecar at once.

### Logs

The last 1,000 lines of each sidecar's stderr are kept, each cut at 4 KiB, beside what the
supervisor did: each start, exit, restart and refusal. That is what **View logs** shows.
Per-app log storage, levels and the Inspector are
[#575](https://github.com/srelens/srelens/issues/575).

## Sandbox

The backends are the ones the #571 spike chose ([ADR](../design/plugin-architecture.md#sandbox-backends-for-executable-extensions)),
in `crates/plugin-host/src/sidecar/sandbox/`:

| OS | Isolation | Memory and CPU |
|---|---|---|
| Linux | Landlock and a seccomp filter, applied by `srelens-sandbox-launch` before it runs the sidecar | a cgroup v2 directory under a root delegated to srelens |
| Windows | an AppContainer with no capabilities, one profile per app | the Job Object the process starts in |
| macOS | Seatbelt through `/usr/bin/sandbox-exec` | **none yet: every sidecar is refused** until the host-side watchdog ([#713](https://github.com/srelens/srelens/issues/713)) exists |
| any other OS | — | — |

A sidecar is **refused, never started unconfined**:

- on an OS with no backend;
- on macOS, until #713;
- on Linux without Landlock, without the launcher, or without a delegated cgroup;
- anywhere the backend cannot set a limit.

In every case the refusal names what is missing. Whether a Linux or Windows machine that
lacks only a limit layer should instead run the sidecar with a warning is still open (ADR,
"Open questions"). Until that is decided, the supervisor refuses.

What the sidecar gets:

- **One writable directory**, which is also its working directory. The per-app,
  size-limited data directory is #573's; the supervisor takes the directory it is given.
- **Only the environment srelens names.** The host's own is never inherited: it may hold
  `KUBECONFIG`, cloud credentials or tokens. On Windows `SystemRoot` is added, which
  Winsock needs.
- **stdin, stdout and stderr, and nothing else.** On Linux and macOS the launcher closes
  every other inherited descriptor before the sidecar runs; on Windows only the three pipe
  ends are inherited.
- On Linux, read access to `/usr`, `/lib` and `/lib64` for the dynamic loader and libc.
  A sidecar that needs other shared libraries beside its binary is not supported.

The conformance suite `crates/plugin-host/tests/sandbox_conformance.rs` runs the spike's
checks against these backends. The `sandbox-conformance` CI job runs it on Linux and
Windows, and it runs on macOS by hand (the file's header says how).

## Not yet

| What | Where |
|---|---|
| A manifest kind that runs a sidecar, and registering its operations as capabilities and MCP tools | [#574](https://github.com/srelens/srelens/issues/574) |
| Broker callbacks, and the per-app data directory | [#573](https://github.com/srelens/srelens/issues/573) |
| Per-app logs, the Inspector, runtime metrics | [#575](https://github.com/srelens/srelens/issues/575) |
| JSON Schema for these messages, and the Rust and Go SDKs | [#576](https://github.com/srelens/srelens/issues/576) |
| Memory and CPU limits on macOS | [#713](https://github.com/srelens/srelens/issues/713) |
| The escape-hardening review of the supervisor and its backends, which the ADR assigns to #572 | not done ([ADR](../design/plugin-architecture.md#follow-up-work)) |
