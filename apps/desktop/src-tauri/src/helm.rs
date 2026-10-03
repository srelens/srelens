//! Tauri adapter for helm write operations: the streaming core lives in
//! srelens_streams::helm; this module only maps the Tauri command surface
//! (and desktop kubeconfig discovery) onto it.
//!
//! **An operation outlives its window (#735).** Every operation the UI runs
//! here — install, upgrade, rollback, uninstall — changes the cluster, and
//! there is no safe point to stop one: `helm_op_close` aborts the task owning
//! helm, which is spawned `kill_on_drop`, so helm is killed wherever it had
//! reached and the release can be left `pending-upgrade` with nothing saying
//! why. So when the window that started one closes or reloads, the window
//! lets go of it ([`HelmOps::let_go`]) and helm runs to the end. How it ended
//! is then the host's to report, since the page that would have is gone: to
//! the app log, and to every open window as a host notice.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use srelens_streams::helm::HelmManager;
use srelens_streams::EventSink;
use tauri::{AppHandle, Emitter, Runtime, State, Window};

use crate::host_notice::{self, Level};
use crate::window_streams::{Stream, WindowStreams};

/// The helm operations still running, by session, so a window that goes can
/// let go of its own.
#[derive(Default)]
pub struct HelmOps(Mutex<HashMap<u64, Arc<HelmOp>>>);

impl HelmOps {
    fn watch(&self, session: u64, op: Arc<HelmOp>) {
        let mut ops = self.0.lock().unwrap();
        ops.retain(|_, op| !op.has_exited());
        ops.insert(session, op);
    }

    /// The window that started `session` is gone (`reason` says how): helm
    /// runs on, and how it ends is reported. `false` when it had already
    /// ended, or is not known.
    pub fn let_go<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        session: u64,
        window: &str,
        reason: &'static str,
    ) -> bool {
        let op = self.0.lock().unwrap().remove(&session);
        op.is_some_and(|op| op.let_go(app, window, reason))
    }

    fn forget(&self, session: u64) {
        self.0.lock().unwrap().remove(&session);
    }
}

/// One operation, as its report names it.
pub struct HelmOp {
    /// "helm upgrade web": the subcommand and release it was run with.
    what: String,
    context: String,
    state: Mutex<OpState>,
}

enum OpState {
    /// Its page is listening, and says how it ends.
    Watched,
    /// Its window closed or reloaded while it ran: the host says how it ends.
    LetGo {
        window: String,
        reason: &'static str,
    },
    /// helm exited, at `at`, with `outcome` as `helm:exit` carries it.
    Exited {
        outcome: Value,
        at: std::time::Instant,
    },
}

impl HelmOp {
    fn new(args: &[String], context: &str) -> Self {
        let named: Vec<&str> = args
            .iter()
            .map(String::as_str)
            .take_while(|arg| !arg.starts_with('-'))
            .take(2)
            .collect();
        Self {
            what: format!("helm {}", named.join(" ")),
            context: context.to_owned(),
            state: Mutex::new(OpState::Watched),
        }
    }

    fn has_exited(&self) -> bool {
        matches!(*self.state.lock().unwrap(), OpState::Exited { .. })
    }

    /// The start was refused: `window` closed or reloaded while helm was being
    /// started, so the page that asked is gone and never had the session.
    /// Its outcome is the host's to report — now, if helm has already exited
    /// in that race, or else when it does.
    fn abandon<R: Runtime>(&self, app: &AppHandle<R>, window: &str) {
        const REASON: &str = "closed or reloaded";
        let mut state = self.state.lock().unwrap();
        match &*state {
            OpState::Watched => {
                *state = OpState::LetGo {
                    window: window.to_owned(),
                    reason: REASON,
                }
            }
            OpState::Exited { outcome, .. } => {
                let outcome = outcome.clone();
                drop(state);
                self.report(app, window, REASON, &outcome);
            }
            OpState::LetGo { .. } => {}
        }
    }

    /// `window` closed or reloaded (`reason` says which). A running operation
    /// is let go of, and reported when it ends: `true`. One that exited only
    /// just now may have gone out as the page went, unseen, so it is reported
    /// at once; one that ended earlier was its page's to show.
    fn let_go<R: Runtime>(&self, app: &AppHandle<R>, window: &str, reason: &'static str) -> bool {
        /// How long before its window went an exit may not have been seen.
        const JUST_EXITED: std::time::Duration = std::time::Duration::from_secs(5);
        let mut state = self.state.lock().unwrap();
        match &*state {
            OpState::Watched => {
                *state = OpState::LetGo {
                    window: window.to_owned(),
                    reason,
                };
                true
            }
            OpState::Exited { outcome, at } if at.elapsed() < JUST_EXITED => {
                let outcome = outcome.clone();
                drop(state);
                self.report(app, window, reason, &outcome);
                false
            }
            _ => false,
        }
    }

    /// helm exited, with `outcome` as it is sent on `helm:exit`: `null` for
    /// success, else why it failed. Reported here when its window let go.
    fn exited<R: Runtime>(&self, app: &AppHandle<R>, outcome: &Value) {
        let was = std::mem::replace(
            &mut *self.state.lock().unwrap(),
            OpState::Exited {
                outcome: outcome.clone(),
                at: std::time::Instant::now(),
            },
        );
        if let OpState::LetGo { window, reason } = was {
            self.report(app, &window, reason, outcome);
        }
    }

    /// Say how helm ended, after `window` went (`reason` says how): to the app
    /// log, and to every open window.
    fn report<R: Runtime>(&self, app: &AppHandle<R>, window: &str, reason: &str, outcome: &Value) {
        let (what, context) = (&self.what, &self.context);
        match outcome.as_str() {
            None => {
                log::info!("{what} on {context} finished, after its window {window} {reason}");
                host_notice::notify(
                    app,
                    Level::Info,
                    &format!("{what} finished"),
                    &format!("On {context}, after its window {reason}."),
                );
            }
            Some(error) => {
                log::warn!("{what} on {context} failed, after its window {window} {reason}: {error}");
                host_notice::notify(
                    app,
                    Level::Error,
                    &format!("{what} failed"),
                    &format!("{error}. On {context}, after its window {reason}."),
                );
            }
        }
    }
}

/// What the helm task emits on: the windows hear everything as before, and
/// the exit also settles the operation.
struct OpSink<R: Runtime> {
    app: AppHandle<R>,
    exit: String,
    op: Arc<HelmOp>,
}

impl<R: Runtime> EventSink for OpSink<R> {
    fn emit(&self, channel: &str, payload: Value) {
        if channel == self.exit {
            self.op.exited(&self.app, &payload);
        }
        let _ = self.app.emit(channel, payload);
    }
}

/// Run `helm <args>` scoped to `context`, streaming stdout+stderr on
/// `helm:out:<channel>`; `helm:exit:<channel>` fires with None on success or an
/// error string on failure. Returns the session id.
///
/// The operation belongs to the calling window, which lets go of it — never
/// kills it — when it closes or reloads; see the module docs. One whose
/// window reloaded while it was starting is let go of at once, and refused.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_helm_op<R: Runtime>(
    context: String,
    extra_kubeconfigs: Vec<String>,
    args: Vec<String>,
    values: String,
    channel: String,
    app: AppHandle<R>,
    window: Window<R>,
    manager: State<'_, HelmManager>,
    owned: State<'_, WindowStreams>,
    ops: State<'_, HelmOps>,
) -> Result<u64, String> {
    let epoch = owned.epoch(window.label());
    let mut paths = crate::capabilities::all_kubeconfig_paths();
    paths.extend(extra_kubeconfigs.iter().map(std::path::PathBuf::from));
    let op = Arc::new(HelmOp::new(&args, &context));
    let sink = OpSink {
        app: app.clone(),
        exit: format!("helm:exit:{channel}"),
        op: op.clone(),
    };
    let session = manager
        .start(
            Arc::new(sink),
            context,
            paths,
            args,
            values,
            channel,
            // Desktop is single-user: keep helm's default home unchanged.
            None,
        )
        .await?;
    ops.watch(session, op.clone());
    owned.keep(window.label(), epoch, Stream::Helm(session), || {
        ops.forget(session);
        op.abandon(&app, window.label());
    })?;
    Ok(session)
}

/// Abort a running helm operation (best-effort) and drop its session. Only the
/// window that started it may (#733, #735): aborting helm partway leaves the
/// release half-applied. An operation no window holds has ended or been let
/// go of, and this is a no-op.
#[tauri::command]
pub async fn helm_op_close<R: Runtime>(
    session: u64,
    window: Window<R>,
    manager: State<'_, HelmManager>,
    owned: State<'_, WindowStreams>,
    ops: State<'_, HelmOps>,
) -> Result<(), String> {
    let stream = Stream::Helm(session);
    if owned.check(window.label(), &stream)? {
        manager.close(session);
        owned.disown(window.label(), &stream);
        ops.forget(session);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::Listener;

    fn notices(app: &tauri::App<tauri::test::MockRuntime>) -> Arc<Mutex<Vec<Value>>> {
        let heard = Arc::new(Mutex::new(Vec::new()));
        let record = heard.clone();
        app.listen_any("host-notice", move |e| {
            record
                .lock()
                .unwrap()
                .push(serde_json::from_str(e.payload()).unwrap_or_default());
        });
        heard
    }

    fn upgrade() -> HelmOp {
        HelmOp::new(&["upgrade".into(), "web".into(), "./chart".into()], "prod")
    }

    /// The late-start race: helm exited while its window was closing or
    /// reloading, before the start's own ownership check refused it. The page
    /// that asked is gone and never had the session, so it heard nothing — the
    /// outcome is the host's to report, though helm has already exited.
    #[test]
    fn an_operation_that_ended_before_its_late_start_was_refused_is_still_reported() {
        let app = tauri::test::mock_app();
        let heard = notices(&app);
        let op = upgrade();
        op.exited(app.handle(), &Value::String("helm exited with code 1".into()));
        assert!(heard.lock().unwrap().is_empty(), "its page might still be listening");

        op.abandon(app.handle(), "main");

        let heard = heard.lock().unwrap();
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert_eq!(heard[0]["level"], "error");
        assert_eq!(heard[0]["title"], "helm upgrade web failed");
    }

    /// An operation that ended while its page was there was heard by that
    /// page, so a window that goes afterwards reports nothing about it.
    #[test]
    fn an_operation_its_page_saw_end_is_not_reported_again() {
        let app = tauri::test::mock_app();
        let heard = notices(&app);
        let op = upgrade();
        op.exited(app.handle(), &Value::Null);
        op.exited_long_ago();

        assert!(!op.let_go(app.handle(), "main", "closed"));
        assert!(heard.lock().unwrap().is_empty());
    }

    /// helm exited just as its window closed or reloaded (#799 review): the
    /// exit went out while the page was going, and it may never have shown
    /// it. An operation that ended that recently is reported when its window
    /// lets go — a second sight of an outcome beats none.
    #[test]
    fn an_operation_that_ended_just_as_its_window_went_is_reported() {
        let app = tauri::test::mock_app();
        let heard = notices(&app);
        let op = upgrade();
        op.exited(app.handle(), &Value::String("helm exited with code 1".into()));

        assert!(!op.let_go(app.handle(), "main", "reloaded"), "it is not running");

        let heard = heard.lock().unwrap();
        assert_eq!(heard.len(), 1, "{heard:?}");
        assert_eq!(heard[0]["title"], "helm upgrade web failed");
    }

    impl HelmOp {
        /// As if helm had exited a while before now.
        fn exited_long_ago(&self) {
            if let OpState::Exited { at, .. } = &mut *self.state.lock().unwrap() {
                *at -= std::time::Duration::from_secs(60);
            }
        }
    }
}
