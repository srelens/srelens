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

/// Wait until something on the page contains `needle`.
async fn wait_text(driver: &WebDriver, needle: &str, secs: u64) -> WebDriverResult<WebElement> {
    driver
        .query(By::XPath(format!("//*[contains(normalize-space(.), '{needle}')]")))
        .wait(Duration::from_secs(secs), Duration::from_millis(500))
        .first()
        .await
}

/// Click the first element matching `by`, retrying until `secs` — on
/// absence AND on click rejection. The retry-on-intercepted half matters as
/// much as the wait: the app mounts views UNDER overlays (the vault gate,
/// dialogs), so a target can exist and still be legitimately covered for
/// seconds — e.g. the landing page behind the gate while a debug build's
/// argon2id derivation finishes. One attempt would fail instantly with
/// ElementClickIntercepted; retrying makes every click "when actually
/// clickable, within the deadline".
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

async fn button_by_text(driver: &WebDriver, text: &str, secs: u64) -> WebDriverResult<()> {
    click(driver, By::XPath(format!("//button[normalize-space()='{text}']")), secs).await
}

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
    // ---- 1. First-launch vault gate: create the master password. The
    // recovery checkbox is UNCHECKED first — CI has no OS keychain, and the
    // setup must not depend on one.
    driver
        .query(By::Css("input[placeholder='At least 8 characters']"))
        .wait(Duration::from_secs(60), Duration::from_millis(500))
        .first()
        .await?;
    // The gate is driven by SCRIPT, not synthesized input: WebDriver-level
    // interaction with this form proved environment-flaky in two different
    // ways (CI intercepted the button click; the local container
    // intercepted the label-nested checkbox and dropped keys mid-render).
    // The gate is prerequisite plumbing — the WebView↔Rust bridge under
    // test is exercised by everything AFTER it, which stays real
    // interaction. React controlled inputs need the native value setter +
    // an input event; requestSubmit fires the form's onSubmit exactly like
    // a user submission.
    driver
        .execute(
            r#"
            const set = Object.getOwnPropertyDescriptor(
                window.HTMLInputElement.prototype, 'value').set;
            for (const el of document.querySelectorAll("input[type='password']")) {
                set.call(el, arguments[0]);
                el.dispatchEvent(new Event('input', { bubbles: true }));
            }
            const cb = document.querySelector("input[type='checkbox'].accent-primary");
            if (cb && cb.checked) cb.click();
            document.querySelector('form').requestSubmit();
            "#,
            vec![serde_json::json!(MASTER_PASSWORD)],
        )
        .await?;

    // ---- 2. Land, and open the kind context from the landing page.
    click(driver, By::Css(&format!("[aria-label='Open context {context}']")), 60).await?;

    // ---- 3. Browse pods. Opening a context lands on Overview, so only the
    // Cluster section starts expanded (`openByDefault` keys off the active
    // kind in Sidebar.tsx) — the Workloads section must be DISCLOSED before
    // its Pods entry exists in the DOM at all. Pods is still tried briefly
    // first, so a future default-open change can't break this by turning
    // the heading click into a collapse.
    wait_text(driver, "Workloads", 30).await?;
    if button_by_text(driver, "Pods", 3).await.is_err() {
        click(
            driver,
            By::XPath("//button[contains(normalize-space(.), 'Workloads')]".to_string()),
            15,
        )
        .await?;
        button_by_text(driver, "Pods", 15).await?;
    }

    // ---- 4. The fixture pod is in the default namespace; open its detail.
    click(
        driver,
        By::XPath(format!("//*[contains(normalize-space(text()), '{POD}')]")),
        60,
    )
    .await?;

    if full {
        // ---- 5F. Port-forward: remote 8080 → explicit local 18080, then
        // prove real bytes flow by fetching from OUTSIDE the app.
        click(driver, By::Css("[aria-label='Forward']"), 30).await?;
        let remote = driver
            .query(By::Css("input[placeholder='e.g. 80']"))
            .wait(Duration::from_secs(15), Duration::from_millis(400))
            .first()
            .await?;
        remote.send_keys("8080").await?;
        // The local-port input is the next numeric text input in the dialog.
        let inputs = driver.find_all(By::Css("div[role='dialog'] input")).await?;
        if let Some(local) = inputs.get(1) {
            local.send_keys("18080").await?;
        }
        button_by_text(driver, "Start forward", 10).await.or(button_by_text(driver, "Forward", 10).await)?;
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

        // ---- 6F. Exec: open a shell, run a command whose OUTPUT differs
        // from its typed form (so matching the output proves execution, not
        // an echo of the keystrokes).
        click(driver, By::Css("[aria-label='Shell']"), 30).await?;
        let term = driver
            .query(By::Css(".xterm"))
            .wait(Duration::from_secs(30), Duration::from_millis(500))
            .first()
            .await?;
        // xterm mounts BEFORE the exec handshake completes, and TerminalPane
        // discards input while it still shows `connecting…` (`conn?.send`) —
        // typing early is silently lost. The status strip renders `live`
        // once the websocket is actually attached; only then are keys real.
        // Scoped to the TERMINAL's own indicator (the emerald span inside
        // the dark pane): ResourceBrowser's watch badge also says `live`
        // and stays mounted behind the dock, so a global text wait would
        // pass while the exec socket is still connecting.
        driver
            .query(By::XPath(
                "//div[contains(@class,'bg-[#1b1f23]')]//span[contains(@class,'text-emerald-400') and contains(normalize-space(.),'live')]"
                    .to_string(),
            ))
            .wait(Duration::from_secs(60), Duration::from_millis(500))
            .first()
            .await?;
        term.click().await.ok();
        // Prove the keystrokes actually EXECUTE in the pod via a side effect
        // observed from OUTSIDE the app (kubectl), not from the terminal's
        // rendering. Today that rendering would even be assertable — no
        // canvas/webgl addon is loaded, so xterm 5 uses its DOM renderer and
        // output text lands in real DOM nodes — but a renderer addon added
        // later would break a DOM-text assertion only on stable-release full
        // runs, the worst place to discover it. A file can also never be
        // faked by keystroke echo, which no output-matching scheme fully
        // rules out.
        driver.action_chain().send_keys("touch /tmp/smoke-exec-done\n").perform().await?;
        let deadline = Instant::now() + Duration::from_secs(60);
        loop {
            if kubectl(&[
                "--context", context, "exec", POD, "--", "test", "-f", "/tmp/smoke-exec-done",
            ])
            .is_ok()
            {
                break;
            }
            if Instant::now() >= deadline {
                return Err("the exec'd command never left its marker in the pod".into());
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    }

    // ---- 7. Logs: the marker must appear INSIDE the log pane
    // (`role="log"`, LogsView.tsx) — a global text match could be satisfied
    // by the pod SPEC in the detail overview, which embeds the fixture's
    // command (also why the command logs the marker via an expansion the
    // spec doesn't contain). Only real log output can put it here.
    click(driver, By::Css("[aria-label='Logs']"), 30).await?;
    driver
        .query(By::XPath(format!(
            "//*[@role='log']//*[contains(normalize-space(.), '{LOG_MARKER}')]"
        )))
        .wait(Duration::from_secs(60), Duration::from_millis(500))
        .first()
        .await?;

    Ok(())
}
