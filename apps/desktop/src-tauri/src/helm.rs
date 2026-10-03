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
    pub fn let_go(&self, session: u64, window: &str, reason: &'static str) -> bool {
        let op = self.0.lock().unwrap().remove(&session);
        op.is_some_and(|op| op.let_go(window, reason))
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
    Exited,
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
        matches!(*self.state.lock().unwrap(), OpState::Exited)
    }

    fn let_go(&self, window: &str, reason: &'static str) -> bool {
        let mut state = self.state.lock().unwrap();
        if !matches!(*state, OpState::Watched) {
            return false;
        }
        *state = OpState::LetGo {
            window: window.to_owned(),
            reason,
        };
        true
    }

    /// helm exited, with `outcome` as it is sent on `helm:exit`: `null` for
    /// success, else why it failed. Reported here when its window let go.
    fn exited<R: Runtime>(&self, app: &AppHandle<R>, outcome: &Value) {
        let was = std::mem::replace(&mut *self.state.lock().unwrap(), OpState::Exited);
        let OpState::LetGo { window, reason } = was else {
            return;
        };
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
        app,
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
    ops.watch(session, op);
    owned.keep(window.label(), epoch, Stream::Helm(session), || {
        ops.let_go(session, window.label(), "closed or reloaded");
    })?;
    Ok(session)
}

/// Abort a running helm operation (best-effort) and drop its session.
#[tauri::command]
pub async fn helm_op_close<R: Runtime>(
    session: u64,
    window: Window<R>,
    manager: State<'_, HelmManager>,
    owned: State<'_, WindowStreams>,
    ops: State<'_, HelmOps>,
) -> Result<(), String> {
    manager.close(session);
    owned.disown(window.label(), &Stream::Helm(session));
    ops.forget(session);
    Ok(())
}
