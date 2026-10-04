//! What the host tells every open window when the page that would have heard
//! it is gone (#735): `@srelens/core`'s `listenForHostNotices` shows each as a
//! toast. A helm operation whose window closed or reloaded while it ran is
//! reported this way.

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

/// The event a notice is broadcast on.
const HOST_NOTICE: &str = "host-notice";

/// How a notice is shown: a failure, or plain information.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Level {
    Info,
    Error,
}

#[derive(Clone, Serialize)]
struct Notice<'a> {
    level: Level,
    title: &'a str,
    detail: &'a str,
}

/// Broadcast a notice to every open window. With none open there is no one
/// to tell, and the caller's own log line is the record.
pub fn notify<R: Runtime>(app: &AppHandle<R>, level: Level, title: &str, detail: &str) {
    let _ = app.emit(
        HOST_NOTICE,
        Notice {
            level,
            title,
            detail,
        },
    );
}
