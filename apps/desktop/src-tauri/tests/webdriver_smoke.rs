//! WebDriver smoke suite (issue #30): the REAL app — built binary, real
//! WebView, real Rust backend — driven end-to-end against a live kind
//! cluster. This is the only layer that exercises the WebView↔Rust boundary
//! itself: a regression in the transport shim or command registration fails
//! here even when every unit and integration suite stays green, because
//! those layers call the registry directly and never cross the bridge.
//!
//! Ignored by default — needs a built app, `tauri-driver`, WebKitWebDriver,
//! a kind cluster, and (headless) a display server. Locally:
//!
//! ```sh
//! kind create cluster --name srelens-e2e
//! pnpm --filter @srelens/desktop build
//! # custom-protocol is what makes the binary serve the EMBEDDED dist — a
//! # plain debug build expects the Vite dev server and renders only
//! # "Could not connect to localhost" (learned from the first CI run's
//! # failure screenshot).
//! cargo build -p srelens-desktop --features custom-protocol
//! cargo install tauri-driver --locked
//! # Linux: sudo apt-get install webkit2gtk-driver xvfb
//! xvfb-run --auto-servernum \
//!   cargo test -p srelens-desktop --features custom-protocol --test webdriver_smoke -- --ignored --nocapture
//! ```
//!
//! Environment:
//! - `SRELENS_E2E_CONTEXT`   — kube context (default `kind-srelens-e2e`)
//! - `SRELENS_SMOKE_APP`     — app binary (default `<workspace>/target/debug/srelens`)
//! - `SRELENS_SMOKE_FULL=1`  — adds the exec + port-forward tier (release runs)
//! - `SRELENS_SMOKE_ARTIFACTS` — where failure artifacts (screenshot + app
//!   logs) are written (default `/tmp/srelens-smoke-artifacts`)
//!
//! The app runs under a throwaway `HOME` so the vault starts at first-launch
//! setup and no real user state is touched; `KUBECONFIG` is pinned to the
//! real one so the kind context stays visible from inside that sandbox.
//!
//! It drives the new design — the default every fresh profile opens on
//! (`packages/ui-next`) — and finds what it clicks by role and accessible
//! name, the way a reader using assistive technology would, rather than by
//! the classes a restyle renames.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use thirtyfour::prelude::*;

const MASTER_PASSWORD: &str = "smoke-master-password";
const POD: &str = "srelens-smoke-pod";
/// Printed by the fixture pod on start; proves the logs path end-to-end.
const LOG_MARKER: &str = "hello-smoke";

/// Deletes the fixture pod even on panic — setup `.expect`s after the pod
/// exists (session creation, most notably) must not leak it. The
/// delete-before-create at suite start is the second line of defense for a
/// hard-killed process, where no Drop runs at all.
struct FixtureGuard {
    context: String,
}
impl Drop for FixtureGuard {
    fn drop(&mut self) {
        let _ = kubectl(&["--context", &self.context, "delete", "pod", POD, "--ignore-not-found"]);
    }
}

/// Removes the sandbox HOME even on panic — and when the thread IS
/// panicking (a setup failure, before the in-flow error path could collect
/// anything), first preserves the app's own logs into the artifacts dir, so
/// the diagnostic that explains the failure survives it.
struct HomeGuard {
    home: std::path::PathBuf,
    artifacts: std::path::PathBuf,
}
impl Drop for HomeGuard {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ = Command::new("cp")
                .args([
                    "-r",
                    &self.home.join(".local/share").to_string_lossy(),
                    &self.artifacts.join("app-data").to_string_lossy(),
                ])
                .status();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// Kills tauri-driver (which kills the app it spawned) even on panic.
struct DriverProc(Child);
impl Drop for DriverProc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn kubectl(args: &[&str]) -> Result<String, String> {
    let out = Command::new("kubectl")
        .args(args)
        .output()
        .map_err(|e| format!("kubectl spawn failed: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "kubectl {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

fn workspace_root() -> PathBuf {
    // apps/desktop/src-tauri → three levels up.
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..").canonicalize().unwrap()
}

/// Wait until an element matching `by` exists.
async fn wait_for(driver: &WebDriver, by: By, secs: u64) -> WebDriverResult<WebElement> {
    driver
        .query(by)
        .wait(Duration::from_secs(secs), Duration::from_millis(500))
        .first()
        .await
}

/// Click the first element matching `by`, retrying until `secs` — on
/// absence AND on click rejection. The retry-on-intercepted half matters as
/// much as the wait: a target can exist and still be legitimately covered
/// for a moment — by a dialog on its way out, or a popover's panel — and one
/// attempt would fail instantly with ElementClickIntercepted; retrying makes
/// every click "when actually clickable, within the deadline".
async fn click(driver: &WebDriver, by: By, secs: u64) -> WebDriverResult<()> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        let attempt = async {
            let el = driver.query(by.clone()).nowait().first().await?;
            el.scroll_into_view().await.ok();
            el.click().await
        }
        .await;
        match attempt {
            Ok(()) => return Ok(()),
            Err(e) if Instant::now() >= deadline => return Err(e),
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await,
        }
    }
}

/// Click the element at `target` (an XPath) until `proof` appears, within
/// `secs`. A WebDriver click can be accepted and still change nothing — on a
/// screen still settling, where the target moves between being located and
/// being clicked, and in CI, where the cluster row's clicks left the window
/// on Home with no error to retry on. So the click's effect is what is waited
/// on, and each miss logs what the click was aimed at.
async fn click_until(driver: &WebDriver, target: &str, proof: By, secs: u64) -> WebDriverResult<()> {
    let deadline = Instant::now() + Duration::from_secs(secs);
    let xpath = vec![serde_json::Value::String(target.to_string())];
    loop {
        // Listeners on the target, so a miss can say which of the click's
        // events reached it, and a record of page errors.
        driver.execute(PROBE_INSTALL, xpath.clone()).await.ok();
        let remaining = deadline.saturating_duration_since(Instant::now()).as_secs().max(1);
        click(driver, By::XPath(target.to_string()), remaining).await?;
        let landed = driver
            .query(proof.clone())
            .wait(Duration::from_secs(5), Duration::from_millis(250))
            .exists()
            .await?;
        if landed {
            return Ok(());
        }
        // A miss with no error is otherwise invisible: where the target is,
        // the viewport, what the aimed-at point holds, which events arrived,
        // and whether React has a click handler on it at all.
        let aimed = describe(driver, PROBE_DESCRIBE, xpath.clone()).await;
        println!("click on {target} changed nothing; aimed at {aimed}");
        if Instant::now() >= deadline {
            // One script click, as evidence only: does the handler work when
            // it is called directly? The step still fails either way.
            let scripted = describe(driver, "arguments[0] && document.evaluate(arguments[0], document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue.click(); return 'clicked';", xpath.clone()).await;
            let opened = driver
                .query(proof.clone())
                .wait(Duration::from_secs(3), Duration::from_millis(250))
                .exists()
                .await
                .unwrap_or(false);
            // An error whatever the script click did: a smoke that passed on
            // it would pass with real clicks broken.
            return Err(WebDriverError::Timeout(format!(
                "clicking {target} never showed its effect; a script click ({scripted}) {} show it",
                if opened { "DID" } else { "did not" }
            )));
        }
    }
}

async fn describe(driver: &WebDriver, script: &str, args: Vec<serde_json::Value>) -> String {
    driver
        .execute(script, args)
        .await
        .map(|ret| ret.json().to_string())
        .unwrap_or_else(|e| format!("(script failed: {e})"))
}

const PROBE_INSTALL: &str = r#"
    const el = document.evaluate(arguments[0], document, null, XPathResult.FIRST_ORDERED_NODE_TYPE, null).singleNodeValue;
    if (!window.__smokeErrors) {
        window.__smokeErrors = [];
        addEventListener('error', (e) => __smokeErrors.push(String(e.message)));
        addEventListener('unhandledrejection', (e) => __smokeErrors.push(String(e.reason)));
    }
    window.__smokeEvents = [];
    if (el && !el.__smokeProbe) {
        el.__smokeProbe = true;
        for (const type of ['pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click']) {
            el.addEventListener(type, () => window.__smokeEvents.push(type), true);
        }
    }
"#;

const PROBE_DESCRIBE: &str = r#"
    const found = document.evaluate(arguments[0], document, null, XPathResult.ORDERED_NODE_SNAPSHOT_TYPE, null);
    const el = found.snapshotLength ? found.snapshotItem(0) : null;
    if (!el) return JSON.stringify({ matches: 0, errors: window.__smokeErrors });
    const r = el.getBoundingClientRect();
    const top = document.elementFromPoint(r.x + r.width / 2, r.y + r.height / 2);
    const propsKey = Object.keys(el).find((k) => k.startsWith('__reactProps'));
    return JSON.stringify({
        matches: found.snapshotLength,
        rect: [r.x, r.y, r.width, r.height],
        viewport: [innerWidth, innerHeight, devicePixelRatio, screen.width, screen.height],
        topIsTarget: !!top && el.contains(top),
        events: window.__smokeEvents,
        errors: window.__smokeErrors,
        reactOnClick: propsKey ? typeof el[propsKey].onClick : 'no react props',
        focused: document.hasFocus(),
    });
"#;

#[tokio::test]
#[ignore]
async fn smoke_launch_to_logs_against_kind() {
    let context =
        std::env::var("SRELENS_E2E_CONTEXT").unwrap_or_else(|_| "kind-srelens-e2e".to_string());
    let full = std::env::var("SRELENS_SMOKE_FULL").is_ok_and(|v| v == "1");
    let artifacts = PathBuf::from(
        std::env::var("SRELENS_SMOKE_ARTIFACTS")
            .unwrap_or_else(|_| "/tmp/srelens-smoke-artifacts".to_string()),
    );
    std::fs::create_dir_all(&artifacts).ok();

    // ---- Cluster fixture: a pod that logs a marker and serves one-line HTTP
    // on 8080 (busybox nc), so logs, exec, and the port-forward check all
    // have something real to hit. `--restart=Never` keeps it a bare pod.
    let _ = kubectl(&["--context", &context, "delete", "pod", POD, "--ignore-not-found"]);
    // `hello-$(echo smoke)`: the POD's shell expands this to the marker, so
    // the LOGGED text is `hello-smoke` while the pod SPEC — which the detail
    // overview renders in the DOM — only ever contains the unexpanded form.
    // The logs assertion therefore cannot be satisfied by the spec text.
    kubectl(&[
        "--context", &context, "run", POD, "--image=busybox:1.36", "--restart=Never",
        "--command", "--", "sh", "-c",
        "echo hello-$(echo smoke); while true; do { echo -e 'HTTP/1.1 200 OK\\r\\nContent-Length: 2\\r\\n\\r\\nok'; } | nc -l -p 8080; done",
    ])
    .expect("fixture pod creation");
    // The guard exists from the moment the pod does — a readiness failure
    // (image pull, scheduling) panics on the very next line, and that panic
    // must delete the pod like any later one.
    let _fixture_guard = FixtureGuard { context: context.clone() };
    kubectl(&["--context", &context, "wait", "--for=condition=Ready", &format!("pod/{POD}"), "--timeout=180s"])
        .expect("fixture pod became Ready");

    // ---- Throwaway HOME: fresh vault (first-launch setup), fresh settings,
    // no real user state touched. KUBECONFIG must be pinned BEFORE HOME moves
    // or the kind context disappears from inside the sandbox.
    let real_kubeconfig = std::env::var("KUBECONFIG").unwrap_or_else(|_| {
        format!("{}/.kube/config", std::env::var("HOME").expect("HOME set"))
    });
    let home = std::env::temp_dir().join(format!("srelens-smoke-home-{}", std::process::id()));
    std::fs::create_dir_all(&home).expect("smoke HOME");
    // On a PANIC (setup failures like session creation — distinct from the
    // in-flow error path below), the guard still preserves the app's logs
    // into the artifacts dir before removing the sandbox.
    let _home_guard = HomeGuard { home: home.clone(), artifacts: artifacts.clone() };

    let app = std::env::var("SRELENS_SMOKE_APP")
        .map(PathBuf::from)
        .unwrap_or_else(|_| workspace_root().join("target/debug/srelens"));
    assert!(
        app.exists(),
        "app binary missing at {app:?} — build with \
         `cargo build -p srelens-desktop --features custom-protocol`"
    );

    // ---- tauri-driver proxies WebDriver to WebKitWebDriver and spawns the
    // app; env set here is inherited by the app process.
    // stderr goes to the artifacts dir, not /dev/null: when session
    // creation fails, tauri-driver's own complaint is the only diagnosis.
    let driver_log = std::fs::File::create(artifacts.join("tauri-driver.log"))
        .expect("driver log file");
    let driver_proc = Command::new("tauri-driver")
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("KUBECONFIG", &real_kubeconfig)
        .stdout(Stdio::null())
        .stderr(driver_log)
        .spawn()
        .expect("tauri-driver on PATH — `cargo install tauri-driver`");
    let _driver_guard = DriverProc(driver_proc);

    // Port 4444 is tauri-driver's default; wait for it to listen.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if std::net::TcpStream::connect(("127.0.0.1", 4444)).is_ok() {
            break;
        }
        assert!(Instant::now() < deadline, "tauri-driver never started listening");
        std::thread::sleep(Duration::from_millis(300));
    }

    let mut caps = Capabilities::new();
    caps.insert(
        "tauri:options".to_string(),
        serde_json::json!({ "application": app.to_string_lossy() }),
    );
    // Session creation retries: the port accepting a TCP connect does not
    // mean tauri-driver is ready to serve /session yet (observed locally as
    // an immediate HttpError) — give it a bounded warmup.
    let mut driver = None;
    let deadline = Instant::now() + Duration::from_secs(30);
    while driver.is_none() {
        match WebDriver::new("http://127.0.0.1:4444", caps.clone()).await {
            Ok(d) => driver = Some(d),
            Err(e) if Instant::now() >= deadline => {
                panic!("WebDriver session never came up: {e} (see tauri-driver.log in artifacts)")
            }
            Err(_) => tokio::time::sleep(Duration::from_secs(1)).await,
        }
    }
    let driver = driver.expect("set above");

    let flow = run_flow(&driver, &context, full).await;

    if flow.is_err() {
        // ---- Failure artifacts: screenshot + the app's own logs, from the
        // sandbox HOME the app actually wrote into.
        let _ = driver.screenshot(&artifacts.join("failure.png")).await;
        let log_dir = home.join(".local/share");
        let _ = Command::new("cp").args(["-r", &log_dir.to_string_lossy(), &artifacts.join("app-data").to_string_lossy()]).status();
    }
    let _ = driver.quit().await;

    flow.expect("smoke flow");
}

type FlowError = Box<dyn std::error::Error + Send + Sync>;

/// Errors, never panics: the caller's failure-artifact branch (screenshot +
/// app logs) and the fixture/HOME cleanup only run on a RETURNED error — a
/// panic would unwind straight past the diagnostics meant to explain it.
async fn run_flow(driver: &WebDriver, context: &str, full: bool) -> Result<(), FlowError> {
    // ---- 1. First-launch vault gate: `LockGate`'s setup form, which covers
    // the whole window until a master passphrase exists. The recovery
    // checkbox is UNCHECKED first — CI has no OS keychain, and the setup must
    // not depend on one.
    wait_for(driver, By::Css("input[aria-label='Master passphrase']"), 60).await?;
    // The gate is driven by SCRIPT, not synthesized input: WebDriver-level
    // interaction with classic's version of this form proved
    // environment-flaky in two different ways (CI intercepted the button
    // click; the local container intercepted the label-nested checkbox and
    // dropped keys mid-render). The gate is prerequisite plumbing — the
    // WebView↔Rust bridge under test is exercised by everything AFTER it,
    // which stays real interaction. React controlled inputs need the native
    // value setter + an input event.
    driver
        .execute(
            r#"
            const set = Object.getOwnPropertyDescriptor(
                window.HTMLInputElement.prototype, 'value').set;
            for (const name of ['Master passphrase', 'Repeat the passphrase']) {
                const el = document.querySelector(`input[aria-label='${name}']`);
                set.call(el, arguments[0]);
                el.dispatchEvent(new Event('input', { bubbles: true }));
            }
            for (const label of document.querySelectorAll('label')) {
                const cb = label.querySelector("input[type='checkbox']");
                if (cb && cb.checked && label.textContent.includes('recovery copy')) cb.click();
            }
            "#,
            vec![serde_json::json!(MASTER_PASSWORD)],
        )
        .await?;
    // The form has no `<form>` to submit: its button stays disabled until
    // React has taken both entries, so the click waits for it to be enabled
    // — a click on a disabled button "succeeds" and does nothing.
    let create = wait_for(
        driver,
        By::XPath("//button[normalize-space()='Create passphrase' and not(@disabled)]".to_string()),
        15,
    )
    .await?;
    driver.execute("arguments[0].click();", vec![create.to_json()?]).await?;

    // ---- 2. The cover lifts onto the shell: the cluster rail and the tab
    // strip. Sixty seconds, because the cover stays up for a debug build's
    // argon2id derivation.
    wait_for(driver, By::XPath("//nav[@aria-label='Clusters']".to_string()), 60).await?;
    wait_for(
        driver,
        By::XPath("//*[@role='tablist' and @aria-label='Open tabs']".to_string()),
        10,
    )
    .await?;

    // ---- 3. A fresh profile opens on Home; open the kind context from its
    // cluster list. The row's name carries the link state after the em dash
    // (`Open cluster <name> — Connected`), which changes as the probe lands.
    // Opening it always opens the cluster's overview tab, which is the proof.
    click_until(
        driver,
        &format!("//button[starts-with(@aria-label, 'Open cluster {context} \u{2014}')]"),
        By::XPath(format!(
            "//*[@role='tablist' and @aria-label='Open tabs']//*[@role='tab' and starts-with(@aria-label, 'Cluster overview')]"
        )),
        60,
    )
    .await?;

    // ---- 4. Browse pods through the sidebar's tree. Its groups start
    // collapsed, so Workloads must be DISCLOSED before its Pods entry exists
    // in the DOM at all. Clicked only while it says it is collapsed, so a
    // future default-open change can't turn the click into a collapse.
    let tree = "//*[@role='tree' and @aria-label='Cluster resources']";
    let workloads = format!("{tree}//*[@role='treeitem' and normalize-space()='Workloads']");
    let group = wait_for(driver, By::XPath(workloads.clone()), 30).await?;
    if group.attr("aria-expanded").await?.as_deref() == Some("false") {
        click(driver, By::XPath(format!("{workloads}[@aria-expanded='false']")), 15).await?;
    }
    click(
        driver,
        By::XPath(format!("{tree}//*[@role='treeitem' and normalize-space()='Pods']")),
        15,
    )
    .await?;

    // ---- 5. The fixture pod is in the default namespace, which a fresh
    // profile's "all namespaces" includes. A single click on its row opens
    // the peek beside the list. Rows the table renders aria-hidden — its
    // row-height sample — are not the row a reader clicks. The peek's action
    // bar (Logs and Shell on it, the rest in its overflow) is the proof.
    let actions = "//*[@role='group' and @aria-label='Pod actions']";
    click_until(
        driver,
        &format!("//tbody//tr[not(@aria-hidden='true')]//*[normalize-space(text())='{POD}']"),
        By::XPath(actions.to_string()),
        60,
    )
    .await?;

    if full {
        // ---- 5F. Port-forward: Forward is in the bar's overflow. The dialog
        // arrives with the pod and its namespace filled in; remote 8080 →
        // explicit local 18080 are typed (a blank local port would let the
        // backend pick one this test could not know), then real bytes are
        // fetched through it from OUTSIDE the app.
        click(driver, By::XPath(format!("{actions}//button[normalize-space()='More actions']")), 30)
            .await?;
        click(
            driver,
            By::XPath(
                "//*[@role='dialog' and @aria-label='More actions']//button[@aria-label='Forward']"
                    .to_string(),
            ),
            15,
        )
        .await?;
        let dialog = "//*[@role='dialog'][.//*[normalize-space()='New port forward']]";
        for (label, port) in [("Local port", "18080"), ("Remote port", "8080")] {
            let field = wait_for(
                driver,
                By::XPath(format!("{dialog}//label[normalize-space(span)='{label}']//input")),
                15,
            )
            .await?;
            field.clear().await?;
            field.send_keys(port).await?;
        }
        click(
            driver,
            By::XPath(format!("{dialog}//button[normalize-space()='Start forward' and not(@disabled)]")),
            30,
        )
        .await?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if let Ok(mut s) = std::net::TcpStream::connect(("127.0.0.1", 18080)) {
                use std::io::{Read, Write};
                // Bounded I/O: an accepted connection whose upstream stalls
                // (port-forward handshake wedged) must fail THIS iteration,
                // not hang the whole job past its deadline with an
                // unbounded read.
                let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
                let _ = s.set_write_timeout(Some(Duration::from_secs(5)));
                let _ = s.write_all(b"GET / HTTP/1.0\r\n\r\n");
                let mut buf = String::new();
                let _ = s.read_to_string(&mut buf);
                if buf.contains("ok") {
                    break;
                }
            }
            if Instant::now() >= deadline {
                return Err("port-forward never served the fixture's response".into());
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }

        // ---- 6F. Exec: Shell on the bar starts a session in the pod's one
        // container and opens the Terminals tab on it. xterm's input is the
        // textarea it names "Terminal input".
        click(driver, By::XPath(format!("{actions}//button[normalize-space()='Shell']")), 30).await?;
        let input = wait_for(driver, By::Css("textarea[aria-label='Terminal input']"), 30).await?;
        // Prove the keystrokes actually EXECUTE in the pod via a side effect
        // observed from OUTSIDE the app (kubectl), not from the terminal's
        // rendering: a renderer addon added later would break a DOM-text
        // assertion only on stable-release full runs, the worst place to
        // discover it, and a file can never be faked by keystroke echo.
        //
        // Typed again until the marker lands, rather than once after a
        // readiness signal: xterm mounts before the exec handshake completes,
        // keys typed before the far end is wired are dropped (`lib/sessions`),
        // and the session's `Attached` badge is drawn from the start, so
        // nothing on screen says when keys become real. `touch` is idempotent,
        // so a line that did land and is typed again changes nothing.
        let deadline = Instant::now() + Duration::from_secs(60);
        'typed: loop {
            input.focus().await?;
            driver.action_chain().send_keys("touch /tmp/smoke-exec-done\n").perform().await?;
            for _ in 0..5 {
                tokio::time::sleep(Duration::from_secs(2)).await;
                if kubectl(&[
                    "--context", context, "exec", POD, "--", "test", "-f", "/tmp/smoke-exec-done",
                ])
                .is_ok()
                {
                    break 'typed;
                }
            }
            if Instant::now() >= deadline {
                return Err("the exec'd command never left its marker in the pod".into());
            }
        }

        // Back to the Pods tab, where the peek is still open beside the list.
        click(
            driver,
            By::XPath(
                "//*[@role='tablist' and @aria-label='Open tabs']//*[@role='tab' and starts-with(@aria-label, 'Pods')]"
                    .to_string(),
            ),
            15,
        )
        .await?;
    }

    // ---- 7. Logs: the bar's Logs opens the pod's log stream in a tab, and
    // the marker must appear INSIDE that stream (`role="log"` named
    // `<pod> logs`, screens/Logs.tsx) — a global text match could be
    // satisfied by the pod SPEC in the peek's YAML, which embeds the
    // fixture's command (also why the command logs the marker via an
    // expansion the spec doesn't contain). Only real log output can put it
    // here.
    click(driver, By::XPath(format!("{actions}//button[normalize-space()='Logs']")), 30).await?;
    wait_for(
        driver,
        By::XPath(format!(
            "//*[@role='log' and @aria-label='{POD} logs']//*[contains(normalize-space(.), '{LOG_MARKER}')]"
        )),
        60,
    )
    .await?;

    Ok(())
}
