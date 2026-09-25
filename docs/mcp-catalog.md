<!-- GENERATED FILE — do not edit by hand.
     Regenerate with: UPDATE_CATALOG=1 cargo test -p srelens-registry -->

# srelens MCP catalog

Everything this server exposes over MCP, generated from the live registry so it cannot drift. Written for someone wiring an agent to srelens; the narrative reference is [MCP.md](MCP.md).

## Tools (119)

Argument schemas are not reproduced here — call `tools/list` for those, which cannot go stale.

**Impact** is how much a successful call disturbs — `low`, `medium` or `high` — and is a different question from the section heading, which is how the call is gated. A capability that accepts several named operations carries the highest level any of them reaches; the per-operation level travels with the resource.

### Kubernetes — read-only (55)

| Tool | Impact | Summary |
| --- | --- | --- |
| `k8s.bindingsForServiceAccount` | low | list the RoleBindings and ClusterRoleBindings that reference a ServiceAccount |
| `k8s.canI` | low | check whether the current user can perform actions (SelfSubjectAccessReview, batched) |
| `k8s.clusterFacts` | low | report a cluster's provider, region and metrics-server availability |
| `k8s.clusterInfo` | low | connect to a kube context and report server version and reachability |
| `k8s.diffManifest` | low | diff a manifest against the cluster via server dry-run apply (per document) |
| `k8s.getCustomResource` | low | Inspect one custom resource and its events |
| `k8s.getManifest` | low | fetch a resource's manifest as YAML (any supported kind) |
| `k8s.getObject` | low | fetch a resource as a structured JSON object (any supported kind) |
| `k8s.listCRDs` | low | list installed CustomResourceDefinitions (group, kind, plural, scope) |
| `k8s.listClusterRoleBindings` | low | list ClusterRoleBindings of a connected kube context (cluster-scoped) |
| `k8s.listClusterRoles` | low | list ClusterRoles of a connected kube context (cluster-scoped) |
| `k8s.listConfigMaps` | low | list ConfigMaps in a namespace of a connected kube context |
| `k8s.listContexts` | low | list the kube contexts available in the kubeconfig |
| `k8s.listCronJobs` | low | list CronJobs in a namespace of a connected kube context |
| `k8s.listCustomResource` | low | list instances of a custom resource by group/version/plural |
| `k8s.listDaemonSets` | low | list DaemonSets in a namespace of a connected kube context |
| `k8s.listDeployments` | low | list deployments in a namespace of a connected kube context |
| `k8s.listEndpointSlices` | low | list EndpointSlices in a namespace of a connected kube context |
| `k8s.listEvents` | low | list events in a connected kube context |
| `k8s.listIngresses` | low | list Ingresses in a namespace of a connected kube context |
| `k8s.listJobs` | low | list Jobs in a namespace of a connected kube context |
| `k8s.listLimitRanges` | low | list LimitRanges in a namespace of a connected kube context |
| `k8s.listNamespaces` | low | list namespaces in a connected kube context |
| `k8s.listNetworkPolicies` | low | list NetworkPolicies in a namespace of a connected kube context |
| `k8s.listNodes` | low | list the nodes of a connected kube context |
| `k8s.listPersistentVolumeClaims` | low | list PersistentVolumeClaims in a namespace of a connected kube context |
| `k8s.listPersistentVolumes` | low | list PersistentVolumes of a connected kube context (cluster-scoped) |
| `k8s.listPods` | low | list pods in a namespace of a connected kube context |
| `k8s.listReplicaSets` | low | list the ReplicaSets owned by a Deployment (its rollout revisions) |
| `k8s.listResource` | low | list any supported resource kind (name + namespace) |
| `k8s.listResourceQuotas` | low | list ResourceQuotas in a namespace of a connected kube context |
| `k8s.listRoleBindings` | low | list RoleBindings in a namespace of a connected kube context |
| `k8s.listRoles` | low | list Roles in a namespace of a connected kube context |
| `k8s.listSecrets` | low | list Secrets in a namespace (name, type, and key count only — no values) |
| `k8s.listServiceAccounts` | low | list ServiceAccounts in a namespace of a connected kube context |
| `k8s.listServices` | low | list services in a namespace of a connected kube context |
| `k8s.listStatefulSets` | low | list StatefulSets in a namespace of a connected kube context |
| `k8s.listStorageClasses` | low | list StorageClasses of a connected kube context (cluster-scoped) |
| `k8s.nodeMetrics` | low | node CPU/memory usage (requires metrics-server) |
| `k8s.openApiSchema` | low | fetch the OpenAPI schema for a resource kind (for field autocomplete) |
| `k8s.podCount` | low | running vs total pod counts for a cluster, counted without listing pod bodies |
| `k8s.podLogs` | low | fetch logs for a pod in a connected kube context: the last 200 lines by default (tail_lines to change), or set all_lines to get everything the runtime still retains (can be large) |
| `k8s.podMetrics` | low | pod CPU/memory usage (requires metrics-server) |
| `k8s.podOverview` | low | pod totals, per-node counts and the pods that are not running, without listing pod bodies |
| `k8s.podsForPvc` | low | list pods in a namespace that mount a given PersistentVolumeClaim |
| `k8s.podsForSelector` | low | list pods matching a label selector (a workload's managed pods) |
| `k8s.podsForServiceAccount` | low | list pods in a namespace running as a given ServiceAccount |
| `k8s.podsOnNode` | low | list pods scheduled on a node across all namespaces |
| `k8s.prometheusDiscover` | low | find a Prometheus-compatible query API running in the cluster |
| `k8s.prometheusQuery` | low | run a PromQL instant query against an in-cluster Prometheus |
| `k8s.queryPodEndpoint` | low | Query an HTTP endpoint (such as /metrics, /healthz, or custom app endpoints) inside any running pod via an on-demand API port-forward tunnel. Automatically resolves pod by selector or name, auto-detects metric/service ports, and supports line filtering. |
| `k8s.synthesizeClusterKubeconfig` | low | synthesize a one-context kubeconfig from Add-cluster form fields |
| `k8s.testClusterConnection` | low | probe a kubeconfig context's server reachability (no exec plugins run) |
| `k8s.topologyGraph` | low | graph the ingresses, services, workloads and dependencies of one or more namespaces |
| `k8s.validateManifest` | low | validate a resource manifest against the API server (dry-run, strict) |

### Kubernetes — sensitive read (6)

| Tool | Impact | Summary |
| --- | --- | --- |
| `k8s.getSecret` | medium | read a Secret's values (sensitive; returns base64-encoded data) |
| `k8s.nodeJournalLogs` | medium | retrieve journalctl logs for a service on a node via SSH |
| `k8s.nodeRuntimeDiagnostics` | medium | run non-invasive host diagnostics (containers, dmesg, disk, memory, process) on a node via SSH |
| `k8s.nodeServiceStatus` | medium | check the status of a systemd service (e.g. rke2-server, kubelet) on a node via SSH |
| `k8s.podConnections` | medium | read the established TCP connections of pods, from their own /proc/net/tcp |
| `k8s.topologyProbe` | medium | the topology graph, plus each pod's open connections read over pods/exec (one exec per pod) |

### Kubernetes — needs confirmation (13)

| Tool | Impact | Summary |
| --- | --- | --- |
| `k8s.annotate` | medium | Write one fixed annotation on the reviewed resource, as an app's action declares it; requires confirmation |
| `k8s.applyManifest` | medium | server-side apply resource manifests (YAML, multi-doc); creates or updates |
| `k8s.cordonNode` | medium | cordon or uncordon a node (set spec.unschedulable) |
| `k8s.cronjobSetSuspend` | medium | suspend or resume a CronJob (set spec.suspend) |
| `k8s.cronjobTriggerNow` | medium | run a CronJob immediately by creating a Job from its jobTemplate |
| `k8s.mergePatch` | high | Send the fixed merge patch an app's action declares, past the host deny-list, to the reviewed resource; requires confirmation |
| `k8s.requestCordonNode` | medium | Request cordon or uncordon of the reviewed Node without eviction; requires confirmation |
| `k8s.requestRolloutRestart` | high | Request a rolling restart of the reviewed built-in workload; requires confirmation |
| `k8s.rolloutRestart` | medium | trigger a rolling restart of a workload |
| `k8s.scale` | medium | set the replica count of a workload (Deployment/StatefulSet/ReplicaSet) |
| `k8s.setFields` | medium | Set fixed spec fields on the reviewed resource, as an app's action declares them; requires confirmation |
| `k8s.setStatusCondition` | medium | Write one status condition on the reviewed resource through the status subresource, as an app's action declares it; requires confirmation |
| `k8s.updateConfigData` | medium | update ConfigMap or Secret values in place (merge patch) |

### Kubernetes — destructive (8)

| Tool | Impact | Summary |
| --- | --- | --- |
| `k8s.createNodeDebugPod` | high | create a privileged debug pod on a node that nsenters into the host namespaces; delete it when the shell closes (destructive) |
| `k8s.debugPod` | high | attach an ephemeral debug container to a running pod and return its name; exec into that container for a debugger shell (destructive) |
| `k8s.deleteContext` | high | delete a context and its associated cluster and user from its kubeconfig source |
| `k8s.deletePod` | high | delete a pod in a connected kube context (destructive) |
| `k8s.deleteResource` | high | delete any supported resource by kind/namespace/name (destructive) |
| `k8s.drainNode` | high | cordon a node and evict its evictable pods (destructive) |
| `k8s.evictPod` | high | evict a pod via the eviction API (respects PodDisruptionBudgets) |
| `k8s.nodeServiceRestart` | high | restart a system service (e.g. rke2-server, kubelet) on a node via SSH (confirm-gated) |

### Helm — read-only (5)

| Tool | Impact | Summary |
| --- | --- | --- |
| `k8s.getHelmRelease` | low | fetch a Helm release's values, manifest, and revision history |
| `k8s.helmSearchRepo` | low | search configured Helm repos for a chart by name, resolving its full ref and available versions |
| `k8s.helmTemplate` | low | render a chart's manifests locally (helm template) for preview |
| `k8s.helmVersion` | low | report the installed Helm client version (detects whether helm is available) |
| `k8s.listHelmReleases` | low | list installed Helm releases (latest revision of each) |

### Helm — needs confirmation (5)

| Tool | Impact | Summary |
| --- | --- | --- |
| `k8s.helmInstall` | medium | install a Helm chart as a new release |
| `k8s.helmRepoAdd` | medium | add a chart repository to the local Helm config |
| `k8s.helmRepoUpdate` | medium | refresh the local cache of chart repositories |
| `k8s.helmRollback` | medium | roll a Helm release back to a previous revision |
| `k8s.helmUpgrade` | medium | upgrade an existing Helm release (new chart version and/or values) |

### Helm — destructive (1)

| Tool | Impact | Summary |
| --- | --- | --- |
| `k8s.helmUninstall` | high | uninstall a Helm release |

### Toolbox — read-only (3)

| Tool | Impact | Summary |
| --- | --- | --- |
| `toolbox.diagnoseContext` | low | diagnose a kube context's exec-auth tool requirements: which external tools it needs and whether each is installed, off the app PATH, or missing |
| `toolbox.searchPlugins` | low | search the krew index for kubectl plugins (name, description, installed) |
| `toolbox.status` | low | inventory the managed CLI toolchain (kubectl, krew, helm): whether each is installed, its path and version, and whether srelens manages it |

### Toolbox — needs confirmation (6)

| Tool | Impact | Summary |
| --- | --- | --- |
| `toolbox.installHelm` | medium | download the latest helm release into ~/.srelens/bin, verified against its published checksum |
| `toolbox.installKrew` | medium | download the latest krew, verify it, and bootstrap it into ~/.krew (the engine for kubectl plugin installs) |
| `toolbox.installKubectl` | medium | download the latest stable kubectl into ~/.srelens/bin, verified against the dl.k8s.io checksum |
| `toolbox.installPlugin` | medium | install a kubectl plugin from the krew index |
| `toolbox.removePlugin` | medium | remove an installed krew plugin |
| `toolbox.upgradePlugin` | medium | upgrade an installed krew plugin |

### Server — read-only (13)

| Tool | Impact | Summary |
| --- | --- | --- |
| `extensions.catalog` | low | Browse the native extension catalog with a durable cache; never connects clusters |
| `extensions.catalogManifest` | low | Download and checksum-verify a catalog manifest for permission review; does not install it |
| `extensions.list` | low | List installed declarative extensions |
| `extensions.read` | low | Read a declared custom-resource contribution from an enabled extension |
| `extensions.resolveCards` | low | Resolve the cluster dashboard cards an enabled extension declares, each to a figure or the reason it has none |
| `extensions.resolveColumns` | low | Resolve native extension table columns and badges in one batch |
| `extensions.resolveLinks` | low | Resolve an app's resource relationship links for a resource Inspector |
| `extensions.resolvePanels` | low | Resolve declarative app panels for a resource Inspector |
| `extensions.resource` | low | Inspect the selected resource of an enabled app |
| `extensions.streams` | low | Report the open app streams in this process and the traffic each app has sent |
| `extensions.validate` | low | Check a declarative extension manifest exactly as installing it would and return every problem; does not install it |
| `ping` | low | health check; echoes the input back as { pong: <input> } |
| `settings.get` | low | read durable desktop settings; omit key to return the complete map |

### Server — needs confirmation (4)

| Tool | Impact | Summary |
| --- | --- | --- |
| `extension.secretStore` | medium | Set or clear a secret an app keeps in srelens's encrypted secrets vault; write-only, never returns a value; requires approval |
| `extensions.action` | high | Run a declared action on an app resource; requires explicit confirmation |
| `extensions.configure` | medium | Install, enable, remove or configure local extensions; requires approval |
| `settings.set` | medium | atomically write or remove durable desktop settings |

## Prompts (4)

| Prompt | Description | Arguments |
| --- | --- | --- |
| `node-pressure` | Triage a node reporting resource pressure | context (required), node |
| `pod-crashloop` | Work out why a pod keeps restarting | context (required), namespace, pod |
| `pod-pending` | Work out why a pod will not schedule | context (required), namespace, pod |
| `service-no-endpoints` | Work out why a service has no endpoints | context (required), namespace, service |

## Resources (2 fixed, 4 templates)

`resources/list` returns only these two:

| URI | Description |
| --- | --- |
| `k8s://contexts` | Contexts srelens can connect to, and which is current. |
| `k8s://catalog` | Every tool, prompt and resource template this server exposes. |

Object addressing is discovered through `resources/templates/list`:

| URI template | Description |
| --- | --- |
| `k8s://{context}/{namespace}/{kind}/{name}` | A resource's manifest as YAML. Use `-` as the namespace for cluster-scoped kinds. Secrets are not addressable — read them with the k8s.getSecret tool. |
| `k8s://{context}/{namespace}/{kind}/{name}/events` | Events whose involved object is this resource. |
| `k8s://{context}/{namespace}/Pod/{name}/logs` | Recent log output for a pod's default container. Omitting the container is only valid for a single-container pod. |
| `k8s://{context}/{namespace}/Pod/{name}/logs/{container}` | Recent log output for one named container. Required for a multi-container (e.g. sidecar) pod, where Kubernetes rejects a log request with no container named. |

## Client configuration

### Claude Desktop

Add to `claude_desktop_config.json`:

```json
{
  "mcpServers": {
    "srelens": {
      "command": "srelens",
      "args": ["--mcp-stdio"]
    }
  }
}
```

### Claude Code

```bash
claude mcp add srelens -- srelens --mcp-stdio
```

### Generic stdio client

Spawn the binary and speak newline-delimited JSON-RPC on stdin/stdout:

```json
{
  "command": "srelens",
  "args": ["--mcp-stdio"]
}
```

### Headless consent

Both transports refuse gated tools by default. To pre-authorise them for an unattended session, add `--mcp-allow-destructive` (mutations) or `--mcp-allow-sensitive-reads` (secret reads).

### HTTP transport

`--mcp-http <addr>` starts a **separate, headless** MCP server process — it does not attach to an already-running GUI, and will fail to bind if the GUI's own Settings → MCP toggle already holds the port. (To share the GUI's process and its in-app confirm dialog instead, use Settings → MCP → Run the MCP server in the running desktop app.) Either way, point an HTTP-capable client at the address with the bearer token as `Authorization: Bearer <token>` — read from `SRELENS_MCP_TOKEN` for the headless process, or from Settings → MCP for the in-app one; never from argv:

```json
{
  "mcpServers": {
    "srelens": {
      "url": "http://127.0.0.1:8765/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

