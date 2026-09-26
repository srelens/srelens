//! The `k8s.listEvents` capability — cluster events with type/reason/object.

use std::sync::Arc;

use k8s_openapi::api::core::v1::Event;
use kube::api::ListParams;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
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
    #[serde(rename = "objectApiVersion")]
    pub object_api_version: String,
    pub source: String,
    #[serde(rename = "firstAge")]
    pub first_age: String,
    /// Raw first occurrence, with creation time as the fallback.
    #[serde(rename = "firstCreated")]
    pub first_created: Option<String>,

    pub message: String,
    /// Last-occurrence timestamp (RFC 3339), for a LIVE last-seen age.
    /// `age` below is rendered once, when this summary is built, and only
    /// rebuilt when a watch event arrives — so it goes stale (#405).
    pub created: Option<String>,
    pub age: String,
    /// Raw ISO 8601 timestamp `age` derives from, so UIs can recompute the
    /// age live at render time. Empty when the resource carries none.
    #[serde(rename = "createdAt")]
    pub created_at: String,
    /// How many times this event has fired. Absent means once, not none.
    pub count: i32,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListEventsOut {
    pub events: Vec<EventSummary>,
    /// True when the list was cut at [`crate::list_cap::APP_LIST_CAP`] and more
    /// events remain on the API server (#609).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

pub(crate) fn event_last_timestamp(ev: &Event) -> Option<k8s_openapi::jiff::Timestamp> {
    ev.last_timestamp
        .as_ref()
        .map(|t| t.0)
        .or_else(|| {
            ev.series
                .as_ref()
                .and_then(|s| s.last_observed_time.as_ref().map(|t| t.0))
        })
        .or_else(|| ev.event_time.as_ref().map(|t| t.0))
        .or_else(|| ev.metadata.creation_timestamp.as_ref().map(|t| t.0))
}

pub(crate) fn event_first_timestamp(ev: &Event) -> Option<k8s_openapi::jiff::Timestamp> {
    ev.first_timestamp
        .as_ref()
        .map(|t| t.0)
        .or_else(|| ev.event_time.as_ref().map(|t| t.0))
        .or_else(|| ev.metadata.creation_timestamp.as_ref().map(|t| t.0))
}

pub(crate) fn summarise(ev: Event) -> EventSummary {
    let object = format!(
        "{}/{}",
        ev.involved_object.kind.clone().unwrap_or_default(),
        ev.involved_object.name.clone().unwrap_or_default()
    );
    let last_ts = event_last_timestamp(&ev);
    let created = last_ts.map(|t| t.to_string());
    let age = last_ts
        .map(|t| {
            crate::format_age(
                k8s_openapi::jiff::Timestamp::now()
                    .duration_since(t)
                    .as_secs(),
            )
        })
        .unwrap_or_else(|| "-".to_string());
    let created_at = last_ts.map(|t| t.to_string()).unwrap_or_default();

    let first_ts = event_first_timestamp(&ev);
    let first_age = first_ts
        .map(|t| {
            crate::format_age(
                k8s_openapi::jiff::Timestamp::now()
                    .duration_since(t)
                    .as_secs(),
            )
        })
        .unwrap_or_else(|| "-".to_string());
    let first_created = first_ts.map(|t| t.to_string());

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
        first_age,
        first_created,
        object_api_version: ev.involved_object.api_version.clone().unwrap_or_default(),
        source: ev
            .reporting_component
            .clone()
            .or_else(|| ev.source.as_ref().and_then(|s| s.component.clone()))
            .unwrap_or_default(),
        message: ev.message.clone().unwrap_or_default(),
        created,
        age,
        created_at,
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
                // No outer timeout: `list_capped` spends `request_timeout()` on
                // each page, which is what that budget measures. Wrapping the
                // walk gave four pages one request's time, and a busy
                // namespace — the one that needs paging — was the likeliest to
                // be cut off by it. The error mapping — an API error's own
                // words, a sentence of ours only for a timeout — is
                // `into_capability_error`'s.
                let (items, truncated) = crate::list_cap::list_capped(&api, params)
                    .await
                    .map_err(|e| e.into_capability_error("list events"))?;
                Ok(ListEventsOut {
                    events: items.into_iter().map(summarise).collect(),
                    truncated,
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
    fn first_occurrence_timestamp_uses_first_seen_then_creation() {
        let mut event: Event = serde_json::from_value(serde_json::json!({
            "metadata":{"creationTimestamp":"2026-09-13T11:59:00Z"},
            "involvedObject":{},
            "firstTimestamp":"2026-09-13T12:00:00Z",
            "lastTimestamp":"2026-09-13T12:00:10Z"
        }))
        .unwrap();
        let value = serde_json::to_value(summarise(event.clone())).unwrap();
        assert_eq!(value["firstCreated"], "2026-09-13T12:00:00Z");
        assert_eq!(value["created"], "2026-09-13T12:00:10Z");
        event.first_timestamp = None;
        event.event_time = Some(k8s_openapi::apimachinery::pkg::apis::meta::v1::MicroTime(
            "2026-09-13T11:59:30Z".parse().unwrap(),
        ));
        let value = serde_json::to_value(summarise(event.clone())).unwrap();
        assert_eq!(value["firstCreated"], "2026-09-13T11:59:30Z");
        event.event_time = None;
        let value = serde_json::to_value(summarise(event.clone())).unwrap();
        assert_eq!(value["firstCreated"], "2026-09-13T11:59:00Z");
        event.metadata.creation_timestamp = None;
        assert!(serde_json::to_value(summarise(event)).unwrap()["firstCreated"].is_null());
    }

    #[test]
    fn extension_event_metadata_preserves_api_identity_and_source() {
        let ev: Event = serde_json::from_value(serde_json::json!({
            "metadata":{"name":"ready","namespace":"flux-system"},
            "involvedObject":{"apiVersion":"source.toolkit.fluxcd.io/v1","kind":"GitRepository","name":"apps"},
            "source":{"component":"source-controller"}
        })).unwrap();
        let value = serde_json::to_value(summarise(ev)).unwrap();
        assert_eq!(value["objectApiVersion"], "source.toolkit.fluxcd.io/v1");
        assert_eq!(value["source"], "source-controller");
    }

    /// The whole-walk timeout this PR removed, re-created at the capability
    /// level: three pages that each answer inside the per-request budget but
    /// together outlast it. `k8s.listEvents` completes; a handler that wrapped
    /// the walk in one `request_timeout()` would have cut it off.
    #[tokio::test]
    async fn list_events_walks_pages_that_together_outlast_one_request_budget() {
        let _budget = crate::list_cap::test_support::hold_request_timeout(1);
        let per_page = std::time::Duration::from_millis(450);
        let event = |name: &str| serde_json::json!({"metadata":{"name":name,"namespace":"default"},"involvedObject":{}});
        let page = |items: Vec<serde_json::Value>, next: Option<&str>| {
            serde_json::json!({"apiVersion":"v1","kind":"EventList",
                "metadata":{"continue":next},"items":items})
        };
        let (client, uris) = crate::list_cap::test_support::mock_slow_pages(
            vec![
                page(vec![event("a")], Some("p2")),
                page(vec![event("b")], Some("p3")),
                page(vec![event("c")], None),
            ],
            per_page,
        );
        let cache = ClientCache::new(PathBuf::from("/x"));
        cache.preload("fake", client).await;
        let capability = list_events_capability(cache);

        let started = std::time::Instant::now();
        let out =
            (capability.handler)(serde_json::json!({"context": "fake", "namespace": "default"}))
                .await
                .expect("every page answered inside the per-request budget");

        assert!(
            started.elapsed() > crate::connect::request_timeout(),
            "the walk must have outlasted one request's budget to prove anything"
        );
        // A false `truncated` is omitted on the wire (`skip_serializing_if`).
        assert_ne!(out["truncated"], true, "{out}");
        assert_eq!(out["events"].as_array().map(Vec::len), Some(3), "{out}");
        assert_eq!(uris.lock().unwrap().len(), 3);
    }

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

    #[test]
    fn event_timestamp_fallback_chain_handles_all_variants() {
        use k8s_openapi::apimachinery::pkg::apis::meta::v1::{MicroTime, Time};
        use k8s_openapi::jiff::Timestamp;

        let t1 = Timestamp::from_second(1_700_000_000).unwrap();
        let t2 = Timestamp::from_second(1_700_000_100).unwrap();
        let t3 = Timestamp::from_second(1_700_000_200).unwrap();
        let t4 = Timestamp::from_second(1_700_000_300).unwrap();

        // 1. last_timestamp takes precedence
        let mut ev1 = Event::default();
        ev1.last_timestamp = Some(Time(t1));
        ev1.series = Some(k8s_openapi::api::core::v1::EventSeries {
            last_observed_time: Some(MicroTime(t2)),
            ..Default::default()
        });
        assert_eq!(event_last_timestamp(&ev1), Some(t1));

        // 2. series.last_observed_time used when last_timestamp is None
        let mut ev2 = Event::default();
        ev2.series = Some(k8s_openapi::api::core::v1::EventSeries {
            last_observed_time: Some(MicroTime(t2)),
            ..Default::default()
        });
        ev2.event_time = Some(MicroTime(t3));
        assert_eq!(event_last_timestamp(&ev2), Some(t2));

        // 3. event_time used when series is None
        let mut ev3 = Event::default();
        ev3.event_time = Some(MicroTime(t3));
        ev3.metadata.creation_timestamp = Some(Time(t4));
        assert_eq!(event_last_timestamp(&ev3), Some(t3));

        // 4. metadata.creation_timestamp is final fallback
        let mut ev4 = Event::default();
        ev4.metadata.creation_timestamp = Some(Time(t4));
        assert_eq!(event_last_timestamp(&ev4), Some(t4));
    }

    #[test]
    fn list_events_out_omits_truncated_when_false() {
        let raw = serde_json::to_value(ListEventsOut {
            events: vec![],
            truncated: false,
        })
        .unwrap();
        assert!(raw.get("truncated").is_none());
        let cut = serde_json::to_value(ListEventsOut {
            events: vec![],
            truncated: true,
        })
        .unwrap();
        assert_eq!(cut["truncated"], true);
    }
}
