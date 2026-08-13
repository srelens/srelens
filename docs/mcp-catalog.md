<!-- GENERATED FILE — do not edit by hand.
     Regenerate with: UPDATE_CATALOG=1 cargo test -p srelens-registry -->

# srelens MCP catalog

Everything this server exposes over MCP, generated from the live registry so it cannot drift. Written for someone wiring an agent to srelens; the narrative reference is [MCP.md](MCP.md).

## Tools (82)

Argument schemas are not reproduced here — call `tools/list` for those, which cannot go stale.

### Kubernetes — read-only (46)

| Tool | Summary |
| --- | --- |
| `k8s.bindingsForServiceAccount` | list the RoleBindings and ClusterRoleBindings that reference a ServiceAccount |
| `k8s.canI` | check whether the current user can perform actions (SelfSubjectAccessReview, batched) |
| `k8s.clusterInfo` | connect to a kube context and report server version and reachability |
| `k8s.diffManifest` | diff a manifest against the cluster via server dry-run apply (per document) |
| `k8s.getManifest` | fetch a resource's manifest as YAML (any supported kind) |
| `k8s.getObject` | fetch a resource as a structured JSON object (any supported kind) |
| `k8s.listCRDs` | list installed CustomResourceDefinitions (group, kind, plural, scope) |
| `k8s.listClusterRoleBindings` | list ClusterRoleBindings of a connected kube context (cluster-scoped) |
| `k8s.listClusterRoles` | list ClusterRoles of a connected kube context (cluster-scoped) |
| `k8s.listConfigMaps` | list ConfigMaps in a namespace of a connected kube context |
| `k8s.listContexts` | list the kube contexts available in the kubeconfig |
| `k8s.listCronJobs` | list CronJobs in a namespace of a connected kube context |
| `k8s.listCustomResource` | list instances of a custom resource by group/version/plural |
| `k8s.listDaemonSets` | list DaemonSets in a namespace of a connected kube context |
| `k8s.listDeployments` | list deployments in a namespace of a connected kube context |
| `k8s.listEndpointSlices` | list EndpointSlices in a namespace of a connected kube context |
| `k8s.listEvents` | list events in a connected kube context |
| `k8s.listIngresses` | list Ingresses in a namespace of a connected kube context |
| `k8s.listJobs` | list Jobs in a namespace of a connected kube context |
| `k8s.listLimitRanges` | list LimitRanges in a namespace of a connected kube context |
| `k8s.listNamespaces` | list namespaces in a connected kube context |
| `k8s.listNetworkPolicies` | list NetworkPolicies in a namespace of a connected kube context |
| `k8s.listNodes` | list the nodes of a connected kube context |
| `k8s.listPersistentVolumeClaims` | list PersistentVolumeClaims in a namespace of a connected kube context |
| `k8s.listPersistentVolumes` | list PersistentVolumes of a connected kube context (cluster-scoped) |
| `k8s.listPods` | list pods in a namespace of a connected kube context |
| `k8s.listReplicaSets` | list the ReplicaSets owned by a Deployment (its rollout revisions) |
| `k8s.listResource` | list any supported resource kind (name + namespace) |
| `k8s.listResourceQuotas` | list ResourceQuotas in a namespace of a connected kube context |
| `k8s.listRoleBindings` | list RoleBindings in a namespace of a connected kube context |
| `k8s.listRoles` | list Roles in a namespace of a connected kube context |
| `k8s.listSecrets` | list Secrets in a namespace (name, type, and key count only — no values) |
| `k8s.listServiceAccounts` | list ServiceAccounts in a namespace of a connected kube context |
| `k8s.listServices` | list services in a namespace of a connected kube context |
| `k8s.listStatefulSets` | list StatefulSets in a namespace of a connected kube context |
| `k8s.listStorageClasses` | list StorageClasses of a connected kube context (cluster-scoped) |
| `k8s.nodeMetrics` | node CPU/memory usage (requires metrics-server) |
| `k8s.openApiSchema` | fetch the OpenAPI schema for a resource kind (for field autocomplete) |
| `k8s.podLogs` | fetch recent logs for a pod in a connected kube context |
| `k8s.podMetrics` | pod CPU/memory usage (requires metrics-server) |
| `k8s.podsForPvc` | list pods in a namespace that mount a given PersistentVolumeClaim |
| `k8s.podsForSelector` | list pods matching a label selector (a workload's managed pods) |
| `k8s.podsForServiceAccount` | list pods in a namespace running as a given ServiceAccount |
| `k8s.synthesizeClusterKubeconfig` | synthesize a one-context kubeconfig from Add-cluster form fields |
| `k8s.testClusterConnection` | probe a kubeconfig context's server reachability (no exec plugins run) |
| `k8s.validateManifest` | validate a resource manifest against the API server (dry-run, strict) |

### Kubernetes — sensitive read (1)

| Tool | Summary |
| --- | --- |
| `k8s.getSecret` | read a Secret's values (sensitive; returns base64-encoded data) |

### Kubernetes — needs confirmation (7)

| Tool | Summary |
| --- | --- |
| `k8s.applyManifest` | server-side apply resource manifests (YAML, multi-doc); creates or updates |
| `k8s.cordonNode` | cordon or uncordon a node (set spec.unschedulable) |
| `k8s.cronjobSetSuspend` | suspend or resume a CronJob (set spec.suspend) |
| `k8s.cronjobTriggerNow` | run a CronJob immediately by creating a Job from its jobTemplate |
| `k8s.rolloutRestart` | trigger a rolling restart of a workload |
| `k8s.scale` | set the replica count of a workload (Deployment/StatefulSet/ReplicaSet) |
| `k8s.updateConfigData` | update ConfigMap or Secret values in place (merge patch) |

### Kubernetes — destructive (7)

| Tool | Summary |
| --- | --- |
| `k8s.createNodeDebugPod` | create a privileged debug pod on a node that nsenters into the host namespaces; delete it when the shell closes (destructive) |
| `k8s.debugPod` | attach an ephemeral debug container to a running pod and return its name; exec into that container for a debugger shell (destructive) |
| `k8s.deleteContext` | delete a context and its associated cluster and user from its kubeconfig source |
| `k8s.deletePod` | delete a pod in a connected kube context (destructive) |
| `k8s.deleteResource` | delete any supported resource by kind/namespace/name (destructive) |
| `k8s.drainNode` | cordon a node and evict its evictable pods (destructive) |
| `k8s.evictPod` | evict a pod via the eviction API (respects PodDisruptionBudgets) |

### Helm — read-only (5)

| Tool | Summary |
| --- | --- |
| `k8s.getHelmRelease` | fetch a Helm release's values, manifest, and revision history |
| `k8s.helmSearchRepo` | search configured Helm repos for a chart by name, resolving its full ref and available versions |
| `k8s.helmTemplate` | render a chart's manifests locally (helm template) for preview |
| `k8s.helmVersion` | report the installed Helm client version (detects whether helm is available) |
| `k8s.listHelmReleases` | list installed Helm releases (latest revision of each) |

### Helm — needs confirmation (5)

| Tool | Summary |
| --- | --- |
| `k8s.helmInstall` | install a Helm chart as a new release |
| `k8s.helmRepoAdd` | add a chart repository to the local Helm config |
| `k8s.helmRepoUpdate` | refresh the local cache of chart repositories |
| `k8s.helmRollback` | roll a Helm release back to a previous revision |
| `k8s.helmUpgrade` | upgrade an existing Helm release (new chart version and/or values) |

### Helm — destructive (1)

| Tool | Summary |
| --- | --- |
| `k8s.helmUninstall` | uninstall a Helm release |

### Toolbox — read-only (3)

| Tool | Summary |
| --- | --- |
| `toolbox.diagnoseContext` | diagnose a kube context's exec-auth tool requirements: which external tools it needs and whether each is installed, off the app PATH, or missing |
| `toolbox.searchPlugins` | search the krew index for kubectl plugins (name, description, installed) |
| `toolbox.status` | inventory the managed CLI toolchain (kubectl, krew, helm): whether each is installed, its path and version, and whether srelens manages it |

### Toolbox — needs confirmation (6)

| Tool | Summary |
| --- | --- |
| `toolbox.installHelm` | download the latest helm release into ~/.srelens/bin, verified against its published checksum |
| `toolbox.installKrew` | download the latest krew, verify it, and bootstrap it into ~/.krew (the engine for kubectl plugin installs) |
| `toolbox.installKubectl` | download the latest stable kubectl into ~/.srelens/bin, verified against the dl.k8s.io checksum |
| `toolbox.installPlugin` | install a kubectl plugin from the krew index |
| `toolbox.removePlugin` | remove an installed krew plugin |
| `toolbox.upgradePlugin` | upgrade an installed krew plugin |

### Server — read-only (1)

| Tool | Summary |
| --- | --- |
| `ping` | health check; echoes the input back as { pong: <input> } |

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

