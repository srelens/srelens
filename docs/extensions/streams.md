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
| Who may open what, lifecycle, the `read` source | `crates/registry/src/extensions/streams.rs` (`ExtensionStreams`) |
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
`appUpdated` or `appRemoved`. `error.code` is `source` (the source failed: the
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

The lifecycle rule holds whichever registry made the change. Every inventory
write is announced to the streams of that inventory, and those are shared by
every registry in the process that serves it, so an `extensions.configure`
over MCP ends the desktop view's streams too. A change made by another process
is caught by the source itself: the `read` source rechecks the installed app
on every tick and ends with an `error` when it no longer may read.

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

Watches for bound kinds ([#566](https://github.com/srelens/srelens/issues/566)),
logs, exec and port-forwards ([#567](https://github.com/srelens/srelens/issues/567))
and metric providers ([#569](https://github.com/srelens/srelens/issues/569)) are
further `source` kinds.

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
the TypeScript wrapper is tested to produce it and the Rust struct to accept it.

It answers `{ "stream", "channel" }`, or refuses with why: not installed,
disabled or at another revision, not enabled for the cluster, no such reader,
an interval out of range, a channel outside `extstream:`, a missing view, or
the open-stream cap.

## Metrics

`extensions.streams` (read-only) answers the open streams of this process and
what each app has sent since it started, for the Inspector
([#575](https://github.com/srelens/srelens/issues/575)):

```json
{
  "apps": [{
    "app": "org.example.flux", "openStreams": 1, "opened": 3,
    "messages": 41, "bytes": 18230, "rateLimited": 0, "refused": 0,
    "streams": [{ "stream": "s-3", "view": "org.example.flux/page:kustomizations#2",
                  "revision": 7, "source": "read", "messages": 12, "bytes": 5400 }]
  }],
  "maxOpenPerApp": 8,
  "messagesPerSecond": 50
}
```

`bytes` counts `data` payloads as compact JSON. `extensionStreamMetrics()` in
`@srelens/core` reads it.

## Hosts

- **Desktop:** frames are Tauri events on the stream's channel, through the
  same `EventSink` every other stream uses.
- **Web:** the three commands are refused (`WEB_DENIED_COMMANDS`) and so is
  `extensions.streams`, for the reason every `extensions.*` capability is: the
  web host keeps no per-user app inventory yet
  ([#515](https://github.com/srelens/srelens/issues/515)). Nothing in the
  contract is desktop-specific: the client goes through the transport shim, so
  on the web its frames would arrive as `/api/ws` frames on the same channel.
