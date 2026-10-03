//! `PodSummary::status`: the STATUS column `kubectl get pods` prints.
//!
//! Every case lives in `fixtures/pod_status_cases.json`, written the way the
//! API server sends a pod, with the word kubectl prints for it. The desktop's
//! TypeScript port (`packages/core/src/lib/kubectlPodStatus.ts`) runs the same
//! file, so the two copies of kubectl's rules cannot drift apart unnoticed.
//! Add a case there, not a test here.

use serde::Deserialize;
use srelens_kube::workloads::summarise_pod;

#[derive(Deserialize)]
struct Case {
    name: String,
    pod: serde_json::Value,
    status: String,
}

#[test]
fn every_shared_case_reads_as_kubectl_prints_it() {
    let cases: Vec<Case> = serde_json::from_str(include_str!("fixtures/pod_status_cases.json"))
        .expect("the case file is valid JSON");
    assert!(
        cases.len() >= 24,
        "the case file lost cases: {}",
        cases.len()
    );
    let wrong: Vec<String> = cases
        .into_iter()
        .filter_map(|case| {
            let pod = serde_json::from_value(case.pod)
                .unwrap_or_else(|e| panic!("{}: not a valid Pod: {e}", case.name));
            let got = summarise_pod(pod).status;
            (got != case.status).then(|| {
                format!(
                    "{}: got {got:?}, kubectl prints {:?}",
                    case.name, case.status
                )
            })
        })
        .collect();
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}
