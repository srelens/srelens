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
- **Loopback HTTP** — `srelens --mcp-http 127.0.0.1:8765` (or the equivalent
  toggle in **Settings → MCP** of an already-running desktop app) exposes the
  same server on a local port, authenticated with a bearer token. Tool calls
  made this way run inside the same process as the GUI, sharing its
  authenticated cluster connections — and, crucially, its confirm dialog: a
  gated tool call over HTTP pops the same in-app approval prompt a click in
  the UI would, in whichever srelens window is open.

Pick stdio unless you specifically need calls to land in an already-running
GUI session — for example, so a human watching the desktop app can see and
approve an agent's confirm dialogs as they happen, or so the agent shares
cluster contexts the GUI already has open. Everything else — including both
worked examples below — works identically over either transport.

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
- A **Host header check** rejects requests whose `Host` isn't a loopback
  value (`127.0.0.1`, `::1`, or `localhost`). Binding loopback alone doesn't
  stop a page on another domain from resolving to 127.0.0.1 and posting to
  the port; the Host check does. It applies to every route, including the
  unauthenticated `/healthz`, so nothing here answers a caller that isn't
  genuinely local.
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
  | `--mcp-allow-sensitive-reads` | reads that return secret material, i.e. `k8s.getSecret` |

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
    would miss them.

  Identifying fields like `context`, `namespace`, `name` and `kind` survive,
  so you can still see which cluster and object an agent touched.
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
needs the matching `--mcp-allow-*` flag plus `_confirm: true` on that call.

Review tool calls and use appropriate Kubernetes RBAC, especially with
critical clusters.

## The catalog and its safety classes

[mcp-catalog.md](mcp-catalog.md) enumerates all 82 tools, the 4 built-in
prompts, and every resource URI, grouped by area (Kubernetes, Helm, Toolbox,
Server) and by **safety class**. There are exactly four:

| Class | Confirm gate? | Headless flag needed |
| --- | --- | --- |
| read-only | never | none |
| sensitive read | yes | `--mcp-allow-sensitive-reads` |
| needs confirmation | yes | `--mcp-allow-destructive` |
| destructive | yes | `--mcp-allow-destructive` |

A tool's safety class is what the security model above actually keys off:
read-only tools never raise a confirm dialog and need no flag headlessly;
the other three all gate on `_confirm: true` plus the app dialog (or, off
the GUI, the matching flag), differing only in which flag authorizes them
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

## Client configuration

Ready-to-paste snippets for Claude Desktop, Claude Code, a generic stdio
client, the headless consent flags, and connecting to a running GUI over
HTTP are all in
[mcp-catalog.md § Client configuration](mcp-catalog.md#client-configuration).
Copy the one that matches your client rather than hand-rolling it — Task 8's
test in this repository parses those exact blocks, so they stay accurate.

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

1. The agent calls `k8s.scale` with `{ "name": "web", "namespace": "...",
   "replicas": 3 }`. `k8s.scale` is classed **needs confirmation**, so the
   call blocks: in the GUI, a dialog appears asking you to approve scaling
   `web` to 3 replicas; headless, it would instead require
   `--mcp-allow-destructive` on the process plus `"_confirm": true` on this
   call.
   - **Approve** it, and the call proceeds — `k8s.scale` sets the replica
     count on the cluster and returns `{ "name": "web", "ok": true }`; it
     does not echo back the updated spec or replica count, so an agent that
     wants to confirm the new count needs a follow-up read (`k8s.getObject`
     or `k8s.listDeployments`).
2. The agent then calls `k8s.rolloutRestart` for `web`. This is a distinct
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

srelens ships a handful of MCP **prompts** — ready-made diagnostic flows for
an agent to run instead of improvising one. Four built-ins cover common
failure modes: `pod-crashloop`, `pod-pending`, `node-pressure`, and
`service-no-endpoints`. An MCP client that supports prompts shows them in
its prompt picker.

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
read, and (over stdio) subscribe to for change notifications.

Two URIs are fixed:

- `k8s://contexts` — the same contexts `k8s.listContexts` returns.
- `k8s://catalog` — every tool, prompt and resource template this server
  exposes, so a client can introspect the whole surface in one read.

Everything else addresses a single object, using one of these URI shapes:

```
k8s://<context>/<namespace>/<kind>/<name>
k8s://<context>/<namespace>/<kind>/<name>/events
k8s://<context>/<namespace>/Pod/<name>/logs
k8s://<context>/<namespace>/Pod/<name>/logs/<container>
```

The first reads the object's manifest as YAML; `/events` lists events whose
involved object is that resource; `/logs` reads a pod's recent log output.
Cluster-scoped kinds (`Node`, `PersistentVolume`, `ClusterRole`, and so on)
have no namespace — use `-` in that slot rather than leaving it blank.

`/logs` without a container works only for a single-container pod — that is
the one case the Kubernetes log API will serve without being told which
container you mean. For a pod with more than one container (a sidecar, say),
add the container as a sixth segment; omitting it gets you an error naming
every container to choose from, not a guess at which one you meant.

**Secrets are not addressable.** A `k8s://.../Secret/...` read is refused
with an error naming the alternative: fetch secret data with the
`k8s.getSecret` tool instead, which is consent-gated. A resource is the kind
of thing a client fetches automatically to build context, with no
confirmation step in front of it, so routing Secret contents through that
path would quietly bypass the one control that exists to gate secret
material.

Every segment is percent-encoded, so a context name containing `/` or `:` —
an EKS cluster ARN, say — round-trips safely.

`resources/list` returns only the two fixed entries above. Enumerating
every object in a cluster would be unbounded, and would need a cluster
round trip just to answer a discovery call. Object addressing is
discoverable instead through `resources/templates/list`, which advertises
the four URI shapes above as templates for a client to fill in.

A resource read is resolved to the same capability call a tool invocation
would make — `k8s.getManifest`, `k8s.listEvents`, `k8s.podLogs`, or
`k8s.listContexts` — so it goes through the identical path and is **audited
exactly like a tool call**, appearing in the same audit log under the
underlying capability's name, with the same redaction rules.

**Subscriptions work over stdio only.** A client can send
`resources/subscribe` for an object URI and receive a
`notifications/resources/updated` message whenever that object changes; the
notification carries only the URI, and the client re-reads to get the new
content. The HTTP transport is request/response only, with no channel for
the server to push a notification back, so it advertises `subscribe: false`
in `initialize` and answers `resources/subscribe` with an error rather than
silently accepting a subscription that can never fire. Pushing resource
updates to HTTP clients (via SSE or similar) is tracked as a follow-up in
[issue #193](https://github.com/srelens/srelens/issues/193). Up to 32
subscriptions can be live at once; re-subscribing to a URI you already hold
replaces it in place rather than counting twice, and past the cap you need
to unsubscribe from something before adding another.
