# Sidecar protocol and supervisor

An executable app runs as a **sidecar**: a separate process that srelens starts inside
the operating system's sandbox, talks to over its stdin and stdout, and restarts when
it crashes. This page is the contract between srelens and a sidecar, and what the
supervisor does to keep one running.

Tracking: [#572](https://github.com/srelens/srelens/issues/572) (the protocol and the
supervisor) and [#573](https://github.com/srelens/srelens/issues/573) (calls back into the
host, and the data directory), part of [#521](https://github.com/srelens/srelens/issues/521).
The code is `crates/plugin-host/src/sidecar/`. The SDKs
([#576](https://github.com/srelens/srelens/issues/576)) wrap what this page specifies.

**Status.** An app of kind `executable` (API 0.6,
[#574](https://github.com/srelens/srelens/issues/574)) names its sidecar's binaries and the
operations it answers ([manifest.md](manifest.md#executable-apps)). srelens starts the
sidecar under this supervisor the first time one of those operations is called in a
process, and stops it when the app is disabled, updated or removed. It calls back into
srelens only through the [broker](#calls-from-the-sidecar), and writes only its
[data directory](#data-directory). Its log and its process show in the app's
Inspector ([#575](https://github.com/srelens/srelens/issues/575)). Each operation is an
MCP tool, `plugin/<id>/<operation>` ([MCP.md](../MCP.md#installed-apps-tools)). The
registry's side is `crates/registry/src/extensions/sidecars.rs`. What is not built is
listed under [Not yet](#not-yet).

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
| `initialize` | host → sidecar, first | `apiVersions`: every sidecar API version srelens speaks; `host`: `{name, version}`; `limits`: `{requestTimeoutMs, maxConcurrentRequests, maxStreams, memoryBytes, cpus, dataBytes, dataEntries}`; `dataDirectory`: the one path it may write (see [Data directory](#data-directory)) | `apiVersion`: the one the sidecar chose; `sidecar`: `{name, version}`, optional |
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
the extension API version (`SUPPORTED_API_VERSIONS`), and the executable kind (#574) kept
the two apart: a manifest names the extension API it is written for, and its sidecar
negotiates this one at `initialize`, so each can move without the other.

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

An app's request to its sidecar is an ordinary JSON-RPC request. Today srelens sends one
kind: a call of one of the operations the manifest declares, as a request named after the
operation, whose `params` is the call's input after srelens has held it to the
operation's declared inputs. A sidecar answers with any JSON result, which is what the
caller gets. The limits are host policy (`Limits` in
`crates/plugin-host/src/sidecar/limits.rs`):

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

A sidecar gets Kubernetes data, and data from outside the machine, the way a declarative
app does, and only that way: it asks srelens. It has no kubeconfig, no token, no network
and no filesystem beyond its [data directory](#data-directory), so there is nothing else
it could do. Its calls are requests on the same pipe: they stay on stdio, so no sandbox has
to open any network, loopback included (the ADR's open question, answered).

### The calls

Each is answered by the app facade the srelens UI itself calls, `extensions.read`,
`extensions.resource` and `extensions.action` (`crates/registry/src/extensions`), so a
sidecar is held to exactly what the app's own pages are. The broker
(`CapabilityBroker`, `crates/plugin-host/src/sidecar/broker.rs`) only builds their input.

| Method | Params, beside `context` | What answers it | Result |
|---|---|---|---|
| `host/read` | `capability`: one of the app's declared readers, or one of its `network.http` requests | `extensions.read` | what the page gets: the reader's rows, or `{status, contentType, body}` for a request |
| `host/resource` | `capability`: a declared custom-resource reader; `name` | `extensions.resource` | the object, with the actions declared for it |
| `host/action` | `capability`, `name`, `action`: one of the app's declared actions; `uid`, `resourceVersion`: the object's, as read | `extensions.action`, once a person has confirmed it | the primitive's answer |

Every param is a string, every one is required, and nothing else is accepted: a call
naming `id`, `revision`, a second `cluster` or `namespace`, or anything the table does not
list is refused with `-32602`. Each is held to its shape before anything runs, so nothing a
sidecar sends reaches a capability, a person or the audit trail unbounded: `capability` and
`action` are names as the manifest writes them (1 to 64 ASCII letters, digits and hyphens),
`name` is a Kubernetes object name (1 to 253 ASCII letters, digits, dots and hyphens), and
`uid` and `resourceVersion` are 1 to 128 visible ASCII characters. Any other method, a host capability named directly
(`k8s.getSecret`, `network.http`, `extensions.configure`) included, is refused with
`-32601`, naming the three above.

```json
{"jsonrpc": "2.0", "id": "c-1", "method": "host/read",
 "params": {"context": {"clusterId": "kind-dev", "namespace": "team"},
            "capability": "applications"}}
```

### Context: every call names its cluster

Every call carries `context`, and there is no current cluster to fall back on:

```text
"context": {"clusterId": "<cluster>", "namespace": "<namespace>" or null}
```

- **`clusterId`** names a cluster as srelens does: the `context` of the host request the
  sidecar is serving, a kubeconfig context's stable ID, pinned ID or name. It is resolved
  as the UI's is, and the call goes out under the pinned ID it resolved to. At most 4096
  bytes.
- **`namespace`** is a Kubernetes namespace name, or `null` for every namespace or a
  cluster-scoped kind. It must be present: "no namespace" is said, not assumed. An empty
  string is refused; send `null`.

A call without `context`, or with one that is not exactly that shape, is refused with
`-32602` before anything runs: "Every call names its cluster: … srelens has no current
cluster to assume".

### What srelens checks

On every call, in this order:

1. **Who is asking.** The app's ID and revision come from the supervisor that started the
   process, never from the sidecar. An app that was updated, disabled, removed or
   blocked by the unsigned-app policy is refused ("Extension was disabled, removed or
   updated"). The next operation call starts the new revision's sidecar in its place, and
   an announced inventory write stops the old one.
2. **Grants.** The binding must be one the app declares, and every permission it needs
   must be granted: the facade runs the install check (`validate_app`) again on each
   call, so an inventory edited by hand gains nothing. An undeclared binding is refused
   with `-32602` ("Scanner declares no capability `secrets`"); a missing grant with
   `-32003` ("permissions: k8s.annotate was not granted").
3. **The cluster.** An app limited to some clusters is refused on any other ("App is not
   enabled for this cluster").
4. **Confirmation.** A call the host gates, which is `host/action` today (`requires_confirm`
   or `destructive`, the same rule MCP's gate reads), is put to a person first, with the
   host's own sentence for it, its impact and the requesting app named. Before anyone is
   asked, the object is read through `extensions.resource`, which makes every check above
   and lists the actions the app declares for it: a person is asked only about one of those,
   on an object that exists, and any other is refused with `-32602` ("Scanner declares no
   action `delete` for `applications`"). Declined, or with no one to ask, it never runs:
   `-32002`, with the reason. This is the single host
   confirmation ([#552](https://github.com/srelens/srelens/issues/552)); the broker asks it
   through the `Consent` trait. The desktop app's MCP server provides one, its own
   confirmation prompt, which names the app from the window's inventory. Headless
   `--mcp-stdio` and `--mcp-http` provide `NoConsent`, so every gated call is refused
   there.
5. **Cluster RBAC.** The call reaches the cluster with the user's own credentials, so the
   cluster answers for itself. A refusal from it is `-32003`, with its words.

Should the facade ever carry a capability marked `sensitive`, such as a Secret read, the broker
refuses it to a sidecar, even with consent. None of the three is one: keeping Secrets out
rests on the facade's readers (below).

### Credentials never cross

Nothing srelens writes on the pipe carries a kubeconfig, a token or a secret's value:

- the sidecar's environment is only what srelens names (see [Sandbox](#sandbox));
- the facade answers only what the app declared, from the readers the manifest allows:
  custom resources, checked to be custom (#601), events, and summaries of built-in
  workloads and nodes. None of them reads a Secret or a kubeconfig;
- `network.http`'s secret headers are injected by srelens and never answered
  ([#568](https://github.com/srelens/srelens/issues/568)). Its answer is what the allowed
  host sent, which that host is trusted with (threat model, APP-2).

`crates/registry/src/extensions/sidecar_tests.rs` runs a supervised sidecar through every
kind of call, answered and refused, and checks everything srelens wrote to its stdin for the
kubeconfig's token, its client key and the app's stored API token. It does so against a
cluster that answers with what it was given, and again with the host's own kube client,
built from the token, failing to reach the cluster.

### Network

A sidecar has no network (see [Sandbox](#sandbox)). What it reaches outside the machine it
reaches through `host/read` of one of its `network.http` requests: a GET to one of the hosts
its manifest declares, with the limits and redirect rules of
[Network requests](manifest.md#network-requests).

### Audit

Every call that changes something or reads secret material is recorded in the local audit
trail ([#555](https://github.com/srelens/srelens/issues/555)) with `source` `app`,
`transport` `sidecar`, and the app's ID and revision, which srelens knows rather than reads
from the call. A declined confirmation is recorded as `denied`. Reads are not recorded, as
from the UI: a scanner reads far more than a screen does, and every read would bury the
write that answers "what did this app change?". A write that has started runs to its end on
a task of its own, so a sidecar that cancels or exits cannot leave one unrecorded; every other
call stops when the sidecar stops waiting for it.

### Cancellation and bounds

- **Cancelling.** A sidecar cancels its own call with the notification `$/cancelRequest`
  `{"id": <its id>}`. The call is answered at once with `-32800`, and whatever it was waiting
  on, a person's confirmation included, is dropped. A cancellation srelens has accepted
  is the answer even if the result was ready in the same moment. A write a person already
  approved still finishes (see above), so for one `-32800` means srelens stopped waiting,
  not that nothing ran: the audit trail has its outcome. A cancellation for a call already
  answered is ignored.
- **When the session ends**, and when the supervisor is dropped, every call still being
  answered is cancelled: no confirmation is left open for a process that is gone.
- **Ids.** A call may not reuse the id of a call still being answered; that is a protocol
  violation. An id is free again once its answer is written. `1` and `"1"` are different
  ids.
- **At most 16 answers wait at once** (`ANSWER_BUFFER`), counting those being worked out
  and those not yet written to the sidecar's stdin. A place is taken before any work
  starts. A sidecar that makes one more call than that without reading its answers is
  stopped (see [Protocol violations](#protocol-violations)). Otherwise, a sidecar that
  never reads its stdin could grow srelens's memory without limit, since its own limits
  bound only itself. srelens's own messages are written first, so answers never hold
  back a cancellation or a health check.
- **At most 8 calls are worked on at once** (the request limit); the next is answered at
  once with `-32603`, "srelens is already answering 8 calls from this extension". A write
  that a person approved and the sidecar then cancelled finishes outside that count, each
  one bounded by a person's yes.
- **A call's `id` and `method` are at most 256 bytes each** (`MAX_CALL_FIELD_BYTES`).

## Errors

| Code | Meaning |
|---|---|
| `-32700`, `-32600`, `-32601`, `-32602`, `-32603` | JSON-RPC 2.0's own |
| `-32001` | `initialize`: no API version in common |
| `-32002` | a call needed a person's confirmation and did not get it: declined, or no one to ask. Nothing ran |
| `-32003` | a call reached the host capability, which refused it or failed; the message is the capability's own |
| `-32800` | the answer to a request srelens cancelled (LSP's value), which srelens ignores; and srelens's answer to a call the sidecar cancelled |

## Supervision

### States

`SidecarStatus` in `crates/plugin-host/src/sidecar/supervisor.rs`:

| State | Meaning | Leaves it |
|---|---|---|
| `Starting` | Being launched, initialized and activated. | on its own |
| `Running` | Serving requests under the negotiated API version. | on its own |
| `Restarting` | It exited unexpectedly; it is started again after the backoff delay. | on its own |
| `Disabled` | It exited unexpectedly once more than the backoff allows. | Restart |
| `Refused` | It cannot run here: no sandbox for this OS or a layer the machine lacks, limits nothing enforces, no API version in common, or a data directory over its limit or not private. | Restart |
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
to its request.

It also stops a sidecar that makes a call whose `id` or `method` is longer than 256 bytes,
a 17th call while 16 answers are still waiting for it, or a call reusing the id of one still
being answered (see [Calls from the sidecar](#calls-from-the-sidecar)), so that nothing a
sidecar sends can grow srelens's memory without limit or make two answers ambiguous. Every
such stop counts as an unexpected exit.

### Stopping

`stop` sends `deactivate` and `shutdown`, 5 s each, closes stdin, and gives the process 5 s
to exit before killing it. Dropping the supervisor kills the sidecar at once.

### Logs

stderr is the sidecar's log. Each line goes into its app's log, which keeps the last 1,000
lines. A line longer than 4 KiB is dropped, and srelens logs a warning that it was. Next to
them are the lines srelens writes about the sidecar: each start, exit, restart and refusal,
and each failed request. **View logs** opens that log in
Settings → Apps → app → Logs.

The sidecar chooses a line's level by starting the line with it:

| The line starts with | Level | Kept as |
|---|---|---|
| `TRACE`, `DEBUG`, `INFO`, `WARN` or `WARNING`, `ERROR`, in any case, optionally `[`in brackets`]`, optionally followed by `:` | that level | the rest of the line |
| `FATAL`, or `panic:` | error | the whole line |
| anything else | info | the whole line |

So `WARN registry is slow`, `[debug] 3 images queued` and `error: scan failed` are read as
warn, debug and error. Every line is redacted before it is kept, including a secret or a
token the sidecar printed by mistake. The log stays in srelens's memory: it is never written
to disk and never offered to MCP. See [inspector.md](inspector.md).

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

- **One writable directory**, its [data directory](#data-directory), which is also its
  working directory.
- **Only the environment srelens names.** The host's own is never inherited: it may hold
  `KUBECONFIG`, cloud credentials or tokens. On Windows it also gets `SystemRoot`, which
  Winsock needs, and `LOCALAPPDATA`, `TEMP` and `TMP`, which Windows reroutes into the
  AppContainer's own folder when it starts the process, so the host's values do not reach
  it. A block without those three failed with error 203 (`ERROR_ENVVAR_NOT_FOUND`) on
  the first CI run.
- **stdin, stdout and stderr, and nothing else.** On Linux and macOS the launcher closes
  every other inherited descriptor before the sidecar runs; on Windows only the three pipe
  ends are inherited.
- On Linux, read access to `/usr`, `/lib` and `/lib64` for the dynamic loader and libc.
  A sidecar that needs other shared libraries beside its binary is not supported.

The conformance suite `crates/plugin-host/tests/sandbox_conformance.rs` runs the spike's
checks against these backends. The `sandbox-conformance` CI job runs it on Linux and
Windows, and it runs on macOS by hand (the file's header says how).

## Data directory

Real tools need scratch space: Trivy keeps its vulnerability database. A sidecar gets one
directory it may write, and nothing else.

- **One per app,** `<root>/<name>`, where `name` is 32 hex characters: the first 16 bytes of
  the SHA-256 of the exact app ID. Two IDs that differ only in case never share one on a
  case-insensitive filesystem, and the path stays short on Windows. On the desktop the root
  is `settings.extensions.data` beside the app inventory (`Apps::data_root`); the web host
  keeps none, as it keeps no app packages. Code: `crates/plugin-host/src/sidecar/data.rs`.
- **Its path** is in `initialize`'s `dataDirectory`, and it is the working directory. On Linux
  and macOS it is also `TMPDIR`, unless the host names another, so temporary files are
  written there and counted. On Windows use it for temporary files: `TEMP` points into the
  AppContainer's folder, which the sidecar may not write.
- **Private.** Created owner-only (`0700`) on Linux and macOS. srelens refuses to start a
  sidecar, or stops a running one, when its directory can be read by other users, belongs to
  another account, or is a symbolic link, which the sandbox would follow. On Windows it
  inherits the user's own directory's ACL, and only the app's AppContainer is granted it.
- **Gone with the app.** Removing an app removes its directory with the next change to the
  inventory, so a later app with the same ID starts empty. An update keeps it. A sidecar may
  have taken its own access away from a directory in it; srelens gives the owner access back
  before removing, and a directory it cannot remove now is tried again with the next change,
  without holding up the others. On Windows srelens also deletes the app's AppContainer
  profile, with its folder and its registry storage. That is best effort too: a profile it
  cannot delete is logged and left, and the change goes ahead.

### Size limit

| Limit | Default | What counts |
|---|---|---|
| `dataBytes` | 1 GiB | for each file, the larger of its length and what the filesystem allocated to it, hard links once; for each directory and link, what was allocated to it |
| `dataEntries` | 100,000 | files, directories and links |

The entry limit also bounds what one measurement costs srelens, and a directory of tiny
files uses disk its byte count does not show.

How it is held:

- **Before each start**, srelens measures the directory. Over a limit, the sidecar is
  `Refused` and not started: "The extension's data directory holds 1.2 GiB, over its 1 GiB
  limit, so srelens did not start it". Clearing the directory and restarting starts it
  (`DataDir::clear`).
- **While it runs**, srelens measures it every 2 s (`Policy::data_check_interval`). Past a
  limit, it stops the sidecar and leaves it `Refused`, not restarting: starting it again
  would only refill it. "…, so srelens stopped it".
- **On Linux and macOS the kernel holds each file** to `dataBytes`: the launcher sets
  `RLIMIT_FSIZE`, and ignores `SIGXFSZ`, so a write past it fails with `EFBIG` and the
  sidecar lives on.

What this guarantees, plainly:
- **On Linux and macOS,** no single file past the limit, for any sidecar.
- **For a sidecar that does not race the walk** (below), a directory past the limit for at
  most one measurement interval, everywhere. Between two measurements such a sidecar can
  write several files each under the limit, so the directory can exceed it by what the
  sidecar can write in 2 s.
- **For a sidecar that does race the walk,** the directory limit does not hold. That is the same kind of guarantee as the
macOS memory watchdog ([ADR](../design/plugin-architecture.md#decision-macos-limits-are-host-enforced)),
for disk: a filesystem quota needs privileges srelens does not have.

Measuring, clearing and removing never follow a symbolic link they find, so a link inside
the directory cannot make srelens empty or delete anything outside it.

**A measurement is a walk, not a snapshot.** A sidecar that renames, moves or swaps
directories and files while srelens walks the directory can make one measurement count a
subtree twice, count what a swapped-in link points to, or miss a subtree altogether. It
cannot make it count past the entry limit. A sidecar that races the walk on purpose can
therefore keep its directory past the limit without being stopped. On Linux and macOS each
file is still capped at the limit by the kernel. Windows has no per-file cap. A cooperating
sidecar is held to the limit; a hostile one is bounded only as far as that. Closing the race
needs a stable view of the directory that no OS gives srelens unprivileged, and it is left for
the escape review ([#744](https://github.com/srelens/srelens/issues/744)).

### Only that path

The sandbox grants the data directory and no other writable path:

- **Linux:** the Landlock ruleset grants write only beneath it. Landlock has no right for a
  file's mode, owner or extended attributes, so the seccomp filter refuses changing them by
  path (`chmod`, `chown`, `setxattr`, `removexattr` and their `*at` forms): without that, a
  sidecar could make a kubeconfig outside its grant readable to every user, or unreadable
  to srelens. A sidecar sets a mode on a file it has open instead (`fchmod`, which Go's
  `(*os.File).Chmod` and Rust's `File::set_permissions` use); it can open for writing only
  what is in its directory. The cost: a tool that sets modes by path, inside its own
  directory too (Go's `os.Chmod`, an archive extractor that keeps each file's mode), gets
  `EPERM` on Linux. Whether Trivy does is not yet known; the SDKs (#576) are where to find
  out. Timestamps (`utimensat`) are not refused, since unpacking an
  archive sets them, so a sidecar can change the times of a file it owns outside its
  directory.
- **macOS:** the Seatbelt profile allows `file-write*` only beneath `DATA`.
- **Windows:** the AppContainer is granted it, and its own profile folder
  (`%LOCALAPPDATA%\Packages\<profile>`) is **made read-only to it**. Windows otherwise
  lets an AppContainer write that folder, and points the sidecar's `TEMP`, `TMP` and
  `LOCALAPPDATA` there; it would be outside the size limit and outlive the app's data. A
  deny entry does not do it: in an AppContainer's access check, a deny for its own SID does
  not outweigh the full control Windows grants it there (the CI run that showed this had
  explicit denies on `AC\Temp` and the write went through). So on every launch srelens
  removes the container's own entries from the folder and everything in it, and grants it
  read and execute on `AC`. `AC\Temp` is made first if it is missing.

The conformance suite checks that a write fails everywhere else a sidecar might try: beside
its directory where the other apps' are, in another app's, in the host's and its own
temporary directories, in `/tmp`, `/var/tmp` and `/dev/shm`, and where its program is. It
checks that a hard link to the kubeconfig cannot be made inside the directory, that one
cannot be read through a symbolic link there, that the kubeconfig's mode cannot be
changed, and that the size limit holds. On macOS 27.0 arm64 all of them passed by hand, and
on Linux (kernel 7.0, arm64) in a privileged container with a delegated cgroup.

The Windows AppContainer's own registry storage is outside this: it is not a path, and it
may be writable to the sidecar while the app is installed. Whether it is was not checked,
and nothing counts it against the limit. It goes with the profile when the app is
uninstalled; locking it down while the app is installed is left for the escape review
([#744](https://github.com/srelens/srelens/issues/744)).

## Not yet

| What | Where |
|---|---|
| An operation that answers with a stream: the protocol has streams, and nothing opens one on an app's behalf yet | — |
| Shipping `srelens-sandbox-launch` in the desktop bundles, and finding a delegated cgroup on a systemd desktop; until then Linux names them with `SRELENS_SANDBOX_LAUNCHER` and `SRELENS_SANDBOX_CGROUP_ROOT` | — |
| A "Clear data" action for an app refused for its data directory (`DataDir::clear` is there; the Inspector, #575, is where a person would find it) | not filed yet |
| JSON Schema for these messages, and the Rust and Go SDKs | [#576](https://github.com/srelens/srelens/issues/576) |
| Memory and CPU limits on macOS | [#713](https://github.com/srelens/srelens/issues/713) |
| The escape-hardening review of the supervisor and its backends, which the ADR assigned to #572 | [#744](https://github.com/srelens/srelens/issues/744) |
