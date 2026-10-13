//! Tauri adapter for live log tails: the streaming core lives in
//! srelens_streams::logs; this module only maps the Tauri command surface.
//!
//! Commands are generic over the runtime (#28): the unit suite below drives
//! them through `tauri::test::MockRuntime`, so this surface counts toward
//! coverage instead of hiding behind the ignore-regex.

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use srelens_streams::log_spool::SpoolPage;
use srelens_streams::logs::{LogStreamManager, LogTarget};
use tauri::{AppHandle, Manager, Runtime, State, Window};
use tauri_plugin_dialog::DialogExt;

use crate::sink::TauriSink;
use crate::window_streams::{Stream, WindowStreams};

/// Start following the given targets, emitting each line as a `LogLine` on the
/// caller-provided `channel`. The WebView subscribes to `channel` first, then
/// invokes this, so the initial tail lines can't race ahead of the listener.
///
/// The stream belongs to the calling window and stops when it closes or
/// reloads (#735). One whose window reloaded while it was starting is stopped
/// at once and refused. `spool_max_bytes` keeps the stream's history on disk
/// (#914).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn start_log_stream<R: Runtime>(
    context: String,
    namespace: String,
    targets: Vec<LogTarget>,
    channel: String,
    timestamps: Option<bool>,
    since_seconds: Option<i64>,
    tail_lines: Option<i64>,
    spool_max_bytes: Option<u64>,
    app: AppHandle<R>,
    window: Window<R>,
    manager: State<'_, LogStreamManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    let epoch = owned.epoch(window.label());
    manager
        .start(
            Arc::new(TauriSink(app)),
            context,
            namespace,
            targets,
            channel.clone(),
            timestamps,
            since_seconds,
            tail_lines,
            spool_max_bytes,
        )
        .await?;
    owned.keep(window.label(), epoch, Stream::Log(channel.clone()), || {
        manager.stop(&channel)
    })
}

/// Stop a log-tail stream and abort all of its follow tasks. Only the window
/// that started it may (#733, #735); one no window holds has ended, and this
/// is a no-op.
#[tauri::command]
pub async fn stop_log_stream<R: Runtime>(
    channel: String,
    window: Window<R>,
    manager: State<'_, LogStreamManager>,
    owned: State<'_, WindowStreams>,
) -> Result<(), String> {
    let stream = Stream::Log(channel.clone());
    if owned.check(window.label(), &stream)? {
        manager.stop(&channel);
        owned.disown(window.label(), &stream);
    }
    Ok(())
}

/// Lines `[before - limit, before)` of a stream's history (#914). Only the
/// window that opened the stream may read it.
#[tauri::command]
pub async fn read_log_spool<R: Runtime>(
    channel: String,
    before: u64,
    limit: usize,
    window: Window<R>,
    manager: State<'_, LogStreamManager>,
    owned: State<'_, WindowStreams>,
) -> Result<SpoolPage, String> {
    owned_by(&owned, window.label(), &channel)?;
    manager.read(&channel, before, limit).await
}

/// Save a stream's history through the native save dialog (#914): every line,
/// or those of the last `since_seconds`. Rust copies it from the spool, so
/// nothing passes through the webview. The saved path, or `None` when the
/// reader cancelled.
#[tauri::command]
pub async fn export_log_spool<R: Runtime>(
    app: AppHandle<R>,
    channel: String,
    filename: String,
    since_seconds: Option<u64>,
    window: Window<R>,
    manager: State<'_, LogStreamManager>,
    owned: State<'_, WindowStreams>,
) -> Result<Option<String>, String> {
    owned_by(&owned, window.label(), &channel)?;
    let picked = app
        .dialog()
        .file()
        .set_file_name(&filename)
        .blocking_save_file();
    let Some(file) = picked else {
        return Ok(None);
    };
    let path = file.as_path().ok_or("invalid save path")?.to_path_buf();
    export_to(&manager, &channel, &path, since_seconds).await?;
    Ok(Some(path.to_string_lossy().into_owned()))
}

/// Refused unless `window` opened `channel` and it is still running.
fn owned_by(owned: &WindowStreams, window: &str, channel: &str) -> Result<(), String> {
    if owned.check(window, &Stream::Log(channel.to_owned()))? {
        Ok(())
    } else {
        Err("this log stream has ended".into())
    }
}

/// Write `channel`'s history to `path`, through a `.partial` file beside it that
/// replaces `path` only once whole. An export that could not finish is never
/// left looking whole, and a file already at `path` survives it. A
/// `since_seconds` reaching back past the clock's start means every line.
pub(crate) async fn export_to(
    manager: &LogStreamManager,
    channel: &str,
    path: &Path,
    since_seconds: Option<u64>,
) -> Result<(), String> {
    let since = since_seconds.map(srelens_streams::log_spool::cutoff);
    let mut partial = path.as_os_str().to_owned();
    partial.push(".partial");
    let partial = PathBuf::from(partial);
    let file = File::create(&partial).map_err(|e| e.to_string())?;
    let written = manager.export(channel, BufWriter::new(file), since).await;
    let saved = match written {
        Ok(_) => std::fs::rename(&partial, path).map_err(|e| e.to_string()),
        Err(e) => Err(e),
    };
    if saved.is_err() {
        let _ = std::fs::remove_file(&partial);
    }
    saved
}

/// srelens is quitting: stop every log stream and remove the history it kept.
pub fn quit<R: Runtime>(app: &AppHandle<R>) {
    if let Some(manager) = app.try_state::<LogStreamManager>() {
        manager.shutdown_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use srelens_kube::client_cache::ClientCache;
    use srelens_streams::log_spool::{SpoolDirs, DESKTOP_SPOOL_CEILING};
    use std::time::Duration;

    /// The empty-target refusal comes back synchronously; a real target is
    /// accepted (its follow task dies later on the unresolvable context) and
    /// stop tears the stream down — plus the unknown-channel no-op.
    #[tokio::test(flavor = "multi_thread")]
    async fn commands_run_against_a_mock_runtime() {
        let app = tauri::test::mock_app();
        app.manage(LogStreamManager::new(ClientCache::new_many(vec![])));
        app.manage(WindowStreams::default());
        let window = crate::window_streams::tests::mock_window(&app, "main");

        let e = start_log_stream(
            "no-such-context".into(),
            "ns".into(),
            vec![],
            "logs:test".into(),
            None,
            None,
            None,
            None,
            app.handle().clone(),
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert!(e.contains("without a pod target"), "unexpected error: {e}");

        start_log_stream(
            "no-such-context".into(),
            "ns".into(),
            vec![LogTarget {
                pod: "pod-0".into(),
                container: None,
                label: String::new(),
            }],
            "logs:test".into(),
            Some(true),
            Some(60),
            Some(100),
            None,
            app.handle().clone(),
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        stop_log_stream("logs:test".into(), window.clone(), app.state(), app.state())
            .await
            .unwrap();
        stop_log_stream("logs:unknown".into(), window, app.state(), app.state())
            .await
            .unwrap();
    }

    fn spooled_app(tmp: &tempfile::TempDir) -> tauri::App<tauri::test::MockRuntime> {
        let app = tauri::test::mock_app();
        app.manage(
            LogStreamManager::new(ClientCache::new_many(vec![])).with_spools(
                SpoolDirs::per_process(tmp.path().to_path_buf(), DESKTOP_SPOOL_CEILING),
            ),
        );
        app.manage(WindowStreams::default());
        app
    }

    fn target() -> Vec<LogTarget> {
        vec![LogTarget {
            pod: "pod-0".into(),
            container: None,
            label: String::new(),
        }]
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_window_reads_back_the_history_of_its_own_stream_and_no_other_window_can() {
        let tmp = tempfile::tempdir().unwrap();
        let app = spooled_app(&tmp);
        let window = crate::window_streams::tests::mock_window(&app, "main");
        let other = crate::window_streams::tests::mock_window(&app, "other");
        start_log_stream(
            "no-such-context".into(),
            "ns".into(),
            target(),
            "logs:test".into(),
            None,
            None,
            None,
            Some(64 << 20),
            app.handle().clone(),
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();

        let mut lines = 0;
        for _ in 0..300 {
            let page = read_log_spool(
                "logs:test".into(),
                u64::MAX,
                10,
                window.clone(),
                app.state(),
                app.state(),
            )
            .await
            .unwrap();
            lines = page.lines.len();
            if lines > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert!(lines > 0, "the stream's error line is in its history");

        let refused = read_log_spool(
            "logs:test".into(),
            u64::MAX,
            10,
            other,
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert!(refused.contains("not opened by this window"), "{refused}");
        let ended = read_log_spool("logs:gone".into(), 10, 10, window, app.state(), app.state())
            .await
            .unwrap_err();
        assert!(ended.contains("has ended"), "{ended}");
    }

    /// Start `logs:test` on a target that cannot connect, and wait until its
    /// error line is in the history.
    async fn stream_with_history(
        app: &tauri::App<tauri::test::MockRuntime>,
        window: &Window<tauri::test::MockRuntime>,
    ) {
        start_log_stream(
            "no-such-context".into(),
            "ns".into(),
            target(),
            "logs:test".into(),
            None,
            None,
            None,
            Some(64 << 20),
            app.handle().clone(),
            window.clone(),
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        let manager = app.state::<LogStreamManager>();
        for _ in 0..300 {
            if !manager
                .read("logs:test", u64::MAX, 1)
                .await
                .unwrap()
                .lines
                .is_empty()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn export_to_writes_the_history_and_leaves_no_file_when_it_cannot() {
        let tmp = tempfile::tempdir().unwrap();
        let app = spooled_app(&tmp);
        let window = crate::window_streams::tests::mock_window(&app, "main");
        stream_with_history(&app, &window).await;
        let manager = app.state::<LogStreamManager>();
        let out = tempfile::tempdir().unwrap();
        let saved = out.path().join("pod-0.log");
        export_to(&manager, "logs:test", &saved, None)
            .await
            .unwrap();
        assert!(std::fs::read_to_string(&saved)
            .unwrap()
            .starts_with("pod-0 | [error:"));
        assert!(
            !out.path().join("pod-0.log.partial").exists(),
            "a finished export leaves no partial file"
        );

        let failed = out.path().join("nothing.log");
        assert!(export_to(&manager, "logs:none", &failed, None)
            .await
            .is_err());
        assert!(!failed.exists(), "a half-written export is not left behind");
        assert!(
            !out.path().join("nothing.log.partial").exists(),
            "a failed export removes its partial file"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_failed_export_leaves_the_file_already_at_the_path_as_it_was() {
        let tmp = tempfile::tempdir().unwrap();
        let app = spooled_app(&tmp);
        let window = crate::window_streams::tests::mock_window(&app, "main");
        stream_with_history(&app, &window).await;
        let manager = app.state::<LogStreamManager>();
        let out = tempfile::tempdir().unwrap();
        let kept = out.path().join("kept.log");
        std::fs::write(&kept, "an earlier export").unwrap();

        assert!(export_to(&manager, "logs:none", &kept, None).await.is_err());
        assert_eq!(
            std::fs::read_to_string(&kept).unwrap(),
            "an earlier export",
            "the file the reader chose to replace survives an export that failed"
        );
        assert!(!out.path().join("kept.log.partial").exists());

        export_to(&manager, "logs:test", &kept, None).await.unwrap();
        assert!(
            std::fs::read_to_string(&kept)
                .unwrap()
                .starts_with("pod-0 | [error:"),
            "a finished export replaces it"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_export_since_more_seconds_than_there_have_been_keeps_every_line() {
        let tmp = tempfile::tempdir().unwrap();
        let app = spooled_app(&tmp);
        let window = crate::window_streams::tests::mock_window(&app, "main");
        stream_with_history(&app, &window).await;
        let manager = app.state::<LogStreamManager>();
        let out = tempfile::tempdir().unwrap();
        let saved = out.path().join("pod-0.log");
        // The webview sends this number, so no value of it may panic.
        export_to(&manager, "logs:test", &saved, Some(u64::MAX))
            .await
            .unwrap();
        assert!(std::fs::read_to_string(&saved)
            .unwrap()
            .starts_with("pod-0 | [error:"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_export_never_reaches_the_save_dialog() {
        let tmp = tempfile::tempdir().unwrap();
        // No dialog plugin is installed here, so reaching `app.dialog()` would
        // panic: an error from each call shows the check ran before it.
        let app = spooled_app(&tmp);
        let window = crate::window_streams::tests::mock_window(&app, "main");
        let other = crate::window_streams::tests::mock_window(&app, "other");
        stream_with_history(&app, &window).await;

        let refused = export_log_spool(
            app.handle().clone(),
            "logs:test".into(),
            "x.log".into(),
            None,
            other,
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert!(refused.contains("not opened by this window"), "{refused}");
        let ended = export_log_spool(
            app.handle().clone(),
            "logs:gone".into(),
            "x.log".into(),
            None,
            window,
            app.state(),
            app.state(),
        )
        .await
        .unwrap_err();
        assert!(ended.contains("has ended"), "{ended}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn quitting_removes_every_spool() {
        let tmp = tempfile::tempdir().unwrap();
        let app = spooled_app(&tmp);
        let window = crate::window_streams::tests::mock_window(&app, "main");
        start_log_stream(
            "no-such-context".into(),
            "ns".into(),
            target(),
            "logs:test".into(),
            None,
            None,
            None,
            Some(64 << 20),
            app.handle().clone(),
            window,
            app.state(),
            app.state(),
        )
        .await
        .unwrap();
        assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_some());
        quit(app.handle());
        assert!(std::fs::read_dir(tmp.path()).unwrap().next().is_none());
    }
}
