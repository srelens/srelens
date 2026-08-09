# srelens User Guide

How to use srelens day to day — connecting to clusters, investigating
resources, and taking action. For installation see the
[installation guide](INSTALL.md); for building from source see the
[developer guide](DEVELOPMENT.md).

srelens organises work around three loops — **discover** what is running,
**understand** why it is behaving the way it is, and **operate** on it safely.
Every action below uses your local kubeconfig credentials and talks directly to
the Kubernetes API; nothing is routed through a hosted service. Destructive
actions are identified and ask for confirmation before they run.

## Contents

- [First launch and connecting to clusters](#first-launch-and-connecting-to-clusters)
- [Context identity and kubeconfig sources](#context-identity-and-kubeconfig-sources)
- [Browsing resources](#browsing-resources)
- [Inspecting a resource](#inspecting-a-resource)
- [Editing and applying YAML](#editing-and-applying-yaml)
- [Taking action on resources](#taking-action-on-resources)
- [Logs](#logs)
- [Terminals and shells](#terminals-and-shells)
- [Port forwarding](#port-forwarding)
- [Helm](#helm)
- [Toolbox: CLI toolchain and auth plugins](#toolbox-cli-toolchain-and-auth-plugins)
- [Metrics](#metrics)
- [Command palette](#command-palette)
- [Application logs](#application-logs)
- [MCP server for AI agents](#mcp-server-for-ai-agents)
- [Settings reference](#settings-reference)
- [Updating](#updating)

## First launch and connecting to clusters

On first launch srelens reads your default kubeconfig (and any files listed in
`KUBECONFIG`) and shows a searchable list of every context it finds. Click a
context to open that cluster.

Once a cluster is open, switch between clusters from the **cluster hotbar** — the
vertical strip of avatars on the far left. Remote clusters appear at the top; a
divider separates locally-detected development clusters (kind, k3d, minikube,
docker-desktop, and similar), which srelens classifies automatically. The bottom
of the hotbar has quick access to the light/dark toggle, the **toolbox**, and
**settings**.

Each avatar shows the context's initials (or a logo you choose) tinted with its
colour, so clusters stay visually distinct — see
[context identity](#context-identity-and-kubeconfig-sources) to customise them.

### Contexts that share a name

Kubeconfig files downloaded per cluster often reuse a generic context name such
as `default` or `kubernetes-admin@kubernetes`. Standard kubeconfig merging keeps
only the first of each name and silently drops the rest. srelens instead shows
**every** context: a name that is unique is shown as-is, and a name that appears
in more than one file is prefixed with its source file, for example
`kube_prod/default` and `kube_stage/default`. Each entry connects to the cluster
in its own file, so all of your clusters are reachable even when their context
names collide.

## Context identity and kubeconfig sources

Open **Settings → Contexts** to manage where contexts come from and how they
look.

**Kubeconfig sources.** Your default kubeconfig / `KUBECONFIG` is loaded first;
additional files merge in order. Use **Add files** to pick more kubeconfig files,
or **Paste** to paste raw YAML (with an optional name) — pasted configs are saved
into the srelens application directory. Each added file appears as a chip you can
remove. srelens watches these files and refreshes automatically when they change
on disk.

**Context identity.** Select a context to give it a recognisable identity without
touching your kubeconfig:

- **Display name** — shown in tabs, navigation, and the switcher.
- **Short label** — a 2–3 character tag.
- **Logo** — Initials, Cluster, Cloud, Shield, Database, Globe, or Custom (a URL
  or an uploaded image up to 512 KB).
- **Colour** — eight presets or a custom colour, to tell environments apart at a
  glance.

Drag the grip handle (or use the move buttons) to reorder contexts. Right-click a
context for **Reset identity** or **Remove context**. Removing a context edits the
kubeconfig on disk and is confirmation-gated.

## Browsing resources

The left sidebar groups every resource kind srelens can browse:

- **Cluster** — Overview, Nodes, Namespaces, Events
- **Workloads** — Pods, Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs,
  CronJobs
- **Config** — ConfigMaps, Secrets, Resource Quotas, Limit Ranges, autoscalers,
  Pod Disruption Budgets, Priority/Runtime Classes, Leases, webhook configs
- **Network** — Services, Endpoint Slices, Endpoints, Ingresses, Ingress Classes,
  Network Policies, Port Forwards
- **Storage** — Persistent Volume Claims, Persistent Volumes, Storage Classes
- **Access Control** — Service Accounts, Cluster Roles, Roles, and their bindings
- **Helm** — Helm Releases
- **Custom Resources** — your CRDs, discovered on open and grouped by API group

Lists that support it stream **live** and show a green **live** badge (or
**reconnecting…** during a blip); others refresh every few seconds. Pods also poll
CPU and memory when a metrics server is available.

**Search, columns, and namespaces.** Filter the current list with the search box,
choose which columns are visible with the column picker (remembered per kind), and
scope to one or more namespaces with the namespace selector (empty means all
namespaces). If your credentials can't list all namespaces, srelens scopes to the
namespaces you can see and tells you so.

**Bulk actions.** Select rows with their checkboxes to reveal a bar with **Delete**,
**Evict** (Pods), and **Rollout restart** (Deployments, StatefulSets, DaemonSets).
Each is confirmation-gated and reports partial failures.

Click any row to open its detail drawer.

## Inspecting a resource

The detail drawer has three tabs:

- **Overview** — a property grid plus kind-specific sections. A Pod shows its
  controller, node, IPs, QoS, restart counts, a conditions timeline, scheduling,
  volumes, and a card per container with **Logs** and **Exec** buttons and
  per-port **Forward** buttons. Workloads link to their revisions and pods.
  Owner references, nodes, PVCs, and secrets render as links you can follow.
- **YAML** — the live manifest with schema-aware editing (see
  [editing and applying YAML](#editing-and-applying-yaml)).
- **Events** — events scoped to this object, with warnings highlighted.

**Metrics.** For Pods, Nodes, and workload controllers a metrics panel shows live
CPU and memory sparklines with a 5m/10m/30m/1h range picker, when a metrics server
is installed.

**Secrets.** Secret values are masked; use **Reveal**/**Hide** per key. Secrets and
ConfigMaps can be edited inline — only the keys you changed are sent, and the edit
respects your RBAC permissions.

## Editing and applying YAML

The YAML editor (in the drawer, a full **Edit** tab, or the **New resource**
editor) provides CodeMirror editing with live schema validation and schema-driven
completion.

- The apply button reads **Apply** when editing an existing object and **Create**
  for a new one. It is disabled while the document is unchanged, empty, or denied
  by RBAC.
- Applying an edit asks for confirmation and uses server-side apply.
- Toggle **Changes** to run a dry-run diff and see a side-by-side comparison per
  document before you apply. A **Changed elsewhere** indicator warns you if the
  live object drifted while you were editing.
- If server-side apply reports a field-manager conflict, srelens shows a banner
  with a **Force apply** option.

To create something new, use the **New** button in a resource list or the
**New resource** editor and pick a **Template** — Blank, Deployment, Service,
ConfigMap, Secret, Ingress, or Namespace — as a starting point.

## Taking action on resources

Action buttons live in the detail drawer header and are preflighted against your
RBAC: an action you can't perform is disabled with the reason. Destructive actions
confirm before running.

**Pods** — Logs, Shell, Debug (attach an ephemeral debug container), Edit, Forward,
Evict, Delete. Evict and Delete confirm; Debug asks for an image (default
`busybox`) and an optional container whose process namespace to share, then opens a
shell into the new debug container.

**Deployments, StatefulSets, DaemonSets, ReplicaSets, Jobs** — Logs, Edit, Scale
(validated replica count), Restart (rollout restart, applied immediately), Delete.

**CronJobs** — Run now (create a one-off Job), Suspend/Resume, Edit, Delete.

**Services** — Forward.

**Nodes** — Cordon/Uncordon, Drain (cordons and evicts pods, leaving DaemonSet and
static pods), and Node shell (a privileged host-namespaced debug pod you get a
shell into; it is deleted when you close the terminal). Drain and Node shell
confirm.

## Logs

Open logs from a Pod (the **Logs** action, or a container's **Logs** button) or
from a workload (its **Logs** action, which fans out to every matching pod). Logs
open as a session in the **dock** at the bottom of the window. The toolbar gives
you:

- **Pod** and **Container** selectors (for workload logs) — pick one or view all,
  multiplexed together.
- **Tail lines** — 100 / 200 / 500 / 1000 / 5000 (default 200).
- **Since** — All time / Last 5m / 15m / 1h / 6h.
- **prev** — show logs from the *previous, crashed* container instance for
  post-crash triage. (Live tail is disabled while this is on, because the API
  can't follow a terminated container.)
- **ts** — prefix each line with a timestamp.
- **Search** — filter the buffer, with a match count.
- **Live tail** (play/pause) — stream new lines as they arrive; the status shows
  connecting / reconnecting / a green **live** dot.
- **Wrap** — wrap long lines.
- **Download** — save the current buffer to a `.log` file.
- **Download all containers** — save a full dump of every container of every
  in-scope pod, each section headed `==> pod/container <==`.

When several pods or containers share the view, each source is tinted with its own
colour, and klog-style `E`/`W`/`I` levels are coloured too. Large logs are
virtualised so scrollback stays responsive.

Post-crash triage is possible entirely from this view: turn on **prev** and **ts**
to read the crashed instance's timestamped logs.

## Terminals and shells

srelens opens interactive sessions in the dock:

- **Pod shell** — an interactive exec session into a container (xterm.js). Open it
  from a Pod's **Shell** action or a container card's **Exec** button. For
  multi-container pods you choose which container.
- **Ephemeral debug container** — the Pod **Debug** action attaches a debug
  container (default image `busybox`, optionally sharing another container's
  process namespace) and opens a shell into it — useful for debugging distroless
  or minimal images.
- **Local terminal** — your own shell on your machine, scoped to the current
  cluster. srelens points `kubectl` at a private, single-context kubeconfig so you
  can't accidentally act on another cluster. Open a new one from the dock's **+**
  button.
- **Node shell** — a privileged debug pod pinned to a node that enters the host
  namespaces (via `nsenter`), for node-level troubleshooting. Open it from a
  Node's **Node shell** action; the pod is deleted automatically when you close the
  terminal.

Dock sessions stay alive when you switch tabs, so scrollback and live streams are
preserved. Resize the dock by dragging its top edge; close individual sessions or
the whole dock.

## Port forwarding

Start a forward from a Pod's or Service's **Forward** action. Enter the remote
(container or service) port and, optionally, a fixed local port — leave it on
**auto** to let the OS choose. A Service is resolved to a backing pod for you.

Active forwards appear in the status-bar indicator (with **Copy** and **Stop** per
forward) and in the **Port Forwards** view, which lists forwards across every open
cluster. Each inbound connection is handled independently.

## Helm

Open **Helm Releases** from the sidebar. srelens reads installed releases directly
from their Kubernetes secrets, so listing and inspecting releases needs no Helm
binary. Write operations and chart queries shell out to your installed `helm`; if
it isn't on your PATH, srelens tells you and points you at the
[toolbox](#toolbox-cli-toolchain-and-auth-plugins) to install it.

- **List** — Name, Namespace, Chart, App version, Revision, Status, and Updated
  time, with namespace scoping, search, and a column picker.
- **Install** a chart, **Add repo**, or **Update repos** from the toolbar.
- Open a release to see its **Values**, rendered **Manifest**, **History**, and
  **Notes**, and to **Upgrade**, **Rollback** (to a chosen revision), or
  **Uninstall** it. Install and upgrade let you edit values as YAML and
  **Preview** the rendered diff before applying. Uninstall is confirmation-gated.

## Toolbox: CLI toolchain and auth plugins

Open the **toolbox** from the hotbar (wrench icon). It manages the command-line
tools srelens and your terminals rely on, installing them into `~/.srelens/bin`
without touching your system installs.

- **Tools** — install or inspect **kubectl**, **krew**, and **helm**. Cards show
  whether a tool is managed by srelens or found on your system, its version, and
  its path. Installs are checksum-verified.
- **Plugins** — search the krew index and install or remove kubectl plugins.
  Installing a plugin is confirmation-gated, since plugins run with your kubectl.
- **Context health** — check a context's authentication requirements. srelens
  detects the external tools a context's exec-auth needs (kubectl plugins such as
  `kubectl-oidc_login`, and cloud CLIs such as `aws`, `gcloud`,
  `gke-gcloud-auth-plugin`, `az`) and reports each as Found, Not on app PATH, or
  Missing — with an install button for the ones it can install for you.

## Metrics

When the cluster has a metrics server installed, srelens shows CPU and memory
usage in three places: live sparklines in a Pod, Node, or workload detail; a
cluster-wide CPU/memory summary in the status bar; and readiness and count tiles
on the cluster **Overview**. Without a metrics server these degrade quietly.

## Command palette

Press **Cmd/Ctrl-K** to open the command palette. It offers your **recent** picks,
**Go to** any resource view or CRD, and fuzzy search across resources by name
(pods, deployments, services, config, and more, indexed when you open the
palette). Selecting a view opens its tab; selecting a resource opens its detail.

## Application logs

srelens keeps its own rotating log file so you can diagnose problems after they
happen — a failed connection, an RBAC denial, an unexpected error. Open
**Settings → Application logs** to read it:

- Filter by **level** (ERROR / WARN / INFO / DEBUG / TRACE) or search the text.
- **Refresh** to re-read the file.
- Click the path to **copy** it, or **Reveal** to open the file in your OS file
  manager.

Lines are coloured by level. This is srelens's own log, separate from the
Kubernetes pod [logs](#logs) above.

## MCP server for AI agents

srelens is MCP-native: every action it can take is also available to MCP-capable
AI clients, using your locally authenticated cluster contexts. Open
**Settings → MCP** to:

- **Run the MCP server over loopback HTTP** — tick the box and choose a port
  (default 8765). srelens shows the URL to connect to. The server is loopback-only
  and shares the app's authenticated clients.
- **Install the srelens CLI** — symlinks the running app so clients can launch
  `srelens --mcp-stdio`.
- **Connect a client** — pick your client (Claude Code, Claude Desktop, Cursor,
  Codex, and others) and transport to get a ready-to-paste configuration snippet.
- **View the bearer token and audit log** — rotate or revoke the HTTP token,
  and review recent agent activity, from the same panel.

You can also start a server directly:

```sh
srelens --mcp-stdio
srelens --mcp-http 127.0.0.1:8765
```

Reads (listing pods, fetching manifests, tailing logs, and so on) run
immediately, with no prompt. Anything that changes cluster state, or reads back
secret material, is gated — but *how* depends on where the server is running:

- **In the app**, via the Settings → MCP toggle above: the call pauses and a
  confirm dialog asks you to approve it, exactly as clicking the same action in
  the UI would.
- **Headless**, via either command above: there is no window, so there is no
  dialog. A gated call is refused outright — unless you started the process
  with `--mcp-allow-destructive` or `--mcp-allow-sensitive-reads` *and* the
  call carries `"_confirm": true`, in which case it proceeds with no approval
  step at all. That combination is for deliberate unattended automation, not a
  way to reach a human.

Wiring an agent to srelens — transports, the security model, the full tool
catalog and worked examples — is documented in [MCP.md](MCP.md).

## Settings reference

**Settings** (gear icon in the hotbar) has seven sections:

1. **Appearance** — display mode (Dark / Light / System) and theme palette.
2. **Layout** — left navigation and right details panel widths, with a reset.
3. **Kubernetes** — default namespace for new tabs and the per-request timeout
   (raise it for large or slow clusters).
4. **Contexts** — kubeconfig sources and per-context identity
   ([above](#context-identity-and-kubeconfig-sources)).
5. **MCP** — the MCP server, CLI, and client configuration
   ([above](#mcp-server-for-ai-agents)).
6. **Application logs** — srelens's own log file
   ([above](#application-logs)).
7. **Updates** — version, release channel, and the in-app updater
   ([below](#updating)).

## Updating

Open **Settings → Updates** to check for and install new versions. Choose the
**Stable** channel for released versions or **Dev** for rolling pre-releases, then
**Check for updates**; when one is available you can download, install, and
restart in place.

If srelens was installed through a system package manager (for example the Arch
AUR package), the in-app updater steps aside and points you at your package
manager instead. AppImage and Windows builds always self-update.

## Questions and feedback

Have a question, hit a rough edge, or want to suggest a feature?

- Join the community on Reddit at [r/srelens](https://www.reddit.com/r/srelens/).
- File bugs and feature requests on
  [GitHub Issues](https://github.com/srelens/srelens/issues).
