//! The `k8s.listEvents` capability — cluster events with type/reason/object.

use std::sync::Arc;

use srelens_capability::{Annotations, Capability, CapabilityError};
use k8s_openapi::api::core::v1::Event;
use kube::api::ListParams;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::client_cache::ClientCache;
use crate::connect::request_timeout;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListEventsIn {
    pub context: String,
    #[serde(default)]
    pub namespace: String,
    #[serde(default, rename = "objectKind")]
    pub object_kind: String,
    #[serde(default, rename = "objectName")]
    pub object_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct EventSummary {
    /// The Event's own object name — a stable unique key for the watch/table.
    pub name: String,
    /// Which namespace the event came from. Empty for a cluster-scoped event.
    ///
    /// Reported beside the composite `name` rather than left to be recovered
    /// from it: the key's `<namespace>/<name>` shape is a key's business, and
    /// reading a namespace back out of it turns "an event name has no slash in
    /// it" into a rule the UI depends on and nothing states.
    pub namespace: String,
    #[serde(rename = "type")]
    pub type_: String,
    pub reason: String,
    pub object: String,
    pub message: String,
    pub age: String,
    /// How many times this event has fired. Absent means once, not none.
    pub count: i32,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListEventsOut {
    pub events: Vec<EventSummary>,
}

pub(crate) fn summarise(ev: Event) -> EventSummary {
    let object = format!(
        "{}/{}",
        ev.involved_object.kind.clone().unwrap_or_default(),
        ev.involved_object.name.clone().unwrap_or_default()
    );
    let age = crate::humanize_age(ev.last_timestamp.as_ref());
    let namespace = ev.metadata.namespace.clone().unwrap_or_default();
    let own_name = ev.metadata.name.clone().unwrap_or_default();
    // One derivation, so the reported namespace and the key it is prefixed to
    // cannot disagree.
    let name = if namespace.is_empty() {
        own_name
    } else {
        format!("{namespace}/{own_name}")
    };
    EventSummary {
        name,
        namespace,
        type_: ev.type_.clone().unwrap_or_default(),
        reason: ev.reason.clone().unwrap_or_default(),
        object,
        message: ev.message.clone().unwrap_or_default(),
        age,
        count: ev.count.unwrap_or(1),
    }
}

fn event_list_params(object_kind: &str, object_name: &str) -> ListParams {
    if object_name.is_empty() {
        return ListParams::default();
    }
    let mut selectors = vec![format!("involvedObject.name={object_name}")];
    if !object_kind.is_empty() {
        selectors.push(format!("involvedObject.kind={object_kind}"));
    }
    ListParams::default().fields(&selectors.join(","))
}

/// `k8s.listEvents` — list events (optionally namespaced).
pub fn list_events_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ListEventsIn, ListEventsOut, _, _>(
        "k8s.listEvents",
        "list events in a connected kube context",
        Annotations::READ_ONLY,
        move |input: ListEventsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let api: kube::Api<Event> = crate::scoped_api(client, &input.namespace);
                let params = event_list_params(&input.object_kind, &input.object_name);
                let list = tokio::time::timeout(request_timeout(), api.list(&params))
                    .await
                    .map_err(|_| CapabilityError::Handler("list events timed out".into()))?
                    .map_err(|e| CapabilityError::Handler(e.to_string()))?;
                Ok(ListEventsOut {
                    events: list.items.into_iter().map(summarise).collect(),
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = list_events_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.listEvents");
    }

    #[test]
    fn summarises_object_ref() {
        let ev = Event {
            type_: Some("Warning".into()),
            reason: Some("BackOff".into()),
            message: Some("Back-off restarting".into()),
            involved_object: k8s_openapi::api::core::v1::ObjectReference {
                kind: Some("Pod".into()),
                name: Some("web-1".into()),
                ..Default::default()
            },
            ..Default::default()
        };
        let s = summarise(ev);
        assert_eq!(s.type_, "Warning");
        assert_eq!(s.object, "Pod/web-1");
    }

    #[test]
    fn summarise_carries_the_repeat_count() {
        let mut ev = Event::default();
        ev.metadata.name = Some("web.17a".into());
        ev.type_ = Some("Warning".into());
        ev.reason = Some("BackOff".into());
        ev.message = Some("Back-off restarting failed container".into());
        ev.count = Some(37);
        assert_eq!(summarise(ev).count, 37);
    }

    #[test]
    fn summarise_reads_an_absent_count_as_one() {
        let mut ev = Event::default();
        ev.metadata.name = Some("web.17b".into());
        assert_eq!(summarise(ev).count, 1);
    }

    #[test]
    fn summarise_reports_the_namespace_beside_the_composite_key() {
        let mut ev = Event::default();
        ev.metadata.namespace = Some("shop".into());
        ev.metadata.name = Some("web-0.17a".into());
        let s = summarise(ev);
        assert_eq!(s.namespace, "shop");
        // The key keeps its job. `namespace` is a second field, not a
        // replacement: the table still needs one value unique across namespaces.
        assert_eq!(s.name, "shop/web-0.17a");
    }

    #[test]
    fn summarise_leaves_a_cluster_scoped_event_without_a_namespace() {
        let mut ev = Event::default();
        ev.metadata.name = Some("node-a.17b".into());
        let s = summarise(ev);
        assert_eq!(s.namespace, "");
        assert_eq!(s.name, "node-a.17b");
    }

    #[test]
    fn summarise_treats_an_explicit_empty_namespace_the_same_as_absent() {
        // The API server never actually sends this — it omits
        // `metadata.namespace` for a cluster-scoped event rather than sending
        // `""` — but `unwrap_or_default()` collapses `None` and `Some("")`
        // one line before the branch that reads it, so both inputs must
        // produce the identical result. Pinned on its own input, rather than
        // assumed from the `None` case above, so the claim that this shape is
        // covered is actually true.
        let mut ev = Event::default();
        ev.metadata.namespace = Some("".into());
        ev.metadata.name = Some("node-a.17c".into());
        let s = summarise(ev);
        assert_eq!(s.namespace, "");
        assert_eq!(s.name, "node-a.17c");
    }

    #[test]
    fn filters_events_by_exact_involved_object() {
        let params = event_list_params("Pod", "web-1");
        assert_eq!(
            params.field_selector.as_deref(),
            Some("involvedObject.name=web-1,involvedObject.kind=Pod")
        );
        assert_eq!(event_list_params("", "").field_selector, None);
    }
}
