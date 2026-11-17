//! Pod tunnels (#1091): TCP connections to pods, carried as `tunnel/*`
//! frames on the sidecar's pipe.
//!
//! Each open tunnel has one pump task, moving both directions at once so a
//! pod that writes before it reads cannot deadlock it. It reads the pod only
//! while srelens has credit to send, and queues each chunk on the tunnel lane,
//! the lowest of the writer's three, waiting when it is full. The reader loop
//! never waits:
//! it adds the sidecar's bytes to the tunnel's buffer, which the sidecar's
//! window bounds, and closes the tunnel if it overruns. So neither the pod nor
//! the sidecar can make srelens hold more than one window per direction per
//! tunnel, plus [`TUNNEL_LANE`] frames per session.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde_json::Value;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot, Notify, OwnedSemaphorePermit};
use tokio::task::AbortHandle;

use super::connection::Connection;
use super::protocol::{self, method, Violation};
use srelens_sidecar_protocol::{
    TunnelCloseParams, TunnelCreditParams, TunnelDataParams, MAX_TUNNEL_CHUNK_BYTES,
    MAX_TUNNEL_CHUNK_TEXT, TUNNEL_WINDOW_BYTES,
};

/// Tunnel frames that may wait for the writer. A pump with a full lane stops
/// reading its pod, so a sidecar that stops reading its stdin holds no more
/// than this per session, whatever the pods send.
pub const TUNNEL_LANE: usize = 32;

const WINDOW: usize = TUNNEL_WINDOW_BYTES as usize;

/// A byte stream to a pod.
pub trait TunnelIo: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> TunnelIo for T {}

/// A connection to a pod, as [`super::Broker::connect`] opens it.
pub struct Tunnel {
    pub io: Box<dyn TunnelIo>,
    pub pod: String,
    /// Resolves with the reason when the host must end the tunnel.
    pub ended: Pin<Box<dyn Future<Output = String> + Send>>,
}

/// A tunnel in the session's table. Removing it stops its pump and drops the
/// connection to the pod.
pub(super) struct OpenTunnel {
    /// The sidecar's bytes the pump has not taken yet.
    inbound: Vec<u8>,
    /// The sidecar's bytes not yet credited back. Never more than the window.
    held: usize,
    /// Bytes srelens may still send the sidecar.
    credit: usize,
    /// Wakes the pod-to-sidecar half: more credit.
    more_credit: Arc<Notify>,
    /// Wakes the sidecar-to-pod half: more bytes, or the sidecar's close.
    more_inbound: Arc<Notify>,
    /// The sidecar closed it: what was accepted is written, then it ends.
    draining: bool,
    /// Tells the pump why srelens is closing the tunnel. Taken once it is
    /// closing: from then on the sidecar's frames for it are dropped.
    close: Option<oneshot::Sender<String>>,
    pump: AbortHandle,
    _slot: OwnedSemaphorePermit,
}

impl Drop for OpenTunnel {
    fn drop(&mut self) {
        self.pump.abort();
    }
}

/// Start the pump of tunnel `id`, whose frames go on `lane`, holding one of
/// the session's tunnel places.
pub(super) fn open(
    connection: &Connection,
    lane: mpsc::Sender<String>,
    id: u64,
    tunnel: Tunnel,
    slot: OwnedSemaphorePermit,
) -> OpenTunnel {
    let (more_credit, more_inbound) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    let (close, closed) = oneshot::channel();
    let pump = tokio::spawn(pump(
        connection.clone(),
        lane,
        id,
        tunnel,
        closed,
        (more_credit.clone(), more_inbound.clone()),
    ));
    OpenTunnel {
        inbound: Vec::new(),
        held: 0,
        credit: WINDOW,
        more_credit,
        more_inbound,
        draining: false,
        close: Some(close),
        pump: pump.abort_handle(),
        _slot: slot,
    }
}

impl OpenTunnel {
    /// A `tunnel/data` frame. `Some(why)` when it breaks the tunnel's rules.
    fn received(&mut self, params: Value) -> Option<String> {
        let too_long = params
            .get("data")
            .and_then(Value::as_str)
            .is_some_and(|text| text.len() > MAX_TUNNEL_CHUNK_TEXT);
        let bytes = match serde_json::from_value::<TunnelDataParams>(params) {
            Ok(frame) => match BASE64.decode(frame.data) {
                Ok(bytes) => Some(bytes),
                Err(_) => return Some("sent tunnel data that is not base64".into()),
            },
            // The text is too long to be a chunk; not a malformed frame.
            Err(_) if too_long => None,
            Err(_) => return Some("sent tunnel data that is not base64".into()),
        };
        let Some(bytes) = bytes.filter(|bytes| bytes.len() <= MAX_TUNNEL_CHUNK_BYTES) else {
            return Some(format!(
                "sent a chunk of more than {MAX_TUNNEL_CHUNK_BYTES} bytes"
            ));
        };
        if self.held + bytes.len() > WINDOW {
            return Some(format!(
                "sent more than its window of {WINDOW} bytes without credit"
            ));
        }
        // An empty chunk would cost a place in nothing; it is dropped.
        if !bytes.is_empty() {
            self.held += bytes.len();
            self.inbound.extend_from_slice(&bytes);
            self.more_inbound.notify_one();
        }
        None
    }

    /// A `tunnel/credit` frame. `Some(why)` when it breaks the tunnel's rules.
    fn credited(&mut self, params: Value) -> Option<String> {
        let Ok(credit) = serde_json::from_value::<TunnelCreditParams>(params) else {
            return Some(format!("sent credit that is not 1 to {WINDOW} bytes"));
        };
        let credit = credit.bytes as usize;
        if self.credit + credit > WINDOW {
            return Some(format!("granted credit past the window of {WINDOW} bytes"));
        }
        self.credit += credit;
        self.more_credit.notify_one();
        None
    }
}

impl Connection {
    /// A `tunnel/*` frame from the sidecar. Never waits. A frame for a tunnel
    /// that is not open crossed its close and is dropped; one that breaks the
    /// tunnel's rules closes that tunnel, not the session.
    pub(super) fn tunnel_frame(&self, name: &str, params: Value) -> Result<(), Violation> {
        let Some(id) = params
            .get("tunnel")
            .and_then(Value::as_u64)
            .filter(|id| *id > 0)
        else {
            return Err(Violation("sent a tunnel frame with no tunnel id".into()));
        };
        let mut state = self.state();
        let Some(open) = state
            .tunnels
            .get_mut(&id)
            .filter(|open| open.close.is_some() && !open.draining)
        else {
            return Ok(());
        };
        let broken = match name {
            method::TUNNEL_DATA => open.received(params),
            method::TUNNEL_CREDIT => open.credited(params),
            // The sidecar closed it, as TCP closes after a write: the bytes
            // already accepted reach the pod, then its connection drops.
            _ => {
                open.draining = true;
                open.more_inbound.notify_one();
                return Ok(());
            }
        };
        if let Some(why) = broken {
            if let Some(close) = open.close.take() {
                let _ = close.send(format!("srelens closed the tunnel: the extension {why}"));
            }
        }
        Ok(())
    }
}

fn frame<T: serde::Serialize>(name: &str, params: &T) -> String {
    protocol::notification(name, &protocol::params(params))
}

/// Carry tunnel `id` until it closes, then say so on the lane, after its
/// data, and leave the table.
async fn pump(
    connection: Connection,
    lane: mpsc::Sender<String>,
    id: u64,
    tunnel: Tunnel,
    mut closed: oneshot::Receiver<String>,
    (more_credit, more_inbound): (Arc<Notify>, Arc<Notify>),
) {
    let Tunnel { io, mut ended, .. } = tunnel;
    let (mut from_pod, mut to_pod) = tokio::io::split(io);
    // Both halves run at once in this task; aborting it drops both.
    let message = tokio::select! {
        biased;
        why = &mut closed => match why {
            Ok(why) => Some(why),
            // Removed from the table; this task is aborted.
            Err(_) => return,
        },
        why = &mut ended => Some(why),
        why = pod_to_sidecar(&connection, &lane, id, &mut from_pod, &more_credit) => why,
        why = sidecar_to_pod(&connection, &lane, id, &mut to_pod, &more_inbound) => why,
    };
    let draining = match connection.state().tunnels.get_mut(&id) {
        Some(open) => {
            open.close = None;
            open.draining
        }
        None => return,
    };
    drop((from_pod, to_pod));
    // The sidecar closed it itself, so it is not told.
    if !draining {
        // Fails only once the writer is gone, when the session is ending.
        if let Ok(place) = lane.reserve().await {
            place.send(frame(
                method::TUNNEL_CLOSE,
                &TunnelCloseParams {
                    tunnel: id,
                    message,
                },
            ));
        }
    }
    connection.state().tunnels.remove(&id);
}

/// The pod's bytes to the sidecar, while there is credit, until the pod's
/// EOF (`None`) or a failure (why).
async fn pod_to_sidecar(
    connection: &Connection,
    lane: &mpsc::Sender<String>,
    id: u64,
    from_pod: &mut (impl AsyncRead + Unpin),
    more_credit: &Notify,
) -> Option<String> {
    let mut buf = vec![0; MAX_TUNNEL_CHUNK_BYTES];
    loop {
        let room = {
            let mut state = connection.state();
            let open = state.tunnels.get_mut(&id)?;
            // A draining tunnel no longer reads its pod.
            if open.draining {
                0
            } else {
                open.credit.min(MAX_TUNNEL_CHUNK_BYTES)
            }
        };
        if room == 0 {
            more_credit.notified().await;
            continue;
        }
        let n = match from_pod.read(&mut buf[..room]).await {
            Ok(0) => return None,
            Ok(n) => n,
            Err(e) => return Some(format!("The connection to the pod failed: {e}")),
        };
        let place = lane.reserve().await.ok()?;
        if let Some(open) = connection.state().tunnels.get_mut(&id) {
            open.credit -= n;
        }
        place.send(frame(
            method::TUNNEL_DATA,
            &TunnelDataParams {
                tunnel: id,
                data: BASE64.encode(&buf[..n]),
            },
        ));
    }
}

/// The sidecar's bytes to the pod, each write credited back, until the
/// sidecar's close has been flushed (`None`) or a write fails (why).
async fn sidecar_to_pod(
    connection: &Connection,
    lane: &mpsc::Sender<String>,
    id: u64,
    to_pod: &mut (impl AsyncWrite + Unpin),
    more_inbound: &Notify,
) -> Option<String> {
    loop {
        let (inbound, draining) = {
            let mut state = connection.state();
            let open = state.tunnels.get_mut(&id)?;
            (std::mem::take(&mut open.inbound), open.draining)
        };
        if inbound.is_empty() {
            if draining {
                return None;
            }
            more_inbound.notified().await;
            continue;
        }
        let written = async {
            to_pod.write_all(&inbound).await?;
            to_pod.flush().await
        };
        if let Err(e) = written.await {
            return Some(format!("srelens could not write to the pod: {e}"));
        }
        if draining {
            continue;
        }
        let place = lane.reserve().await.ok()?;
        // Returned before the credit is queued, so the sidecar can never be
        // told of room srelens has not made.
        if let Some(open) = connection.state().tunnels.get_mut(&id) {
            open.held -= inbound.len();
        }
        place.send(frame(
            method::TUNNEL_CREDIT,
            &TunnelCreditParams {
                tunnel: id,
                bytes: inbound.len() as u32,
            },
        ));
    }
}
