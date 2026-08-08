//! Live resource watching via kube-rs `watcher`, generic over the summary
//! type. Maintains an in-memory map and emits a full sorted snapshot on every
//! change, so the UI can replace its list without applying deltas itself.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::sync::Arc;

use futures::StreamExt;
use k8s_openapi::api::apps::v1::{DaemonSet, Deployment, StatefulSet};
use k8s_openapi::api::batch::v1::{CronJob, Job};
use k8s_openapi::api::core::v1::{
    ConfigMap, Event as CoreEvent, LimitRange, PersistentVolume, PersistentVolumeClaim, Pod,
    ResourceQuota, Secret, Service, ServiceAccount,
};
use k8s_openapi::api::discovery::v1::EndpointSlice;
use k8s_openapi::api::networking::v1::{Ingress, NetworkPolicy};
use k8s_openapi::api::rbac::v1::{ClusterRole, ClusterRoleBinding, Role, RoleBinding};
use k8s_openapi::api::storage::v1::StorageClass;
use kube::runtime::watcher::{Config, Event};
use kube::Api;
use serde::de::DeserializeOwned;

use crate::client_cache::ClientCache;
use crate::configmaps::{summarise as summarise_configmap, ConfigMapSummary};
use crate::cronjobs::{summarise as summarise_cronjob, CronJobSummary};
use crate::daemonsets::{summarise as summarise_daemonset, DaemonSetSummary};
use crate::endpointslices::{summarise as summarise_endpointslice, EndpointSliceSummary};
use crate::ingresses::{summarise as summarise_ingress, IngressSummary};
use crate::limitranges::{summarise as summarise_limitrange, LimitRangeSummary};
use crate::networkpolicies::{summarise as summarise_networkpolicy, NetworkPolicySummary};
use crate::persistentvolumes::{summarise as summarise_pv, PvSummary};
use crate::pvcs::{summarise as summarise_pvc, PvcSummary};
use crate::rolebindings::{
    summarise as summarise_rolebinding, summarise_cluster as summarise_clusterrolebinding,
    ClusterRoleBindingSummary, RoleBindingSummary,
};
use crate::roles::{
    summarise as summarise_role, summarise_cluster as summarise_clusterrole, ClusterRoleSummary,
    RoleSummary,
};
use crate::serviceaccounts::{summarise as summarise_serviceaccount, ServiceAccountSummary};
use crate::storageclasses::{summarise as summarise_storageclass, StorageClassSummary};
use crate::resourcequotas::{summarise as summarise_resourcequota, ResourceQuotaSummary};
use crate::secrets::{summarise as summarise_secret, SecretSummary};
use crate::deployments::{summarise as summarise_deployment, DeploymentSummary};
use crate::events::{summarise as summarise_event, EventSummary};
use crate::jobs::{summarise as summarise_job, JobSummary};
use crate::services::{summarise as summarise_service, ServiceSummary};
use crate::statefulsets::{summarise as summarise_statefulset, StatefulSetSummary};
use crate::workloads::{summarise_pod, PodSummary};

/// Normalised watch event over summaries (decoupled from kube-rs types so the
/// reducer is unit-testable).
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WatchEvent<T> {
    Init,
    InitApply(T),
    InitDone,
    Apply(T),
    Delete(String),
}

/// Apply an event to the map. Returns `true` when a snapshot should be emitted.
pub(crate) fn reduce<T>(
    state: &mut BTreeMap<String, T>,
    key_of: &impl Fn(&T) -> String,
    event: WatchEvent<T>,
) -> bool {
    match event {
        WatchEvent::Init => {
            state.clear();
            false
        }
        WatchEvent::InitApply(item) => {
            state.insert(key_of(&item), item);
            false
        }
        WatchEvent::InitDone => true,
        WatchEvent::Apply(item) => {
            state.insert(key_of(&item), item);
            true
        }
        WatchEvent::Delete(key) => {
            state.remove(&key);
            true
        }
    }
}

/// Current snapshot as a name-sorted vector.
pub fn snapshot<T: Clone>(state: &BTreeMap<String, T>) -> Vec<T> {
    state.values().cloned().collect()
}

/// Connection health of a watch, surfaced to the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchStatus {
    Live,
    Reconnecting,
}

impl WatchStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            WatchStatus::Live => "live",
            WatchStatus::Reconnecting => "reconnecting",
        }
    }
}

/// A watcher error that will never self-heal (an RBAC Forbidden denial) —
/// surface it and stop, instead of reconnecting forever.
fn is_permanent_watch_error(msg: &str) -> bool {
    // Only a Forbidden (RBAC) denial is stable and won't self-heal. 401/token
    // expiry IS recoverable (kube-rs refreshes creds and re-lists), so we keep
    // reconnecting on it; and matching bare "401"/"403" substrings risks false
    // positives (e.g. an id or byte count containing those digits).
    msg.to_ascii_lowercase().contains("forbidden")
}

/// Generic watch loop: stream `K`, summarise to `T`, key by `key_of`, call
/// `on_update` with a full snapshot on every change, and `on_status` on
/// connection transitions.
///
/// kube-rs `watcher()` is a self-healing infinite stream — on API errors it
/// yields an `Err` item but keeps running and re-lists on the next poll. So we
/// consume with `next()` (not `try_next()?`) and, instead of terminating on the
/// first error, surface `Reconnecting` and carry on until it recovers.
async fn watch_typed<K, T, S, N, F, G>(
    api: Api<K>,
    summarise: S,
    key_of: N,
    mut on_update: F,
    mut on_status: G,
) -> Result<(), String>
where
    K: kube::Resource + Clone + DeserializeOwned + Debug + Send + 'static,
    K::DynamicType: Default + Eq + Hash + Clone + Debug + Unpin,
    T: Clone,
    S: Fn(K) -> T,
    N: Fn(&T) -> String,
    F: FnMut(Vec<T>),
    G: FnMut(WatchStatus),
{
    let mut state: BTreeMap<String, T> = BTreeMap::new();
    let mut stream = kube::runtime::watcher(api, Config::default()).boxed();
    let mut reconnecting = false;
    while let Some(item) = stream.next().await {
        match item {
            Ok(event) => {
                if reconnecting {
                    reconnecting = false;
                    on_status(WatchStatus::Live);
                }
                let mapped = match event {
                    Event::Init => WatchEvent::Init,
                    Event::InitApply(obj) => WatchEvent::InitApply(summarise(obj)),
                    Event::InitDone => WatchEvent::InitDone,
                    Event::Apply(obj) => WatchEvent::Apply(summarise(obj)),
                    Event::Delete(obj) => WatchEvent::Delete(key_of(&summarise(obj))),
                };
                if reduce(&mut state, &key_of, mapped) {
                    on_update(snapshot(&state));
                }
            }
            Err(e) => {
                let msg = e.to_string();
                if is_permanent_watch_error(&msg) {
                    // Auth/authz failures never self-heal: stop and surface the
                    // error so the caller can emit it to the UI.
                    return Err(msg);
                }
                // Transient: the watcher backs off and re-lists internally. Flag
                // the UI once per outage, then keep consuming.
                if !reconnecting {
                    reconnecting = true;
                    on_status(WatchStatus::Reconnecting);
                }
            }
        }
    }
    Ok(())
}

/// Watch pods in a namespace.
pub async fn watch_pods<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<PodSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<Pod> = crate::scoped_api(client, &namespace);
    watch_typed(api, summarise_pod, |p: &PodSummary| p.name.clone(), on_update, on_status).await
}

/// Watch deployments in a namespace.
pub async fn watch_deployments<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<DeploymentSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<Deployment> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_deployment,
        |d: &DeploymentSummary| d.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch StatefulSets in a namespace.
pub async fn watch_statefulsets<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<StatefulSetSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<StatefulSet> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_statefulset,
        |s: &StatefulSetSummary| s.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch DaemonSets in a namespace.
pub async fn watch_daemonsets<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<DaemonSetSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<DaemonSet> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_daemonset,
        |d: &DaemonSetSummary| d.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch Jobs in a namespace.
pub async fn watch_jobs<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<JobSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<Job> = crate::scoped_api(client, &namespace);
    watch_typed(api, summarise_job, |j: &JobSummary| j.name.clone(), on_update, on_status).await
}

/// Watch CronJobs in a namespace.
pub async fn watch_cronjobs<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<CronJobSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<CronJob> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_cronjob,
        |c: &CronJobSummary| c.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch ConfigMaps in a namespace.
pub async fn watch_configmaps<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<ConfigMapSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<ConfigMap> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_configmap,
        |c: &ConfigMapSummary| c.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch Secrets in a namespace (type + key count only — no values).
pub async fn watch_secrets<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<SecretSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<Secret> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_secret,
        |s: &SecretSummary| s.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch ResourceQuotas in a namespace.
pub async fn watch_resourcequotas<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<ResourceQuotaSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<ResourceQuota> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_resourcequota,
        |r: &ResourceQuotaSummary| r.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch LimitRanges in a namespace.
pub async fn watch_limitranges<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<LimitRangeSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<LimitRange> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_limitrange,
        |l: &LimitRangeSummary| l.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch services in a namespace.
pub async fn watch_services<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<ServiceSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<Service> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_service,
        |s: &ServiceSummary| s.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch events in a namespace — a true stream, replacing the poll.
pub async fn watch_events<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<EventSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<CoreEvent> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_event,
        |e: &EventSummary| e.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch Ingresses in a namespace.
pub async fn watch_ingresses<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<IngressSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<Ingress> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_ingress,
        |i: &IngressSummary| i.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch EndpointSlices in a namespace.
pub async fn watch_endpointslices<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<EndpointSliceSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<EndpointSlice> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_endpointslice,
        |e: &EndpointSliceSummary| e.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch NetworkPolicies in a namespace.
pub async fn watch_networkpolicies<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<NetworkPolicySummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<NetworkPolicy> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_networkpolicy,
        |n: &NetworkPolicySummary| n.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch PersistentVolumeClaims in a namespace.
pub async fn watch_pvcs<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<PvcSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<PersistentVolumeClaim> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_pvc,
        |p: &PvcSummary| p.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch cluster PersistentVolumes (cluster-scoped; namespace ignored).
pub async fn watch_persistentvolumes<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    _namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<PvSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<PersistentVolume> = Api::all(client);
    watch_typed(
        api,
        summarise_pv,
        |p: &PvSummary| p.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch cluster StorageClasses (cluster-scoped; namespace ignored).
pub async fn watch_storageclasses<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    _namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<StorageClassSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<StorageClass> = Api::all(client);
    watch_typed(
        api,
        summarise_storageclass,
        |s: &StorageClassSummary| s.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch ServiceAccounts in a namespace.
pub async fn watch_serviceaccounts<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<ServiceAccountSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<ServiceAccount> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_serviceaccount,
        |s: &ServiceAccountSummary| s.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch Roles in a namespace.
pub async fn watch_roles<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<RoleSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<Role> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_role,
        |r: &RoleSummary| r.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch cluster ClusterRoles (cluster-scoped; namespace ignored).
pub async fn watch_clusterroles<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    _namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<ClusterRoleSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<ClusterRole> = Api::all(client);
    watch_typed(
        api,
        summarise_clusterrole,
        |r: &ClusterRoleSummary| r.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch RoleBindings in a namespace.
pub async fn watch_rolebindings<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<RoleBindingSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<RoleBinding> = crate::scoped_api(client, &namespace);
    watch_typed(
        api,
        summarise_rolebinding,
        |r: &RoleBindingSummary| r.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Watch cluster ClusterRoleBindings (cluster-scoped; namespace ignored).
pub async fn watch_clusterrolebindings<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    _namespace: String,
    on_update: F,
    on_status: G,
) -> Result<(), String>
where
    F: FnMut(Vec<ClusterRoleBindingSummary>) + Send,
    G: FnMut(WatchStatus) + Send,
{
    let client = cache.get(&context).await?;
    let api: Api<ClusterRoleBinding> = Api::all(client);
    watch_typed(
        api,
        summarise_clusterrolebinding,
        |r: &ClusterRoleBindingSummary| r.name.clone(),
        on_update,
        on_status,
    )
    .await
}

/// Classify a watcher event as an object change. `Apply`/`Delete` always are.
/// `Init`/`InitApply` never are — they are just the replay of the current list
/// building up in memory, with nothing yet to report. `InitDone` — which
/// closes *every* init sequence, the very first list and every subsequent
/// relist alike — always is a change.
///
/// That includes the first list deliberately. A subscribe is never atomic
/// with the read that precedes it: the client reads the object at version A,
/// the object changes to B, and only then does the client subscribe. If the
/// initial list stayed silent, the client would be stuck believing A is
/// current until some unrelated later change happened to notify it — a lost
/// update with no bound on how long it lasts. Notifying on the first list
/// costs one redundant re-read when nothing actually changed between the read
/// and the subscribe; that is the correct direction to be wrong in. This
/// module's documented contract is that the signal may over-notify but must
/// never under-notify, and firing on every `InitDone` — not just relists — is
/// what makes that hold.
///
/// kube-runtime also runs additional init sequences (`Init` → `InitApply`* →
/// `InitDone`) whenever it re-lists after a desync (410 GONE, etcd
/// compaction, apiserver restart), and a change that lands during that
/// reconnect gap arrives only as part of the replayed list, never as a
/// standalone `Apply`. Firing on every `InitDone` covers that case too, for
/// the same reason.
///
/// Do not special-case the first `InitDone` back to silent "for tidiness" —
/// that reintroduces the lost-update window this rule closes.
///
/// Pulled out as a pure function (rather than inlined in `watch_object`)
/// because `watch_object` itself cannot be unit-tested without an API server,
/// and getting this classification wrong would either under-notify from
/// missing a change made before a subscribe or during a relist, or spuriously
/// notify on a relist that changed nothing.
fn is_object_change<K>(event: &Event<K>) -> bool {
    match event {
        Event::Apply(_) | Event::Delete(_) => true,
        Event::Init | Event::InitApply(_) => false,
        Event::InitDone => true,
    }
}

/// Watch a single object by name, calling `on_change` on every apply, delete,
/// or relist-that-changed-something.
///
/// One API watch per object via a `metadata.name` field selector, rather than
/// streaming a whole namespace to observe one member. `on_change` takes no
/// payload deliberately: the MCP subscription it backs sends only a URI and the
/// client re-reads, so this needs to detect *that* something changed.
///
/// **Every list notifies, initial and relists alike:** each init sequence
/// (`Init` → `InitApply`* → `InitDone`) fires `on_change` once, on its
/// `InitDone` — see `is_object_change`. The initial list is included
/// deliberately: a read followed by a subscribe is not atomic, so the object
/// can already differ from what the client last saw by the time the
/// subscription's first list completes, and staying silent would leave the
/// client trusting stale state with nothing to correct it. A redundant
/// notification when nothing actually changed is the correct failure
/// direction, not a bug to tidy away. The same rule also covers a relist after
/// a watch desync, which is the only way a change made during the reconnect
/// gap ever arrives.
///
/// **Namespace contract:** `namespace: None` means the caller has already
/// established that `gvk` is cluster-scoped. This function cannot detect a
/// kind's scope itself, so passing `None` for a *namespaced* kind will watch
/// with `Api::all_with` and the bare `metadata.name={name}` selector will
/// match same-named objects in every namespace, silently firing `on_change`
/// for a different object than the one the caller subscribed to. Establishing
/// scope before calling is the caller's responsibility.
pub async fn watch_object<F, G>(
    cache: Arc<ClientCache>,
    context: String,
    namespace: Option<String>,
    gvk: kube::api::GroupVersionKind,
    name: String,
    mut on_change: F,
    mut on_status: G,
) -> Result<(), String>
where
    F: FnMut() + Send,
    G: FnMut(WatchStatus) + Send,
{
    use kube::api::{ApiResource, DynamicObject};

    if name.is_empty() {
        return Err(
            "watch_object: object name must not be empty (an empty metadata.name \
             selector matches nothing, so the watch would sit forever without firing)"
                .into(),
        );
    }
    if let Some(bad) = name.chars().find(|c| *c == ',' || *c == '=') {
        return Err(format!(
            "watch_object: object name {name:?} contains '{bad}', which is not valid in \
             a Kubernetes object name and would corrupt the metadata.name field selector"
        ));
    }
    if namespace.as_deref() == Some("") {
        return Err(
            "watch_object: namespace must not be an empty string; pass None for a \
             cluster-scoped kind, not Some(\"\") — an empty namespace would silently \
             widen the watch to Api::all_with, matching same-named objects in every \
             namespace"
                .into(),
        );
    }

    let client = cache.get(&context).await?;
    let ar = ApiResource::from_gvk(&gvk);
    let api: Api<DynamicObject> = match namespace.as_deref() {
        Some(ns) => Api::namespaced_with(client, ns, &ar),
        None => Api::all_with(client, &ar),
    };

    let config = Config::default().fields(&format!("metadata.name={name}"));
    let mut stream = kube::runtime::watcher(api, config).boxed();
    let mut reconnecting = false;

    while let Some(item) = stream.next().await {
        match item {
            Ok(event) => {
                if reconnecting {
                    reconnecting = false;
                    on_status(WatchStatus::Live);
                }
                if is_object_change(&event) {
                    on_change();
                }
            }
            Err(e) => {
                let msg = e.to_string();
                if is_permanent_watch_error(&msg) {
                    return Err(msg);
                }
                if !reconnecting {
                    reconnecting = true;
                    on_status(WatchStatus::Reconnecting);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pod(name: &str, phase: &str) -> PodSummary {
        PodSummary {
            name: name.into(),
            namespace: "default".into(),
            phase: phase.into(),
            ready: "1/1".into(),
            restarts: 0,
            node: "n".into(),
            age: "1m".into(),
        }
    }

    fn key(p: &PodSummary) -> String {
        p.name.clone()
    }

    #[test]
    fn init_sequence_emits_only_on_init_done() {
        let mut state = BTreeMap::new();
        assert!(!reduce(&mut state, &key, WatchEvent::Init));
        assert!(!reduce(&mut state, &key, WatchEvent::InitApply(pod("a", "Running"))));
        assert!(!reduce(&mut state, &key, WatchEvent::InitApply(pod("b", "Pending"))));
        assert!(reduce(&mut state, &key, WatchEvent::InitDone));
        let snap = snapshot(&state);
        assert_eq!(snap.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), vec!["a", "b"]);
    }

    #[test]
    fn apply_upserts_and_emits() {
        let mut state = BTreeMap::new();
        reduce(&mut state, &key, WatchEvent::InitApply(pod("a", "Pending")));
        reduce(&mut state, &key, WatchEvent::InitDone);
        assert!(reduce(&mut state, &key, WatchEvent::Apply(pod("a", "Running"))));
        assert_eq!(snapshot(&state)[0].phase, "Running");
    }

    #[test]
    fn delete_removes_and_emits() {
        let mut state = BTreeMap::new();
        reduce(&mut state, &key, WatchEvent::Apply(pod("a", "Running")));
        reduce(&mut state, &key, WatchEvent::Apply(pod("b", "Running")));
        assert!(reduce(&mut state, &key, WatchEvent::Delete("a".into())));
        let snap = snapshot(&state);
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].name, "b");
    }

    /// `watch_object` cannot be exercised without an API server, so this pins
    /// the part that is pure: a permanent error must stop the loop rather than
    /// reconnect forever, and everything else must be treated as transient.
    #[test]
    fn only_forbidden_is_a_permanent_watch_error_for_object_watches() {
        assert!(is_permanent_watch_error("Forbidden: User cannot watch pods"));
        assert!(is_permanent_watch_error("FORBIDDEN"));
        assert!(!is_permanent_watch_error("connection reset by peer"));
        assert!(!is_permanent_watch_error("401 Unauthorized"));
        assert!(!is_permanent_watch_error("too old resource version"));
    }

    #[tokio::test]
    async fn watch_object_surfaces_a_client_error_instead_of_panicking() {
        // No such context, so `cache.get` fails before any watch begins.
        let cache = ClientCache::new(std::path::PathBuf::from("/nonexistent/kubeconfig"));
        let gvk = kube::api::GroupVersionKind::gvk("", "v1", "Pod");
        let err = watch_object(
            cache,
            "no-such-context".into(),
            Some("ns".into()),
            gvk,
            "web-0".into(),
            || {},
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(!err.is_empty(), "the client failure must be surfaced as an error string");
    }

    #[tokio::test]
    async fn watch_object_rejects_an_empty_name_before_touching_the_client() {
        // An empty `metadata.name=` selector matches nothing, so the watch
        // would sit forever without ever firing `on_change` or erroring — the
        // worst possible failure shape for a subscription. Reject up front.
        // Uses the same nonexistent kubeconfig as the client-error test above;
        // if this reached `cache.get` it would still error, but for the wrong
        // reason, so the assertion pins the *message*, not just `is_err()`.
        let cache = ClientCache::new(std::path::PathBuf::from("/nonexistent/kubeconfig"));
        let gvk = kube::api::GroupVersionKind::gvk("", "v1", "Pod");
        let err = watch_object(
            cache,
            "no-such-context".into(),
            Some("ns".into()),
            gvk,
            "".into(),
            || {},
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(err.contains("empty"), "expected an empty-name error, got: {err}");
    }

    #[tokio::test]
    async fn watch_object_rejects_a_name_containing_a_selector_metacharacter() {
        // A `,` or `=` in `name` reshapes `metadata.name={name}` into extra or
        // malformed field-selector terms. Neither character is valid in a real
        // Kubernetes object name (RFC 1123), so reject both before any client
        // work rather than silently producing a selector that matches the
        // wrong thing (or nothing).
        let gvk = kube::api::GroupVersionKind::gvk("", "v1", "Pod");
        for bad_name in ["web-0,evil=1", "web=0"] {
            let cache = ClientCache::new(std::path::PathBuf::from("/nonexistent/kubeconfig"));
            let err = watch_object(
                cache,
                "no-such-context".into(),
                Some("ns".into()),
                gvk.clone(),
                bad_name.into(),
                || {},
                |_| {},
            )
            .await
            .unwrap_err();
            assert!(
                err.contains(bad_name),
                "expected the offending name {bad_name:?} to be named in the error, got: {err}"
            );
        }
    }

    #[tokio::test]
    async fn watch_object_rejects_an_empty_namespace_instead_of_widening_the_watch() {
        // `ResourceUri::parse` cannot currently produce `Some("")` (the `-`
        // sentinel maps to `None`), so this is defence at the layer that
        // documents the contract, not a live bug from the URI path. But
        // silently coercing `Some("")` into `Api::all_with` would be exactly
        // the cross-namespace wrong-object watch this function's own doc
        // comment warns about, so it must be rejected like the empty-name and
        // metacharacter cases above rather than swallowed.
        let cache = ClientCache::new(std::path::PathBuf::from("/nonexistent/kubeconfig"));
        let gvk = kube::api::GroupVersionKind::gvk("", "v1", "Pod");
        let err = watch_object(
            cache,
            "no-such-context".into(),
            Some("".into()),
            gvk,
            "web-0".into(),
            || {},
            |_| {},
        )
        .await
        .unwrap_err();
        assert!(err.contains("namespace"), "expected a namespace error, got: {err}");
    }

    /// `Event` is generic; `()` is the cheapest `K` that still lets us
    /// construct every variant, since `is_object_change` never inspects the
    /// payload. `Apply`/`Delete` and `InitDone` are always changes;
    /// `Init`/`InitApply` never are.
    #[test]
    fn is_object_change_fires_on_apply_delete_and_init_done() {
        assert!(!is_object_change(&Event::<()>::Init));
        assert!(!is_object_change(&Event::InitApply(())));
        assert!(is_object_change(&Event::Apply(())));
        assert!(is_object_change(&Event::Delete(())));
        assert!(is_object_change(&Event::<()>::InitDone));
    }

    /// The relist scenario this signal exists to cover: a change that lands
    /// during the reconnect gap arrives only as part of the replayed init
    /// sequence, never as a standalone `Apply`. Both the first list and the
    /// relist must report a change, once each, on their respective
    /// `InitDone` — mirroring how `reduce` emits once per list.
    #[test]
    fn every_init_sequence_including_the_first_is_reported_as_one_change() {
        let mut changes = 0;

        for event in [
            Event::<()>::Init,
            Event::InitApply(()),
            Event::InitDone,
            Event::Init,
            Event::InitApply(()),
            Event::InitDone,
        ] {
            if is_object_change(&event) {
                changes += 1;
            }
        }

        assert_eq!(changes, 2, "expected one change per InitDone, first list and relist alike");
    }

    #[test]
    fn only_forbidden_is_a_permanent_watch_error() {
        // A genuine RBAC Forbidden denial is stable and won't self-heal — stop.
        assert!(is_permanent_watch_error(
            "watch deployments is forbidden: User cannot watch"
        ));
        assert!(is_permanent_watch_error(
            "deployments.apps is forbidden: ..."
        ));
        assert!(is_permanent_watch_error(
            "Forbidden (ErrorResponse { code: 403 })"
        ));

        // 401 / token expiry IS recoverable: kube-rs refreshes creds and re-lists,
        // so we keep reconnecting rather than permanently stopping the watch.
        assert!(!is_permanent_watch_error("Unauthorized"));
        assert!(!is_permanent_watch_error("server responded with 401"));

        // Transient failures must NOT be treated as permanent (they self-heal).
        assert!(!is_permanent_watch_error("list deployments timed out"));
        assert!(!is_permanent_watch_error(
            "error trying to connect: connection refused"
        ));
        // A benign message with a "14030" substring must not false-positive.
        assert!(!is_permanent_watch_error("read 14030 bytes"));
    }
}
