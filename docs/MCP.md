# MCP: wiring an agent to srelens

## What this is

srelens exposes everything it can do to a Kubernetes cluster over the [Model
Context Protocol](https://modelcontextprotocol.io) — the same capabilities
the desktop UI uses, reachable by an AI agent as MCP tools, prompts and
resources, using your locally authenticated cluster contexts. This file is
for whoever is wiring an agent to that server: what to connect to, what the
safety model actually gates, and two worked examples of an agent driving it.
For the desktop app itself — installing srelens, browsing clusters by hand,
the Settings panels — see [USAGE.md](USAGE.md). For the exhaustive,
generated list of every tool, prompt and resource template, see
[mcp-catalog.md](mcp-catalog.md); this file links to it rather than
restating it, so the two cannot drift apart.

## Transports

srelens speaks MCP over two transports:

- **stdio** — the client spawns `srelens --mcp-stdio` itself and talks
  newline-delimited JSON-RPC over its stdin/stdout. The client already holds
  your privileges by virtue of having started the process, so there is no
  token to configure. This is the simplest setup and what Claude Desktop,
  Claude Code, and most local agent tooling expect.
- **Loopback HTTP** exposes the server on a local port over a bearer token —
  but as two distinct modes, because they run in different processes with
  different consequences for who approves a gated call:
  - **Settings → MCP → Run the MCP server**, in an already-running desktop
    app, runs the HTTP server *inside* the GUI's own process, sharing its
    authenticated cluster connections and, crucially, its confirm dialog: a
    gated tool call over this connection pops the same in-app approval
    prompt a click in the UI would, in whichever srelens window is open.
    This is the mode with a human in the loop.
  - **`srelens --mcp-http <addr>`** spawns a **separate, headless** process.
    `<addr>` must be a loopback address — `127.0.0.1:8765` (the default) or
    `[::1]:8765`; anything else, such as `0.0.0.0:8765`, is refused at
    startup with an error naming the address, and nothing listens, unless
    you also pass `--mcp-expose-http` (see [Security model](#security-model)
    before you do). The startup message reports the address actually bound
    and whether it is loopback or exposed. It does not attach to a running
    GUI — if the GUI's own toggle already holds the port, this second
    process fails to bind rather than sharing it. There is no window and
    therefore no dialog: a gated call is refused
    unless the process was started with `--mcp-allow-destructive` /
    `--mcp-allow-sensitive-reads` and the individual call carries
    `"_confirm": true`. Once a call is pre-authorised that way, it proceeds
    with **no human approval at any point** — this flag combination is for
    deliberate unattended automation, not a way to reach a human reviewer.

Pick stdio for a client that spawns srelens itself — the common case. Reach
for the Settings → MCP toggle only if you need calls to land in an
already-running GUI session, so a human watching the app can see and approve
confirm dialogs as they happen, or so the agent shares cluster contexts the
GUI already has open. Reach for `srelens --mcp-http` only for unattended
automation where you deliberately pre-authorise gated calls with the flags
above; it is not a way to reach a human reviewer, since headless means
exactly that.

### Request size limits

Both transports refuse a request over **4 MiB** (4,194,304 bytes,
`MAX_REQUEST_BYTES` in `crates/mcp/src/lib.rs`) before handing it to a tool:

- **stdio** counts the bytes of one line, without its `\n` or `\r\n` ending. A longer line
  is read and dropped as it arrives, never buffered whole, and answered with
  a JSON-RPC error whose `id` is `null`, since the request was never parsed:
  `{"code": -32600, "message": "request exceeds the 4194304-byte limit on one
  stdio line", "data": {"field": "request", "limit": 4194304}}`. The session
  carries on with the next line.
- **HTTP** counts the request body. A larger body is refused with
  `413 Payload Too Large` before the JSON-RPC handler runs.

Within that, some tool arguments have limits of their own, checked while the
arguments are decoded and refused as invalid input naming the field and its
limit. On `extensions.validate` and `extensions.configure`, a `signature` must
be exactly 64 bytes, a `manifest` at most 256 KiB, and a `settings` object at
most 64 KiB as compact JSON.

The desktop app's own WebView calls capabilities through a Tauri command, not
through MCP, so the 4 MiB transport limit does not apply there; the per-field
limits do.

## Security model

- **HTTP requires a bearer token.** The transport never serves
  unauthenticated. Settings → MCP shows the current token (masked, with
  reveal/copy) plus **Rotate** and **Revoke** buttons. If the server is
  running, rotating restarts it immediately so the new token takes effect —
  any in-flight agent request is dropped, and every client still configured
  with the old value needs the new one before it works again. (Rotating
  while the server is stopped just replaces the stored token; it does not
  start the server.) Revoking also stops the server — it must never run
  without a valid token.
- **The HTTP transport binds loopback only.** The in-app server always binds
  `127.0.0.1`. Headless `srelens --mcp-http <addr>` refuses any address
  that is not loopback (`127.0.0.0/8` or `::1`) with an error naming it,
  and exits without listening. This is the network boundary; the Host check
  below is not one, because a non-browser client can send any `Host` it
  likes.
- A **Host header check** rejects requests whose `Host` isn't a loopback
  value (a loopback IP such as `127.0.0.1` or `::1`, or `localhost`) —
  every address the bind rule above accepts. Binding loopback alone doesn't
  stop a page on another domain from resolving to 127.0.0.1 and posting to
  the port; the Host check does. It applies to every route, including the
  unauthenticated `/healthz`, so nothing here answers a caller that isn't
  genuinely local.
- **`--mcp-expose-http` exposes the headless server beyond loopback.** Pass
  it with `--mcp-http` to bind a non-loopback address, such as
  `0.0.0.0:8765` or a LAN interface. Understand the risk before you do:
  - The server speaks **plain HTTP**. There is no TLS in srelens and the
    flag does not require any, so the bearer token and every tool result —
    cluster data included — cross the network in the clear. Anyone who can
    observe the traffic can read them and replay the token; anyone who can
    reach the address and holds the token can call every tool the process
    allows. `/healthz` answers anyone who can reach it.
  - The Host check widens to fit: an exposed server also accepts a `Host`
    that is an IP literal (`192.168.1.5:8765`, `[fd00::5]:8765`), because
    that is what a client that connected by IP sends. A hostname other than
    `localhost` is still refused, since DNS rebinding always presents a
    hostname. Connect to an exposed server by IP.
  - Gated tools still need the process flags and `"_confirm": true`; the
    flag changes where the server listens, nothing else.

  Use it only on a network you trust end to end — a private lab network,
  a VPN, a container network you control. To reach the server from
  elsewhere, do not expose it: keep the loopback bind and put an SSH tunnel
  or a TLS-terminating reverse proxy in front. A proxy that forwards to
  `http://127.0.0.1:8765` with `Host: 127.0.0.1:8765` (nginx's default for
  `proxy_pass`) passes the Host check without any flag, and TLS then covers
  the token on the wire.
- **stdio needs no token** — the client spawned the `srelens --mcp-stdio`
  process itself and already holds your privileges.
- **To supply your own token**, set `SRELENS_MCP_TOKEN` to 64 hex
  characters. There is deliberately **no `--mcp-token` flag**: command-line
  arguments are visible to every account on the machine via `ps`, which
  would hand the token to exactly the local processes it exists to keep out.
  Without the variable, srelens reads a token from the store, or generates
  one and prints it to stderr. HTTP only — stdio takes no token at all.
- **Destructive tools prompt in the app.** The MCP call blocks until you
  approve or deny the dialog that pops up. Letting it time out, dismissing
  it, or having no srelens window open at all count as **deny**. Confirmation
  requests from concurrent calls queue rather than colliding.
- **Headless use** (`--mcp-stdio` / `--mcp-http` with no GUI to show a
  dialog) needs an explicit opt-in instead: a process-level flag *and*
  `"_confirm": true` on the individual tool call. Neither alone is enough —
  `_confirm` states intent, it does not authorize anything by itself. There
  are two flags, because they are two different risks and granting one must
  not grant the other:

  | Flag | Authorizes |
  | --- | --- |
  | `--mcp-allow-destructive` | anything that changes state — delete, drain, scale, apply, helm install, installing local tooling |
  | `--mcp-allow-sensitive-reads` | reads that return secret material — `k8s.getSecret`, or the SSH node diagnostics, whose host logs and process arguments can carry credentials (the full set is under "sensitive read" in [mcp-catalog.md](mcp-catalog.md)) |

  So an agent allowed to read a Secret still cannot drain a node, and an
  agent allowed to drain nodes cannot read your Secrets. Both flags apply to
  both transports.
- **There is no GUI toggle for stdio.** A GUI can't govern a process a
  client spawned directly, so those CLI flags are the entire stdio control
  surface.
- Every call is recorded to an **audit log** at
  `<app config dir>/mcp/audit.jsonl` (mode `0600`, rotated once to `.1` past
  5 MB), viewable in Settings → MCP under recent agent activity. Argument
  values are redacted before they're written, so the log records the shape
  of a call without its contents:
  - sensitive capabilities redact every value;
  - keys that look like credentials (`token`, `secret`, `password`, `key`)
    are redacted at any nesting depth;
  - whole payload fields are redacted — `data`/`stringData` on a Secret
    write, `yaml` on `k8s.applyManifest`, and `values` on the helm
    capabilities. These carry secret material under key names that look
    perfectly ordinary (`username`, `ca.crt`), so matching key names alone
    would miss them;
  - `settings` on `extensions.configure` keeps its setting names but loses
    every value. An app's settings are free-form and nothing marks one as
    secret, so a value under `credential` or `certificate` would otherwise be
    written verbatim — for a denied call too;
  - a recorded error message is scrubbed of every value the rules above
    removed, because a capability that refuses an argument tends to echo it
    (`invalid type: string "…", expected a map`).

  Identifying fields like `context`, `namespace`, `name` and `kind` survive,
  so you can still see which cluster and object an agent touched, and the
  `action` and app `id` of an `extensions.configure` call survive with them.
- The bearer token lives in your **OS keychain** where one is available,
  falling back to a `0600` file otherwise (headless Linux, minimal window
  managers). Settings → MCP only speaks up about this when it has fallen
  back: a warning appears saying the token is stored in a plain file on disk
  (readable only by your user account) instead of the keychain. No warning
  means the keychain is serving.

Put plainly: **reads flow freely, gated tools do not.** An agent can list
pods, fetch manifests, tail logs and walk events without ever touching a
confirm gate; the moment it calls something that mutates the cluster or
reads secret material, it either raises a dialog in the app or, headless,
needs the matching `--mcp-allow-destructive` or `--mcp-allow-sensitive-reads`
flag plus `_confirm: true` on that call.

Review tool calls and use appropriate Kubernetes RBAC, especially with
critical clusters.

## The catalog and its safety classes

[mcp-catalog.md](mcp-catalog.md) enumerates every tool, the built-in prompts,
and every resource URI, grouped by area (Kubernetes, Helm, Toolbox, Server)
and, for tools, by **safety class**. There are exactly four:

| Class | Confirm gate? | Headless flag needed |
| --- | --- | --- |
| read-only | never | none |
| sensitive read | yes | `--mcp-allow-sensitive-reads` |
| needs confirmation | yes | `--mcp-allow-destructive` |
| destructive | yes | `--mcp-allow-destructive` |

A tool's safety class is what the security model above actually keys off:
read-only tools never raise a confirm dialog and need no flag headlessly;
the other three are all gated, and *how* depends on where the server runs —
in the app the call pauses on the confirm dialog and nothing else is needed,
while headless it needs the matching flag plus `"_confirm": true` on the
call. `_confirm` is a headless-only mechanism: the in-app policy never reads
it. The three classes differ only in which flag authorizes them headlessly
and in how much damage they can do.

**`sensitive` is a separate, orthogonal property — not a fifth safety class
and not a synonym for "gated".** It governs one thing: whether the audit log
redacts a call's arguments (see Security model above). Whether a tool also
requires confirmation is a completely independent decision. The catalog
has both combinations, on purpose:

- `k8s.diffManifest` is sensitive (it can echo back manifest content, so its
  arguments are redacted in the audit log) but is **not** confirm-gated —
  it changes nothing on the cluster, so it's classed plain **read-only**.
  You can call it headlessly with no flag at all.
- `k8s.getSecret` is sensitive **and** confirm-gated, because unlike a diff
  it returns actual secret values. That combination is its own class,
  **sensitive read**, gated behind `--mcp-allow-sensitive-reads` rather than
  `--mcp-allow-destructive` — reading a Secret is not a mutation, so it
  would be wrong to lump it in with tools that drain nodes or delete
  resources.

If you only remember one thing from this section: don't infer whether a
tool needs confirmation from whether the catalog marked it `sensitive` — go
look up its actual safety class.

### Impact is a third axis

The catalog's tool tables also carry an **impact** level — `low`, `medium` or
`high` — which answers a different question again: not *whether* a call is
gated, but how much it disturbs if it runs.

| Level | What it covers |
| --- | --- |
| `low` | A read, or a write whose only effect is to make a controller look again. |
| `medium` | Changes cluster or host state but leaves workloads running: a scale, a suspend, a tool install, a Secret returned to the caller. |
| `high` | Destroys, disrupts or replaces something running: a delete, a drain, a sync that applies manifests and runs hooks. |

It cannot contradict the safety class — anything destructive is `high`,
anything gated is at least `medium`, an ungated read is `low` — but it
separates tools the class puts together. Two "needs confirmation" tools are
not equally alarming.

A headless denial now names the level and, where the host has authored one,
the host's own sentence for the call, so an agent asked to re-send with
`"_confirm": true` is told what it is confirming rather than only that the
tool "mutates the cluster".

One caveat, for a tool that takes a named operation: the published level is
the **highest** any operation it accepts can reach, because `tools/list`
carries one row per tool. `k8s.gitOpsAction` is `high` because one of its
eight actions is an Argo CD sync, although its `refresh` action is `low`.
The per-action level comes back with the resource, on `extensions.resource`.

## Client configuration

**Prerequisite: `srelens` must resolve on `PATH`.** Every generated config
below runs `"command": "srelens"` as a bare name, but a normal install does
not put it there. From the desktop app, use **Settings → MCP → Install the
srelens CLI**, which symlinks the running binary to `~/.local/bin/srelens`.
Then confirm it actually resolves — `command -v srelens` — before pasting a
config that assumes it does. (Don't reach for `srelens --version`: there is no
version flag, and an unrecognised argument launches the GUI instead of
printing anything.) If it doesn't, either add
`~/.local/bin` to your shell's `PATH` or replace `"srelens"` with the
absolute path Settings → MCP reports. **This install step is Unix-only**
(macOS/Linux); on Windows, skip it and put the absolute path to the
installed `srelens.exe` directly in `"command"` instead.

Ready-to-paste snippets for Claude Desktop, Claude Code, a generic stdio
client, the headless consent flags, and the HTTP transport are all in
[mcp-catalog.md § Client configuration](mcp-catalog.md#client-configuration).
Copy the one that matches your client rather than hand-rolling it — a test
in this repository parses every one of those fenced blocks as JSON, so a
config that isn't valid JSON fails the build; it does not, by itself, prove
the envelope matches what your specific client expects.

## Worked example 1: read-only triage

Prompt: *"Why is pod `web-0` failing?"*

An agent working this purely from cluster state, with no destructive intent,
might run:

1. `k8s.listPods` — confirms `web-0` exists and reads its phase/restart
   count in the `web` namespace.
2. `k8s.getManifest` — fetches `web-0`'s full manifest to check its image,
   resource requests, probes, and volume mounts.
3. `k8s.listEvents` — pulls events involving `web-0` (`FailedScheduling`,
   `BackOff`, `Unhealthy`, and so on) to see what the scheduler and kubelet
   have already reported.
4. `k8s.podLogs` — tails the container's recent output for an application-
   level error or panic.

The property this sequence demonstrates: **no confirm dialog appears at any
point.** All four tools are classed read-only — none of them changes
cluster state or returns secret material — so they run straight through on
either transport, headless or not, with no `_confirm` field, no
`--mcp-allow-*` flag, and nothing for a human to click through. An agent can
run this entire investigation unattended.

## Worked example 2: a consented write

Prompt: *"Scale `web` to 3, then restart it."*

Unlike triage, this intends to change the cluster, so it walks through the
confirm gate twice — once per mutating call.

1. The agent calls `k8s.scale` with `{ "context": "...", "kind": "Deployment",
   "namespace": "...", "name": "web", "replicas": 3 }`. `k8s.scale` is classed
   **needs confirmation**, so the call blocks: in the GUI, a dialog appears
   asking you to approve scaling `web` to 3 replicas; headless, it would
   instead require `--mcp-allow-destructive` on the process plus
   `"_confirm": true` on this call.
   - **Approve** it, and the call proceeds — `k8s.scale` sets the replica
     count on the cluster and returns `{ "name": "web", "ok": true }`; it
     does not echo back the updated spec or replica count, so an agent that
     wants to confirm the new count needs a follow-up read (`k8s.getObject`
     or `k8s.listDeployments`).
2. The agent then calls `k8s.rolloutRestart` with `{ "context": "...", "kind":
   "Deployment", "namespace": "...", "name": "web" }`. This is a distinct
   confirm-gated call, not covered by the first approval — it pauses on its
   own dialog. **Approve** that too, and the rollout restart is triggered
   and returns success.

Now the denial path, which is the part worth being deliberate about. Say
the agent instead calls `k8s.rolloutRestart` and you **deny** the dialog (or
it times out, or no srelens window is open to show it at all). The call
does not crash the connection, and the server does not raise a transport or
protocol-level error — the agent gets back a normal tool result whose
content says the call was denied and why (e.g. that confirmation was
declined, or that the required `--mcp-allow-destructive` flag/`_confirm`
combination was missing). A reasonably built agent reads that text like any
other tool output: it can report back to the user that the restart was
declined, ask whether to try something else, or stop — rather than treating
the denial as an exception that aborts the whole interaction.

## Prompts

srelens ships a set of built-in MCP **prompts** — ready-made diagnostic flows
for an agent to run instead of improvising one, covering common failure
modes such as a crash-looping pod or a service with no endpoints. The full
list, with each one's description and arguments, is in
[mcp-catalog.md § Prompts](mcp-catalog.md#prompts). An MCP client that
supports prompts shows them in its prompt picker.

Every prompt takes one required argument, `context`; everything else is
optional. Naming the object it's about — `pod`, `node`, or `service` —
triages that specific object. Omitting it runs the prompt's discovery half
instead, which lists candidates first and lets you pick one. Built-ins only
diagnose: they read cluster state and end by recommending a `kubectl`
command for you to run yourself, never a call that changes anything — so
headless triage needs neither `--mcp-allow-destructive` nor
`--mcp-allow-sensitive-reads`.

**Writing your own prompts.** Drop `*.md` files into
`<app config dir>/mcp/prompts/`. Each file is a YAML front-matter header
followed by the prompt body:

```md
---
name: high-restart-count
description: Investigate a pod restarting more than expected
mode: targeted
priority: 10
arguments:
  - { name: context, required: true }
  - { name: pod, target: true, description: Pod to investigate }
---
Check `{{pod}}` on `{{context}}` for its restart count and recent events.
```

Front-matter fields:

- `name` — identifies the prompt. Give a targeted and a discover file the
  same `name` to offer both halves of one flow.
- `description` — shown to the client alongside the prompt.
- `mode` — `targeted` (default) or `discover`.
- `priority` — an integer; built-ins ship at `0`. On a name/mode collision
  the higher `priority` wins, and a built-in wins an equal-priority tie — so
  overriding a built-in prompt requires declaring a `priority` above `0`.
- `arguments` — a list of `{ name, description, required, target, default
  }`. `target` marks the argument whose presence switches the prompt into
  targeted mode (`pod` for the pod flows, `node` or `service` for the
  others). `default` fills in an omitted optional argument so no
  `{{token}}` survives into the rendered instructions.

Caps: at most 100 prompt files, 64 KB each. A body may only reference
`{{name}}` placeholders declared in `arguments` — pasting in helm-style
example text such as `{{ .Values.foo }}` is rejected as an undeclared
placeholder rather than rendered literally, and an unterminated `{{` with
no matching `}}` is rejected outright rather than left in the rendered
instructions.

Files that fail to load are listed in **Settings → MCP** with the reason,
and edits to a prompt file take effect immediately — no restart needed.

Retrieving a prompt is not itself an audited event — nothing touches a
cluster to fetch one. The tool calls an agent makes while following it are
audited exactly like any other MCP call.

## Resources

Alongside tools and prompts, srelens exposes cluster state as MCP
**resources** — addressable under `k8s://` URIs that a client can list,
read, and subscribe to for change notifications (on either transport — see
the subscription section below for how HTTP clients open the push stream).
The exact
fixed URIs and parameterised URI shapes are listed in
[mcp-catalog.md § Resources](mcp-catalog.md#resources); what matters here is
what they mean and how to use them.

A fixed URI addresses something that isn't a single cluster object — the
list of contexts srelens can connect to, or a dump of the whole tool/prompt/
resource catalog for a client that wants to introspect the server in one
read. Everything else addresses a single object by context, namespace, kind
and name, with an optional trailing segment for that object's events or
(Pod only) its logs. What each shape resolves to:

- the bare object URI reads its manifest as YAML;
- appending `/events` lists events whose involved object is that resource;
- appending `/logs` (Pod only) reads its recent log output, with the
  container name as an optional final segment.

Cluster-scoped kinds (`Node`, `PersistentVolume`, `ClusterRole`, and so on)
have no namespace — use `-` in that slot rather than leaving it blank.

`/logs` without a container works only for a single-container pod — that is
the one case the Kubernetes log API will serve without being told which
container you mean. For a pod with more than one container (a sidecar, say),
name the container explicitly; omitting it gets you an error naming every
container to choose from, not a guess at which one you meant.

**Secrets are not addressable.** A `k8s://.../Secret/...` read is refused
with an error naming the alternative: fetch secret data with the
`k8s.getSecret` tool instead, which is consent-gated. A resource is the kind
of thing a client fetches automatically to build context, with no
confirmation step in front of it, so routing Secret contents through that
path would quietly bypass the one control that exists to gate secret
material.

Every segment is percent-encoded, so a context name containing `/` or `:` —
an EKS cluster ARN, say — round-trips safely.

`resources/list` returns only the fixed entries. Enumerating every object in
a cluster would be unbounded, and would need a cluster round trip just to
answer a discovery call. Object addressing is discoverable instead through
`resources/templates/list`, which advertises the parameterised shapes above
as templates for a client to fill in.

A resource read is resolved to the same capability call a tool invocation
would make — `k8s.getManifest`, `k8s.listEvents`, `k8s.podLogs`, or
`k8s.listContexts` — so it goes through the identical path and is **audited
exactly like a tool call**, appearing in the same audit log under the
underlying capability's name, with the same redaction rules.

**Subscriptions work on both transports.** A client can send
`resources/subscribe` for an object URI and receive a
`notifications/resources/updated` message whenever that object changes; the
notification carries only the URI, and the client re-reads to get the new
content. Over stdio, notifications arrive on the server's stdout. Over HTTP
([issue #193](https://github.com/srelens/srelens/issues/193)), the client
first opens the push channel — `GET /mcp` with `Accept: text/event-stream`,
authenticated with the same bearer token and covered by the same loopback
`Host` check — and notifications arrive as SSE events on that stream.
Subscribing with no stream connected is refused rather than silently
accepted, since the notifications would have nowhere to go, and
subscriptions live exactly as long as the stream that carries them: a
disconnect, a reconnect replacing the stream, or a token rotation (which
restarts the server) releases every watch, and the client re-subscribes on
its new stream. Up to 32
subscriptions can be live at once; re-subscribing to a URI you already hold
replaces it in place rather than counting twice, and past the cap you need
to unsubscribe from something before adding another.
