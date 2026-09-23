# UI contributions

Everything an app shows is rendered by srelens with its own components, in both the new
and classic designs. User-facing extension management is named **Apps**; internal
`extensions.*` capability IDs, manifest IDs and routes keep their names. Field
reference: [manifest.md](manifest.md#contributions).

## Pages and navigation

Enabled apps with pages appear beneath **Apps → app name** in the connected cluster's
sidebar. Pages may declare `group` to nest their navigation. Page icons follow each
page's role (overview, sources, Helm, notifications) rather than repeating the app's
logo. App routes pin the cluster, so a page stays on the cluster it was opened for.

Namespace and search filters are scoped to the open cluster; they are temporary view
state, not saved preferences. Refresh explicitly repeats a read.

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
- **`statusColumns`** are deprecated but still accepted on the 0.3 line. They name
  printer-column indices for `ready`, and optionally `suspended` and `progressing`, and
  map onto the same statuses: suspended, then progressing, then Ready True (healthy) or
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
reads the cluster in focus by its stable ID, and has a **Refresh** action. Each card is
shows its figure or one of three visibly different states:

- **Loading**: a spinner and no figure.
- **Couldn't read**: the reason and a **Retry**, and no figure.
- **None**: the figure `0` (or *No value* for an empty minimum or maximum), drawn
  quieter than a count.

A `countByStatus` card counts by the app's status resolver for its source's kind, one
line per label, as the same objects' badges read on the app's list.

A card with a `target` opens that app page narrowed to what the card counted; the page
says so and offers the whole list. A card the app no longer declares is reported as
gone rather than shown as every row. Titles, app names and values render as plain text.

## Detail tabs and detail links

A Namespace's resource overview has an **Apps** section with the app's declared detail
views and an **App links** menu, scoped to that namespace. `detailLinks` open read
panels there; they do not write to the cluster.

## Requirement checks

When a page opens, the host checks the CRDs and served versions it needs, including
those of dashboard summaries.

- A missing CRD or version shows a requirements page with the exact API names and a
  **Check again** action. It does not uninstall or disable the app.
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

## Actions and refresh

The details footer offers the installed manifest's explicitly granted actions (see [capabilities.md](capabilities.md#declared-gitops-actions)). Each opens a
review naming the cluster and resource. The review keeps the UID and resourceVersion
the reader saw, even if another view refreshes the resource while it is open.

Acknowledgement says **Request accepted**, not that reconciliation completed. Every
open list, dashboard and detail view of that resource then refreshes.

Arbitrary custom renderer code is not supported.
