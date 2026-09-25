//! Quick AI RCA for the `:changed` diagnostic card.
//!
//! One on-demand completion, with no tools, over what the card already knows
//! about a failing workload plus a longer log tail. The reply is two lines, a
//! root cause and an action item, rendered inline so the reader keeps the
//! workload list in view. `[a]` remains the full Assistant hand-off.
//!
//! Everything here except [`run`] is pure, so the prompt and the parser are
//! tested without a network.

use srelens_kube::changed::AppDeploymentChange;
use srelens_llm::{HttpProvider, Provider, ProviderConfig, StreamItem, Turn};

/// Log lines sent to the model. The card shows five; the model gets more.
pub const LOG_TAIL_LINES: i64 = 20;

/// Longest single line (log, event or symptom) placed in the prompt. A
/// stack-trace line or a JSON log record can run to kilobytes.
const MAX_LINE_CHARS: usize = 300;

/// Pod symptoms and warning events placed in the prompt, each.
const MAX_ITEMS: usize = 5;

/// What the prompt can say about the failing container's logs. The three
/// cases are different facts and must not read as one: a pod that never ran
/// has no logs, while a fetch that failed says nothing about the pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogEvidence {
    Lines(Vec<String>),
    /// The failing pods never started a process (Pending, image pull,
    /// config error), or the pod answered with no log.
    NeverRan,
    /// The log fetch itself failed, for this reason.
    Unavailable(String),
}

/// A parsed reply: one root-cause sentence and one action item. The action
/// item is empty when the model ignored the format and the whole reply was
/// kept as the root cause instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickRcaReply {
    pub root_cause: String,
    pub action_item: String,
}

fn clip(s: &str) -> String {
    let one_line = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.chars().count() <= MAX_LINE_CHARS {
        one_line
    } else {
        let mut out: String = one_line.chars().take(MAX_LINE_CHARS).collect();
        out.push('…');
        out
    }
}

/// The prompt for one workload and what is known about its logs. Without
/// logs the pod's own state is the evidence, and the prompt says why there
/// are none, so the model neither invents an application failure nor reads
/// a failed fetch as a pod that never ran.
pub fn build_prompt(d: &AppDeploymentChange, logs: &LogEvidence) -> String {
    let mut p = String::new();
    p.push_str(
        "You are an SRE incident responder triaging a page. Using only the evidence below, \
         give the most likely root cause in one sentence and the single most useful next \
         action in one sentence. Do not call tools. Do not speculate beyond the evidence; \
         if it is insufficient, say what to check.\n\n",
    );
    p.push_str(&format!(
        "Workload: {} {}/{}\n",
        d.kind, d.namespace, d.app_name
    ));
    p.push_str(&format!("Status: {}\n", d.incident_status.label()));
    p.push_str(&format!(
        "Detected cause: {} {}\n",
        d.failure_category.badge(),
        clip(&d.failure_detail)
    ));
    p.push_str(&format!(
        "Replicas: {}/{} ready\n",
        d.ready_replicas, d.desired_replicas
    ));
    p.push_str(&format!(
        "Revision: {} (previous: {})\n",
        d.current_revision,
        d.previous_revision.as_deref().unwrap_or("none")
    ));
    p.push_str(&format!("Image change: {}\n", clip(&d.image_diff)));
    if let Some(g) = &d.gitops {
        p.push_str(&format!(
            "GitOps: Argo app {} is {} / {} at {} ({})\n",
            g.app_name, g.sync_status, g.health_status, g.sync_revision, g.target_revision
        ));
        if let Some(msg) = &g.sync_message {
            p.push_str(&format!("GitOps message: {}\n", clip(msg)));
        }
    }

    if !d.pod_symptoms.is_empty() {
        // Grouped, so 147 identical pods read as one symptom with a count.
        let groups = crate::views::changed_view::group_pod_symptoms(&d.pod_symptoms);
        p.push_str(&format!("Failing pods ({}):\n", d.pod_symptoms.len()));
        for g in groups.iter().take(MAX_ITEMS) {
            p.push_str(&format!(
                "- <pod_symptom>{}</pod_symptom>\n",
                clip(&g.describe())
            ));
        }
        if groups.len() > MAX_ITEMS {
            p.push_str(&format!(
                "- and {} other distinct symptoms\n",
                groups.len() - MAX_ITEMS
            ));
        }
    }

    let warnings: Vec<_> = d
        .top_events
        .iter()
        .filter(|e| e.type_ == "Warning")
        .take(MAX_ITEMS)
        .collect();
    if !warnings.is_empty() {
        p.push_str("Warning events:\n");
        for e in warnings {
            p.push_str(&format!(
                "- [{}] <event_message>{}</event_message> (x{}, {} ago)\n",
                e.reason,
                clip(&e.message),
                e.count,
                e.age
            ));
        }
    }

    match logs {
        LogEvidence::Lines(lines) if !lines.is_empty() => {
            let source = match (&d.error_log_pod, &d.error_log_container) {
                (Some(pod), Some(c)) => format!(" ({pod}/{c})"),
                (Some(pod), None) => format!(" ({pod})"),
                _ => String::new(),
            };
            p.push_str(&format!("Last {} log lines{source}:\n", lines.len()));
            for l in lines {
                p.push_str(&format!("| <log_line>{}</log_line>\n", clip(l)));
            }
        }
        LogEvidence::Unavailable(reason) => p.push_str(&format!(
            "Logs: could not be fetched ({}). The pod state and events above are the only \
             evidence; do not assume the process never ran.\n",
            clip(reason)
        )),
        _ => p.push_str(
            "Logs: none. The failing pods never started a process, so the pod state and \
             events above are the only evidence.\n",
        ),
    }

    p.push_str("\nReply with exactly two lines and nothing else:\nRoot Cause: <one sentence>\nAction Item: <one sentence>\n");
    p
}

/// Strip the decoration models add around a label line: list bullets,
/// heading marks, bold and italic markers.
fn undecorate(line: &str) -> String {
    let t = line
        .trim()
        .trim_start_matches(|c: char| matches!(c, '-' | '*' | '•' | '#' | '>' | ' '));
    t.replace("**", "").replace("__", "").trim().to_string()
}

/// The text after `label:` when `line` starts with it, case-insensitive.
/// The colon is required, so prose that merely begins with the word
/// ("Fixing this requires...") is not taken for a label.
fn after_label(line: &str, labels: &[&str]) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    for label in labels {
        if lower.starts_with(label) {
            // ASCII lowercasing keeps byte offsets, so `label.len()` is a
            // valid boundary in `line` too.
            let rest = line[label.len()..].trim_start();
            if let Some(v) = rest.strip_prefix(':') {
                return Some(v.trim().to_string());
            }
        }
    }
    None
}

/// Parse a reply. Models often wrap the labels in markdown or add a
/// preamble; both are tolerated. A reply with no labels at all is kept whole
/// as the root cause rather than dropped, because an unformatted answer is
/// still an answer. Only an empty reply is an error.
pub fn parse_reply(text: &str) -> Result<QuickRcaReply, String> {
    let mut root_cause = None;
    let mut action_item = None;
    for raw in text.lines() {
        let line = undecorate(raw);
        if line.is_empty() {
            continue;
        }
        if root_cause.is_none() {
            if let Some(v) = after_label(&line, &["root cause"]) {
                root_cause = Some(v);
                continue;
            }
        }
        if action_item.is_none() {
            if let Some(v) = after_label(&line, &["action item", "action", "next step", "fix"]) {
                action_item = Some(v);
                continue;
            }
        }
    }

    match (root_cause, action_item) {
        (Some(rc), ai) if !rc.is_empty() => Ok(QuickRcaReply {
            root_cause: rc,
            action_item: ai.unwrap_or_default(),
        }),
        (_, Some(ai)) if !ai.is_empty() => Ok(QuickRcaReply {
            root_cause: "Not stated by the model.".to_string(),
            action_item: ai,
        }),
        // The labels came back with nothing after them.
        (Some(_), _) | (None, Some(_)) => Err("The model returned empty answers".to_string()),
        (None, None) => {
            let whole = clip(text);
            if whole.is_empty() {
                Err("The model returned an empty reply".to_string())
            } else {
                Ok(QuickRcaReply {
                    root_cause: whole,
                    action_item: String::new(),
                })
            }
        }
    }
}

/// Run one completion and return the model's text. `timeout_seconds` bounds
/// the whole call; a provider-reported error ends it with that error.
pub async fn run(
    config: ProviderConfig,
    prompt: String,
    timeout_seconds: u32,
) -> Result<String, String> {
    let provider = HttpProvider::new(config);
    let turns = [Turn::User(prompt)];
    let mut text = String::new();
    let mut stream_error: Option<String> = None;
    let call = async {
        let mut on_item = |item: StreamItem| match item {
            StreamItem::Text(t) => text.push_str(&t),
            StreamItem::Error(e) => stream_error = Some(e),
            _ => {}
        };
        provider.stream_turn(&turns, &[], &mut on_item).await
    };
    let limit = std::time::Duration::from_secs(u64::from(timeout_seconds.max(5)));
    // Bound first, so the future (and its borrow of `text`) is gone before
    // the arms read it.
    let outcome = tokio::time::timeout(limit, call).await;
    match outcome {
        Err(_) => Err(format!("no answer within {}s", timeout_seconds.max(5))),
        Ok(Err(e)) => Err(e.to_string()),
        Ok(Ok(())) => match stream_error {
            Some(e) => Err(e),
            None => Ok(text),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_kube::changed::{
        FailureCategory, GitOpsReleaseInfo, IncidentStatus, PodIncidentDetail, RolloutStatus,
    };
    use srelens_kube::events::EventSummary;

    fn event(type_: &str, reason: &str, message: &str) -> EventSummary {
        EventSummary {
            name: format!("ev.{reason}"),
            namespace: "prod".to_string(),
            type_: type_.to_string(),
            reason: reason.to_string(),
            object: "Pod/payment-api-1".to_string(),
            first_age: "10m".to_string(),
            first_created: None,
            object_api_version: "v1".to_string(),
            source: "kubelet".to_string(),
            message: message.to_string(),
            created: None,
            age: "2m".to_string(),
            created_at: String::new(),
            count: 4,
        }
    }

    fn workload() -> AppDeploymentChange {
        AppDeploymentChange {
            app_name: "payment-api".to_string(),
            kind: "Deployment".to_string(),
            namespace: "prod".to_string(),
            incident_status: IncidentStatus::CrashLoop,
            failure_category: FailureCategory::App,
            failure_detail: "payment terminated with Exit Code 1".to_string(),
            gitops: Some(GitOpsReleaseInfo {
                app_name: "payment-prod".to_string(),
                sync_status: "Synced".to_string(),
                health_status: "Degraded".to_string(),
                repo_url: "https://github.com/org/payment.git".to_string(),
                target_revision: "main".to_string(),
                sync_revision: "7b89abc".to_string(),
                sync_age: "12m".to_string(),
                sync_message: None,
            }),
            argo_rollout_in_window: None,
            error_log_snippet: Some(vec!["FATAL: bad DB_HOST".to_string()]),
            error_log_pod: Some("payment-api-1".to_string()),
            error_log_container: Some("payment".to_string()),
            change_kind: srelens_kube::changed::ChangeKind::Rollout,
            changed_age: String::new(),
            change_detail: None,
            deployed_at: None,
            deployed_age: "12m".to_string(),
            current_revision: "5".to_string(),
            previous_revision: Some("4".to_string()),
            current_images: vec![],
            previous_images: vec![],
            image_diff: "payment:v1 ➔ payment:v2".to_string(),
            desired_replicas: 2,
            updated_replicas: 2,
            ready_replicas: 0,
            available_replicas: 0,
            rollout_status: RolloutStatus::Failed,
            failing_pods_count: 1,
            crash_loop_count: 1,
            oom_killed_count: 0,
            probe_failure_count: 0,
            restart_count: 9,
            primary_symptoms: vec![],
            pod_symptoms: vec![PodIncidentDetail {
                pod_name: "payment-api-1".to_string(),
                status: "CrashLoopBackOff".to_string(),
                detail_message: "exited with code 1".to_string(),
            }],
            failing_pod_names: vec!["payment-api-1".to_string()],
            top_events: vec![
                event("Warning", "BackOff", "Back-off restarting failed container"),
                event("Normal", "Pulled", "Container image already present"),
            ],
        }
    }

    #[test]
    fn prompt_carries_the_card_evidence_and_the_log_tail() {
        let logs = vec![
            "connecting to bad-db-host.internal".to_string(),
            "FATAL: bad DB_HOST".to_string(),
        ];
        let p = build_prompt(&workload(), &LogEvidence::Lines(logs));

        assert!(p.contains("Workload: Deployment prod/payment-api"));
        assert!(p.contains("Status: CrashLoop"));
        assert!(p.contains("Detected cause: [APP] payment terminated with Exit Code 1"));
        assert!(p.contains("Replicas: 0/2 ready"));
        assert!(p.contains("Revision: 5 (previous: 4)"));
        assert!(p.contains("payment:v1 ➔ payment:v2"));
        assert!(p.contains("Argo app payment-prod is Synced / Degraded at 7b89abc (main)"));
        assert!(p.contains("Failing pods (1):"));
        assert!(p.contains(
            "- <pod_symptom>payment-api-1: CrashLoopBackOff | exited with code 1</pod_symptom>"
        ));
        assert!(p.contains("[BackOff] <event_message>Back-off restarting failed container</event_message> (x4, 2m ago)"));
        assert!(!p.contains("Pulled"), "Normal events are not evidence");
        assert!(p.contains("Last 2 log lines (payment-api-1/payment):"));
        assert!(p.contains("| <log_line>FATAL: bad DB_HOST</log_line>"));
        assert!(p.contains("Root Cause: <one sentence>"));
        assert!(p.contains("Action Item: <one sentence>"));
    }

    #[test]
    fn prompt_without_logs_says_the_pod_never_ran() {
        let mut d = workload();
        d.incident_status = IncidentStatus::Pending;
        let p = build_prompt(&d, &LogEvidence::NeverRan);
        assert!(p.contains("Logs: none. The failing pods never started a process"));
        assert!(!p.contains("log lines"));

        // An empty tail is the same as none, not "Last 0 log lines".
        let p = build_prompt(&d, &LogEvidence::Lines(vec![]));
        assert!(p.contains("Logs: none."));
    }

    #[test]
    fn prompt_for_a_failed_fetch_does_not_claim_the_pod_never_ran() {
        let p = build_prompt(
            &workload(),
            &LogEvidence::Unavailable("connection refused".to_string()),
        );
        assert!(p.contains("Logs: could not be fetched (connection refused)"));
        assert!(!p.contains("never started a process"));
    }

    #[test]
    fn prompt_groups_many_identical_pods_into_one_symptom() {
        let mut d = workload();
        d.pod_symptoms = (0..150)
            .map(|i| PodIncidentDetail {
                pod_name: format!("payment-api-{i}"),
                status: "CrashLoopBackOff".to_string(),
                detail_message: "exited with code 1".to_string(),
            })
            .collect();
        let p = build_prompt(&d, &LogEvidence::NeverRan);
        assert!(p.contains("Failing pods (150):"));
        assert!(p.contains(
            "- <pod_symptom>CrashLoopBackOff | exited with code 1 — 150 pods (payment-api-0, payment-api-1, +148 more)</pod_symptom>"
        ));
        assert!(!p.contains("payment-api-2,"), "not every pod is listed");
    }

    #[test]
    fn prompt_clips_long_and_multi_line_values() {
        let long = format!("{}\nsecond line", "x".repeat(2_000));
        let p = build_prompt(&workload(), &LogEvidence::Lines(vec![long]));
        let log_line = p.lines().find(|l| l.starts_with("| <log_line>x")).unwrap();
        assert!(
            log_line.chars().count() <= MAX_LINE_CHARS + 3 + "<log_line></log_line>".len(),
            "clipped to one bounded line"
        );
        assert!(log_line.ends_with("…</log_line>"));
    }

    #[test]
    fn parses_the_requested_format() {
        let r = parse_reply(
            "Root Cause: The app cannot resolve bad-db-host.internal.\nAction Item: Fix DB_HOST in the payment ConfigMap.",
        )
        .unwrap();
        assert_eq!(r.root_cause, "The app cannot resolve bad-db-host.internal.");
        assert_eq!(r.action_item, "Fix DB_HOST in the payment ConfigMap.");
    }

    #[test]
    fn parses_markdown_decorated_labels_after_a_preamble() {
        let r = parse_reply(
            "Here is my analysis:\n\n- **Root Cause:** Redis refused connections.\n* **Action item**: Check the redis-master Service.\n",
        )
        .unwrap();
        assert_eq!(r.root_cause, "Redis refused connections.");
        assert_eq!(r.action_item, "Check the redis-master Service.");
    }

    #[test]
    fn prose_starting_with_a_label_word_is_not_a_label() {
        let r = parse_reply("Root cause: OOM at startup.\nFixing this needs more memory.").unwrap();
        assert_eq!(r.root_cause, "OOM at startup.");
        assert_eq!(
            r.action_item, "",
            "'Fixing ...' has no colon and is not an action label"
        );
    }

    #[test]
    fn an_unformatted_reply_is_kept_whole_as_the_root_cause() {
        let r = parse_reply(
            "The container exits because DB_HOST\npoints at a host that does not exist.",
        )
        .unwrap();
        assert_eq!(
            r.root_cause,
            "The container exits because DB_HOST points at a host that does not exist."
        );
        assert_eq!(r.action_item, "");
    }

    #[test]
    fn an_empty_reply_is_an_error() {
        assert!(parse_reply("").is_err());
        assert!(parse_reply("  \n\n ").is_err());
        // Labels with nothing after them are not an answer either.
        assert!(parse_reply("Root Cause:\nAction Item:").is_err());
    }

    #[test]
    fn an_action_without_a_root_cause_is_still_shown() {
        let r = parse_reply("Action Item: Roll back to revision 4.").unwrap();
        assert_eq!(r.root_cause, "Not stated by the model.");
        assert_eq!(r.action_item, "Roll back to revision 4.");
    }
}
