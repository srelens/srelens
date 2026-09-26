# Streams

Extension calls are request/response. Logs, watches, exec, port-forwards,
traces and long-running AI tasks are not, so the host carries them over one
generic stream contract ([#565](https://github.com/srelens/srelens/issues/565),
part of [#520](https://github.com/srelens/srelens/issues/520)). A new stream
source adds a `source` kind; it does not change the frames, the ownership rules
or the limits.

| Layer | Where |
|---|---|
| The contract: frames, ownership, limits, metrics | `crates/streams/src/app.rs` (`AppStreams`) |
| Who may open what, lifecycle, the `read` and `watch` sources | `crates/registry/src/extensions/streams.rs` (`ExtensionStreams`) |
| The pod sources `logs`, `exec`, `portForward` (#567) | `crates/registry/src/extensions/streams/pods.rs`; scopes in `crates/registry/src/extensions/pods.rs`, cluster calls in `crates/kube/src/app_pods.rs` |
| Desktop commands, frames as Tauri events | `apps/desktop/src-tauri/src/extension_streams.rs` |
| Client | `packages/core/src/lib/extensionStreams.ts` (`openExtensionView`) |

## Frames

The client subscribes to a channel it chose, `extstream:` followed by letters,
digits, `-`, `/`, `:` or `_` (at most 200 characters), and only then asks the
host to open. So the first frame cannot race the listener. Every frame names
its `stream`:

| Frame | Fields | When |
|---|---|---|
| `open` | `stream` | Once, before anything else. |
| `data` | `stream`, `seq` (from 1), `data` | Any number of times. |
| `close` | `stream`, `reason` | Terminal: the stream ended normally. |
| `error` | `stream`, `code`, `message` | Terminal: the stream failed or was stopped. |

Exactly one terminal frame is sent, and nothing after it.

`close.reason` is `completed`, `cancelled`, `viewClosed`, `appDisabled`,
`appUpdated`, `appRemoved`, `windowClosed` or `windowReloaded`. `error.code` is `source` (the source failed: the
cluster refused, timed out or was unreachable) or `rateLimited` (the host
stopped the stream for the app's message rate). A source that panics ends
with `error: source` too, and frees its place in the app's open-stream count.
When the host's stream manager itself is dropped, it aborts every source it
started, without a terminal frame, because nothing is left to deliver one.

**An error is never a close.** A failed or stopped stream must not read as one
that ended, and an ended one must not read as empty. `onEnd` in
`@srelens/core` receives the two as different variants, and
`describeStreamEnd` words them differently ("The stream failed: …" against
"The app was disabled.").

The client's half is `cancel`, which ends one stream with `close: cancelled`.
It is idempotent: cancelling again, or cancelling a stream that already ended,
changes nothing and sends nothing.

## Ownership

Every stream is opened by one **view** of an app (a page, a detail tab, a
panel) and belongs to it. The view id is chosen by the client, unique per
mounted view, and never interpreted by the host. `openExtensionView(appId, label)`
makes one from a counter and a random part: the host groups streams by view id
across the whole process, and the counter starts again in every window and
after every reload, so a counter alone would let one window's close end
another's streams.

- **Closing the view** (`view.close()`, the `extension_stream_close_view`
  command) ends every stream that view opened, with `viewClosed`, and no other.
  A stream whose open was still in flight when the view closed is cancelled as
  soon as its open returns.
- **Disabling the app** ends every stream it opened, with `appDisabled`. That
  includes a disable by the unsigned-app policy and a quarantine.
- **Updating or rolling back the app** assigns a new revision. Every stream
  opened from another revision ends with `appUpdated`, and an open that names
  it is refused ("refresh the view").
- **Removing the app** ends its streams with `appRemoved`.
- **Closing the window** that opened a stream ends it with `windowClosed`,
  and **reloading** it ends it with `windowReloaded`
  ([#700](https://github.com/srelens/srelens/issues/700)). A page that goes
  away never closes its views, so the host tracks the window too: each stream
  records the label of the window whose call opened it — the label Tauri gives
  the call, never one the page sends, so one window cannot end another's. A
  destroyed window ends its streams from the host's window-event handler. A
  reload keeps the window, so the new page calls `window_streams_reset` as the
  transport loads, and every command that opens a stream waits for it; the
  reset cannot end the new page's own streams. If the reset fails, the stream
  is not opened: the open is refused with the reason, and the next open tries
  the reset again. Once a reset succeeds it is not repeated for that page.
  Each ending moves the window's epoch, and an open the old page began before
  it is refused when it lands ("The window … closed or reloaded while this
  stream was opening"), with no frame. Another window's streams — the same
  app, the same page — are untouched. The desktop's built-in resource watches
  and pod exec sessions end the same way; a shell's task is aborted, which
  drops its connection to the cluster.

The lifecycle rule holds whichever registry made the change. Every inventory
write is announced to the streams of that inventory, and those are shared by
every registry in the process that serves it, so an `extensions.configure`
over MCP ends the desktop view's streams too. A change made by another process
is caught by the source itself: the `read` source rechecks the installed app
on every tick and ends with an `error` when it no longer may read, and the
`watch` source rechecks on every reconnect.

## Limits

Per app, over all its views:

| Limit | Value | Exceeding it |
|---|---|---|
| Open streams | 8 | The open is refused: "App … already has 8 open streams, the most one app may have; close a view or cancel a stream first". No frame is sent. |
| Data frames | 50 per second (a token bucket, so a burst of 50) | The stream that sent the frame ends with `error: rateLimited`: "App … sent more than 50 stream messages per second, the most one app may send; the host stopped this stream". |

## Sources

### `read`

```json
{ "kind": "read", "capability": "kustomizations", "intervalSeconds": 15 }
```

Re-runs one of the app's declared readers every `intervalSeconds` (5–300,
default 15), and sends each answer as a `data` frame. It is the first source
because it adds no permission, no new cluster call and no new trust: every
tick goes through `extensions.read`'s own path, so it rechecks the installed
revision, the grants, the cluster scope and the bound CustomResourceDefinition
exactly as a single read would. A read that fails ends the stream with
`error: source` and the reason.

### `watch`

```json
{ "kind": "watch", "capability": "kustomizations" }
```

Follows the kind one of the app's declared readers lists, and says when what
that reader would answer has changed
([#566](https://github.com/srelens/srelens/issues/566)). App pages, table
columns and dashboard cards use it instead of a manual Refresh.

**What it watches.** Only the kind the named binding reads, and only where
that binding may read it:

| Binding target | Kind watched |
|---|---|
| `k8s.listCustomResource` | The bound `group` and `plural`, at the version a read resolves to ([#547](https://github.com/srelens/srelens/issues/547)): the first of the binding's accepted versions the CustomResourceDefinition serves. Resolved at the open and again on every reconnect. |
| `k8s.listEvents` | Core `v1` Events. |
| `k8s.listDeployments`, `…StatefulSets`, `…DaemonSets`, `…Nodes` | That built-in kind, as `builtin_reader_identity` names it. |

The cluster is the stream's `context`, resolved and scope-checked exactly as
`extensions.read` resolves it. A namespaced kind is watched in the stream's
`namespace` — the view's — or in every namespace when that is empty, as the
reader would list it; a cluster-scoped kind ignores the namespace. The open is
authorized by the same check a read makes (installed, enabled, this revision,
enabled for this cluster, the binding still valid against its grants, the
CustomResourceDefinition serving it). The app's part of the check runs again on
every inventory write the host announces and on every reconnect; the cluster's
part — the CRD — on every reconnect. A check that fails ends the stream with
`error: source` and the reason, except a CRD lookup the cluster could not
answer, which is one more failed reconnect attempt: only a CRD the cluster says
is gone ends the watch. A revision change ends it with `close: appUpdated`
first, as for every source.

**What a frame carries.** Never an object. A `data` frame is one of:

| `data` | Meaning |
|---|---|
| `{ "event": "synced" }` | A full list of the kind completed — the first one, and the one after every reconnect. What the view shows may be stale: read again. |
| `{ "event": "changed" }` | One or more objects of the kind were added, modified or deleted since the last frame. Read again. |
| `{ "event": "reconnecting", "message": "…" }` | The watch was lost. Until the next `synced`, what the view shows is **not current** and must say so. |

The view re-reads through the path it already uses — `extensions.read`,
`extensions.resolveColumns`, `extensions.resolveCards` — so every re-read goes
through every check, redaction and rule a manual Refresh does, and the watch
adds no new way for cluster data to reach the app. The host drops the shared
reader snapshot columns and cards are answered from before it sends
`synced` or `changed`, so the re-read lists the cluster instead of answering
from a snapshot older than the change. The host watches metadata only, so a
watched object's `data` or `spec` does not reach the host through the watch
either, and nothing of an object — not its name — is in a frame.

**Lost watches.** The watch is one Kubernetes list followed by a watch. Any
error on it ends that session. Forbidden ends the stream with `error: source`.
Any other error, `410 Gone` among them, sends `reconnecting`, waits (1 s,
doubling to 30 s), resolves the kind again as a read would, and starts again
from a **fresh list**, never from the old `resourceVersion`, so nothing that
changed while it was down is missed; that list sends `synced`. A kind no longer
served at the version followed (404) is resolved again too: a binding that
accepts another version the CRD still serves is followed there, as its reads
now are; one that resolves to the same kind again, or to none, ends the stream
with `error: source` — with the read's own refusal when none is served. A
change of preferred version that does not break the watch is not chased: the
watch only says when to read, and the read resolves the version itself.

**Coalescing.** A noisy kind does not send a frame per event. `changed` is sent
at most once per **1-second window** per stream: the first change after a
quiet second is sent at once, and any further changes in the window are
folded into one frame at its end (a change still waiting out its window when
the watch is lost is reported by the `synced` of the relist after it). So one
watch sends at most about one frame
a second under any churn, and an app's eight streams stay well below its 50
frames per second. The cap still applies: an app that exceeds it anyway
(`synced` and `reconnecting` are not coalesced) has the stream ended with
`error: rateLimited`, as for every source, never silently thinned out.

**Cap.** A watch is a stream, and counts against the app's 8 open streams;
there is no separate watch cap. One view opens one watch per reader it shows
(the Flux overview shows six), so a lower sub-cap would refuse the second
view of the same app. A refused open is a view with no live updates, and it
says so.

**Web.** The stream commands are refused on the web host (see
[Hosts](#hosts)), so on the web a page, a column and a card stay as they
were: read once, and again on Refresh. Pages and the dashboard's app cards say
"Not live" rather than imply they are current; a joined column follows its
table's own refresh.

**In the UI.** `packages/ui-next/src/extensions/liveReaders.tsx` holds a
view's watches (`LiveReaders`, `useLiveReaders`, `useLiveApps`): one watch per
reader however many parts of the view show it, a re-read in place
(`useResource`'s `refresh`) on `synced` and `changed`, and `LiveStatus` /
`LiveNotice` to say which state the view is in. An app page is one view; a
table's joined columns and the dashboard's cards are one view per app.

## Inventory announcements

Every inventory write the host announces to the streams (see
[Ownership](#ownership)) is also sent to the desktop window as a Tauri event,
`extensions:inventory`, with `{ "type": "changed" }`. The window's app list
(`inventoryStore.ts`) listens for it and reads `extensions.list` again when it
arrives, instead of polling every five seconds. It still reads again when the
window gains focus, which catches a change made by another process (a
headless `srelens mcp` writing the same inventory). If the event cannot be
subscribed to, the store falls back to reading every five seconds and says so
on the Apps screen. The web host keeps each user's inventory (#515) but
announces nothing, so there the list is read when the window changes it and
when the window gains focus, which is where a change made in another tab shows.
It neither polls nor claims to be live.

Metric providers ([#569](https://github.com/srelens/srelens/issues/569)) are further
`source` kinds.

### Pod sources

`logs`, `exec` and `portForward` ([#567](https://github.com/srelens/srelens/issues/567))
go through one of an app's pod bindings (`capability`) to one pod its scope admits —
selected by the object `name` names, in the stream's `namespace`, for a binding scoped
by `resource`; or any pod in a namespace its permission grants, and then no `name`. See
[manifest.md](manifest.md#logs-exec-and-port-forwards) for the bindings and their scopes.

Every open is authorized as a read is (installed, enabled, this revision, enabled for
this cluster, the manifest valid against its grants), then held to the scope: the host
reads the object and its selector, then the pod the view names, and matches the pod's
namespace and labels itself. A pod outside the scope is refused with why ("Pod db-1 is
not selected by Deployment web in team, so app … may not reach it"), and nothing is
opened. The app's part of the check runs again on every inventory write the host
announces; the scope is read again whenever a log stream reconnects and whenever a
Service's forward follows another pod.

Frames are batched: lines and output gathered over **250 ms** go as one frame, so one
stream sends at most about four a second and a chatty container cannot take the app
past its 50 frames per second.

#### `logs`

```json
{ "kind": "logs", "capability": "controllerLogs", "name": "cert-manager",
  "pod": "cert-manager-7d9f8b6c5-x2x9k", "container": "cert-manager-controller",
  "tailLines": 200, "sinceSeconds": 3600, "timestamps": true }
```

Follows one container: the binding's own `container`, else the one the view names,
else the pod's only one. `tailLines` (0–5000, default 200) and `sinceSeconds` apply to
the first follow; a reconnect asks for nothing already sent.

| `data` | Meaning |
|---|---|
| `{ "event": "lines", "lines": [{ "source": "pod/container", "line": "…", "truncated"?: true }], "dropped"?: n }` | Up to 500 lines and 512 KiB of text (a single longer line goes alone). A line past 16 KiB is cut, on a character boundary, and says so. `dropped` counts lines the host let go because they arrived faster than it sends them: it holds up to 5000 lines and 4 MiB of text. |
| `{ "event": "status", "source": "pod/container", "status": "live" \| "reconnecting" \| "completed", "message"?: "…" }` | The follow's state, tagged like its lines. `reconnecting` says why; `completed` says the pod is gone or finished, and is followed by `close: completed`. |

A lost stream reconnects after 2 s, from nothing new, while the app may still reach
the pod; Forbidden ends it with `error: source`, and so does a pod that has left the
scope. **This is the log contract a log source speaks**, not only this one: a log
provider's stream (#569) sends the same two events, and `startExtensionLogStream` in
`@srelens/core` hands them to the callbacks `startLogStream` takes, so the pod log
view's buffer (`useLogStream`'s `source` option in `packages/ui-next/src/lib/logStream.ts`)
follows any of them the same way.

#### `exec`

```json
{ "kind": "exec", "capability": "status", "name": "cert-manager",
  "pod": "cert-manager-7d9f8b6c5-x2x9k", "container": "cert-manager-controller",
  "confirmed": { "pod": "cert-manager-7d9f8b6c5-x2x9k", "container": "cert-manager-controller",
                 "command": ["cmctl", "status", "certificate", "--all-namespaces"] } }
```

Runs the binding's command once, as written, with no stdin and no terminal. There is no
field for a command: it is the manifest's. `confirmed` is what the host confirmation
named when the person approved it, and the open is refused — before anything runs —
unless it names exactly the pod, container and command the host would run. A refusal
says what it would have run, and ends "nothing ran".

| `data` | Meaning |
|---|---|
| `{ "event": "output", "chunks": [{ "stream": "stdout" \| "stderr", "text": "…" }] }` | Output as it arrives, consecutive chunks of one stream joined. |
| `{ "event": "exit", "code": n }` | The command exited, with its code; `close: completed` follows. A non-zero code is the command's answer, not the stream's failure. |

A command the cluster could not run (no such container, no such program) ends with
`error: source` and the cluster's own words. So does one that runs past 300 s or writes
more than 1 MiB: the host stops it and says which.

#### `portForward`

```json
{ "kind": "portForward", "capability": "webhook", "name": "cert-manager",
  "service": "cert-manager-webhook" }
```

`pod` for a binding that forwards to a pod, `service` for one that forwards through a
Service. The host binds `127.0.0.1` on a port it picks before the stream opens — a port
it cannot open refuses the open — and there is no field for the view to choose one.

| `data` | Meaning |
|---|---|
| `{ "event": "ready", "localPort": n, "pod": "…", "port": n, "service"?: "…", "servicePort"?: n }` | Listening on `localPort`; each connection reaches `port` of `pod`. Sent again when a Service's forward follows another pod. |
| `{ "event": "connectionFailed", "count": n, "message": "…" }` | `count` connections through the forward were refused by the cluster since the last report (RBAC `pods/portforward`, a closed container port), the last with `message`. Each was closed at once on this computer's side. Sent at most once per 2 s check, so a client retrying in a loop cannot take the app past its message rate. The forward is still listening. |

The source owns the listener and every connection through it. When the stream ends —
cancelled, its view closed, its window closed or reloaded, the app disabled, updated or
removed — the port stops listening and each connection is closed, on both sides. Every
2 s it checks its pod: a pod that leaves the scope ends it with why; one that stops
running ends a forward to a pod, and moves a Service's forward to another running pod
in scope, or ends it when there is none.

#### Recorded sessions

Exec and port-forward sessions are in the audit trail (#555), like every other
sensitive or mutating call from the app: one record when a session starts, and one for
every session refused once its binding was found, with the reason. Each names the app
and revision, the cluster, `namespace/pod`, the binding, and the command (exec) or the
ports (a forward). An exec record's decision is `approved` when the host confirmation
was named, `denied` when it was not; a forward's is `auto`. Log streams are reads, and
are not recorded.

#### `extensions.pods`

`extensions.pods` (read-only) answers the pods one pod binding may reach now, for a view
to offer, held to the same authority and scope an open is — `{ "id", "revision",
"capability", "context", "namespace", "name"? }` in, `{ "pods": [{ "name", "namespace",
"containers", "phase", "ready" }], "services"?, "truncated"?, "scope" }` out, where
`scope` is the host's own words ("pods selected by Deployment web") and `services`, for a
forward through a Service, lists the Services that send to a pod in scope.
`extensionPods()` in `@srelens/core` reads it.

## Opening a stream

`extension_stream_open` takes one argument, `input`, which the host holds to
`OpenStreamIn`. Its fields are camelCase and unknown fields are refused, the
snake_case spelling among them:

```json
{
  "id": "org.example.argocd",
  "revision": 1,
  "view": "org.example.argocd/page:applications#1",
  "channel": "extstream:1-k2j3h4",
  "context": "cluster/a",
  "namespace": "team",
  "source": { "kind": "read", "capability": "applications", "intervalSeconds": 30 }
}
```

That payload is committed as `packages/core/src/lib/extension-stream-open.json`:
the TypeScript wrapper is tested to produce it and the Rust struct to accept it. The
watch and pod sources have theirs beside it (`extension-stream-watch.json`,
`extension-stream-logs.json`, `extension-stream-exec.json`,
`extension-stream-port-forward.json`).

It answers `{ "stream", "channel" }`, or refuses with why: not installed,
disabled or at another revision, not enabled for the cluster, no such reader,
an interval out of range, a channel outside `extstream:`, a missing view, the
open-stream cap, or — for a pod source — a pod outside the binding's scope, an exec
without the host confirmation, or a local port the host could not open.

## Metrics

`extensions.streams` (read-only) answers the open streams of this process and
what each app has sent since it started, for the Inspector
([#575](https://github.com/srelens/srelens/issues/575)):

```json
{
  "apps": [{
    "app": "org.example.flux", "openStreams": 1, "opened": 3,
    "messages": 41, "bytes": 18230, "rateLimited": 0, "refused": 0, "windowEnded": 2,
    "streams": [{ "stream": "s-3", "view": "org.example.flux/page:kustomizations#2",
                  "revision": 7, "source": "read", "messages": 12, "bytes": 5400 }]
  }],
  "maxOpenPerApp": 8,
  "messagesPerSecond": 50
}
```

`bytes` counts `data` payloads as compact JSON. `windowEnded` counts the
streams that ended because their window closed or reloaded. `extensionStreamMetrics()` in
`@srelens/core` reads it.

## Hosts

- **Desktop:** frames are Tauri events on the stream's channel, through the
  same `EventSink` every other stream uses.
- **Web:** the three commands are refused (`WEB_DENIED_COMMANDS`). Each user
  has their own apps there ([#515](https://github.com/srelens/srelens/issues/515)),
  but the server does not yet open a user's streams or carry their frames, so
  `extensions.streams` answers with no open streams. An app that binds the pod
  capabilities still installs there, and its pod tools say that app streams run in the
  desktop app; running them on the server is [#727](https://github.com/srelens/srelens/issues/727),
  and the pod sources need nothing of the contract changed to run there. Nothing in the contract is
  desktop-specific: the client goes through the transport shim, so on the web
  its frames would arrive as `/api/ws` frames on the same channel.
