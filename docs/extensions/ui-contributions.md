# UI contributions

Everything an app shows is rendered by srelens with its own components, in both the new
and classic designs. User-facing extension management is named **Apps**; internal
`extensions.*` capability IDs, manifest IDs and routes keep their names. Field
reference: [manifest.md](manifest.md#contributions).

## Pages and navigation

Enabled apps with pages appear beneath **Apps → app name** in the connected cluster's
sidebar. Pages may declare `group` to nest their navigation. Page icons follow each
page's role (overview, sources, Helm, notifications) rather than repeating the app's
logo. App routes pin the cluster by its context key, so a page stays on the cluster it
was opened for, and two contexts that share a stable ID (#623) open two tabs (#695).

Namespace and search filters are scoped to the open cluster; they are temporary view
state, not saved preferences. Refresh explicitly repeats a read.

On the desktop, app pages are **live** ([#566](https://github.com/srelens/srelens/issues/566)):
they follow the kinds their readers list, through `watch` app streams
([streams.md](streams.md#watch)), and read again in place when one changes. Each says
which it is, in words: **Live**, **Connecting…**, **Reconnecting…** (a notice says what
is shown may be out of date until the watch lists afresh, and a warning rule marks it)
or **Not live** with why (the host ended or refused the watch; Refresh still reads).
Where the words are:

- A **resource page** shows them once, above its list.
- A **dashboard page** shows them on each summary, and its events section has its own
  notice when reconnecting or stopped.
- A **joined table column** follows the reader it joins through, and says so above the
  table when it is reconnecting or stopped.

The web app has no app streams yet, so there app pages read on open and on Refresh only,
and say **Not live**.

The Flux 0.2.0 example includes Overview, Kustomizations, Helm releases, Sources (Git
repositories, Helm repositories, Helm charts, Buckets, OCI repositories), Image
Automation (repositories, policies, update automations), and Notifications (alerts,
providers, receivers).

## Resource columns

App resource lists request `useCrdColumns: true` through `extensions.read`. The host
keeps the installed binding's API target and reads that CRD's
`additionalPrinterColumns` for the exact served version. Columns use the same JSONPath
renderer, priority filtering and Age deduplication as Custom Resources, and the
response carries the column definitions with the values so headers cannot drift from
their data.

If definition discovery fails, the binding's own `printerColumns` remain available
with an explicit error notice.

## Status and dashboards

- **`statusResolvers`** (#541) resolve each listed resource to one of six statuses —
  healthy, warning, error, progressing, suspended, unknown — by the app's first-hit
  rules, evaluated by the host on the whole object. App tables show a **Status**
  column: the rule's word in a toned badge, and its reason beside it as plain text.
  A kind with no resolver has no status column.
- **`statusColumns`** are deprecated but still accepted on the 0.3 and 0.4 lines. They
  name printer-column indices for `ready`, and optionally `suspended` and `progressing`,
  and map onto the same statuses: suspended, then progressing, then Ready True (healthy) or
  False (error). A missing or unknown Ready condition stays **Unknown**.
- **Dashboards** summarize the pages they reference by those six statuses, each listed
  with its word and count, zero included.
- **Badges** put an app's word on built-in rows — for example Flux or Argo CD ownership
  on Deployments — in a column named after the app. A row with no badge shows `—`; a
  badge the host could not answer shows *Couldn't read*, never `—`.
- **Dashboard events** filter by the involved object's API group, so an unrelated kind
  with the same name is excluded. The events section uses the workspace namespace and
  search controls. Counts reflect the namespace and are not changed by event search.

Failed reads keep their error and a retry; they never become zero-count summaries.

## Cluster dashboard cards

`dashboardCards` ([Manifest reference](manifest.md#dashboard-cards)) draw an **App
cards** band on the cluster overview, under the capacity strip, in the new design only.
The band follows the cluster's namespace selection — the one the resource lists use —
reads the cluster in focus by its pinned ID, and has a **Refresh** action. On the
desktop it is live too: each app's card readers are watched, in the one selected
namespace or, for several or none, in every namespace, and a change redraws that app's
figures in place. While a watch reconnects, the band says its figures may be out of
date, and a warning rule marks them. Until the cluster's namespaces are known, no watch
starts and the band says **Not live** with that reason, while its cards stay
**Loading**.
Each card
shows its figure or one of three visibly different states:

- **Loading**: a spinner and no figure.
- **Couldn't read**: the reason and a **Retry**, and no figure.
- **None**: the figure `0` (or *No value* for an empty minimum or maximum), drawn
  quieter than a count.

A `countByStatus` card counts by the app's status resolver for its source's kind, one
line per label, as the same objects' badges read on the app's list.

Until the cluster's namespaces are listed, cards stay loading and read nothing, so a
namespace-restricted credential is never shown a refusal for namespaces it was not
going to read; it then reads its one namespace.

A card with a `target` opens that app page narrowed to what the card counted, over the
same namespaces; the page names them, its namespace picker shows them, and it offers
the whole list. Choosing another namespace in that picker opens the plain page for it,
and the card's page keeps showing what the card counted. A card the app no longer
declares is reported as gone rather than shown as every row. Titles, app names and
values render as plain text.

## Detail tabs and detail links

A Namespace's resource overview has an **Apps** section with the app's declared detail
views and an **App links** menu, scoped to that namespace. `detailLinks` open read
panels there; they do not write to the cluster.

## Related resources

A resource's Inspector — the peek, the full tab, and an app resource's own
Inspector — has a **Related** section when an enabled app declares
[`resourceLinks`](manifest.md#resourcelinks) from its kind. Each row reads
"Managed by Application argocd/guestbook". A link needs only a declared reader
for its target kind, but a row opens the target only when one of the app's
`pages` is backed by that reader: it opens in the app's resource view, on a
route that carries the cluster and that page, whose reader pins the target's
group and kind, so two targets are always two tabs and opening one again
focuses its tab. With no such page the row names the target as plain text. A target the resource names but the cluster does not have
is listed as *not found*, without a link. A link the host could not answer shows
*Couldn't read* with its reason and a Retry; only when every link answered with
nothing does the section say *No related resources.*

A built-in target (API 0.5, #728) — a Service an HTTPRoute sends to, the Secret an
ExternalSecret writes — opens in the host's own Inspector, whose route follows the
rail. So it is a link only when the rail shows the cluster this Inspector is for;
from an app page pinned to another cluster it is named as plain text rather than
opening the rail cluster's namesake.

The section also appears on a resource of a link's **target** kind, and lists the
resources whose links name it, read from the same declarations (#728). They are
grouped under the relation as it reads from here — **Owns**, **Manages**,
**Exposes**, **Referenced by** — one row per resource ("Deployment team/api"), each
opening like a target does. A list the host read only 2,000 of says the resources
shown were found among the first 2,000 and there may be more; resources the link
could not be read on are counted, with the first one's reason; a reverse link that
failed shows *Couldn't read* and the Retry. None of these reads as *No related
resources.* The Inspector sends the resolvers the resource's identity and metadata
only, in both directions.

## Requirement checks

When a page opens, the host checks the CRDs and served versions it needs, including
those of dashboard summaries.

- A missing CRD or version shows a requirements page with the exact API names and a
  **Check again** action. It does not uninstall or disable the app.
- A reader that lists several `versions` is available when the cluster serves any of
  them. The page lists every accepted version in order of preference and, under
  **Reads**, the one this cluster resolves to, or *None served* (#547).
- A CRD discovery failure is reported as an unverifiable requirement and does not
  block reads the user's RBAC allows.
- Switching clusters never reuses another cluster's discovery result.

## Resource inspection

Click a resource row to open its overview in the shared Inspector, beside the list:
metadata, spec, conditions, status, labels and annotations, a read-only YAML manifest
in the shared CodeEditor, and the resource's events. **Open tab** promotes the details
to an independent resource tab. Existing manifests need no update.

Events are filtered by the resource's UID and shown newest first by when each was last
seen, including the latest occurrence of a recurring event series. At most 100 are
shown, and the panel says when older ones were left out. The host reads up to 10 pages
of at most 500 events each; if pages remain, the panel says how many events it read and
that it shows the newest of those, rather than claiming they are the latest. Event RBAC
failures are shown separately and keep the rest of the overview.

## Pod logs, commands and port-forwards

API 0.5 ([#567](https://github.com/srelens/srelens/issues/567)). An app's pod bindings
appear as a **pods** section of the Inspector, after the host's own sections and the
app's detail panels (`ExtensionPodSlot` in
`packages/ui-next/src/extensions/ExtensionPodTools.tsx`): on a resource of a kind a
binding's `resource` reader lists — an app's own custom resource, or a Deployment,
StatefulSet or DaemonSet — for that object's pods; and on a Pod in a namespace a pod
permission grants, for that pod. The section lists the pods the host says the binding
may reach, in the host's words for the scope ("Pods selected by Deployment web"), with a
container picker and a control per binding:

- **Logs** follows the container in a pane below, through the pod log view's own buffer
  and status (`useLogStream` with the app's log source), and says when it is
  reconnecting, when lines were dropped, and why it ended.
- **Run** opens the host confirmation first: the level, the host's sentence, the
  cluster, the pod, the container, the command argument by argument, and the app that
  asked. Nothing runs until **Run command**; the output then streams in, stderr marked
  in words, and the exit code ends it.
- **Forward** opens a local port the host picks and says which, and where it reaches.
  A binding that forwards through a Service lists the Services that reach a pod in
  scope instead.

Each section is one app stream view: closing the Inspector, switching resources or
closing the window ends every session it opened, forwards and their ports included. On
the web the section says that app streams run in the desktop app.

## Metrics, logs and traces from providers

API 0.5 ([#569](https://github.com/srelens/srelens/issues/569)). An app's providers
([manifest.md](manifest.md#metric-log-and-trace-providers)) are drawn by the host in two
places, in the new design:

- **Overviews.** On a Deployment, StatefulSet, DaemonSet or Pod a metric or trace
  provider is for, the Overview (and the Inspector's Details) gains a **Metrics and
  traces from apps** section after the app's detail panels (`ExtensionProviderSlot` in
  `packages/ui-next/src/extensions/ExtensionProviderSlot.tsx`): one range picker (15
  minutes to 7 days, an hour by default) and **Refresh**, then a
  [timeseries chart](native-components.md#timeseries) per metric provider and a table of
  traces per trace provider, each saying which app it is from. A panel asks when it
  opens, when the range changes and on Refresh, never on a timer. Loading, a failure
  (with its reason and Retry) and a query that matched nothing each look different; a
  failure is never drawn as an empty chart.
- **The log view.** On `/logs/<kind>/<namespace>/<name>`, when an installed, enabled
  app has a log provider for the kind and the cluster, the controls bar gains a
  **from** picker (the log source; the rail's Sources are the pods): **Kubernetes**,
  then each provider as "title · app". Choosing one
  follows it through the view's own buffer, pause, filters and readout (`useLogStream`'s
  `source`, from `packages/ui-next/src/extensions/logProviders.ts`), on an app stream
  view of its own that closes when the source changes or the screen closes. The
  **Previous instance** control is the cluster's and is not drawn for a provider. A
  provider's stream that ended says how, as an ending or a failure, with **Follow
  again**; a provider the inventory no longer offers says so, and the view follows
  Kubernetes.

The classic design does not draw providers. On the web there are none, since
`network.http` is desktop only.

## Actions and refresh

The details footer offers the installed manifest's explicitly granted actions (see [capabilities.md](capabilities.md#declared-gitops-actions)). Each opens a
review naming the cluster and resource. The review keeps the UID and resourceVersion
the reader saw, even if another view refreshes the resource while it is open.

Acknowledgement says **Request accepted**, not that reconciliation completed. Every
open list, dashboard and detail view of that resource then refreshes.

## Command palette

In the new design, typing `/` in the console lists enabled apps' `commands` (#544),
each labelled with the app's name, for example **Flux: Open Helm releases**. Page
commands sit under **Apps** and open the page on the cluster in focus; an app that is
disabled or not allowed on that cluster contributes nothing.

Action commands sit under **Action** and appear only while an app resource of the
command's kind is open in its own tab. Running one opens that tab's review — the same
host confirmation as the footer button, pinned to the UID and resourceVersion shown —
and nothing is written until it is confirmed there. An action the resource's
availability rules exclude, or a resource read without a UID and resourceVersion to
pin, says why instead; a failed read drops the request, so a later Retry does not open
a review. The classic palette does not list app
commands.

Arbitrary custom renderer code is not supported.
