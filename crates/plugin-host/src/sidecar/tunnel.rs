//! Pod tunnels (#1091): TCP connections to pods, carried as `tunnel/*`
//! frames on the sidecar's pipe.
//!
//! Each open tunnel has one pump task. It reads the pod only while srelens
//! has credit to send, and queues each chunk on the tunnel lane, the lowest of
//! the writer's three, waiting when it is full. The reader loop never waits:
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
    TUNNEL_WINDOW_BYTES,
};

/// Tunnel frames that may wait for the writer. A pump with a full lane stops
/// reading its pod, so a sidecar that stops reading its stdin holds no more
/// than this per session, whatever the pods send.
pub const TUNNEL_LANE: usize = 32;

const WINDOW: usize = TUNNEL_WINDOW_BYTES as usize;

/// The longest base64 text of one chunk, checked before decoding it.
const MAX_CHUNK_TEXT: usize = MAX_TUNNEL_CHUNK_BYTES.div_ceil(3) * 4;

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
    wake: Arc<Notify>,
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
    let wake = Arc::new(Notify::new());
    let (close, closed) = oneshot::channel();
    let pump = tokio::spawn(pump(
        connection.clone(),
        lane,
        id,
        tunnel,
        closed,
        wake.clone(),
    ));
    OpenTunnel {
        inbound: Vec::new(),
        held: 0,
        credit: WINDOW,
        wake,
        close: Some(close),
        pump: pump.abort_handle(),
        _slot: slot,
    }
}

impl OpenTunnel {
    /// A `tunnel/data` frame. `Some(why)` when it breaks the tunnel's rules.
    fn received(&mut self, params: Value) -> Option<String> {
        let bytes = match serde_json::from_value::<TunnelDataParams>(params) {
            Ok(frame) if frame.data.len() > MAX_CHUNK_TEXT => None,
            Ok(frame) => match BASE64.decode(frame.data) {
                Ok(bytes) => Some(bytes),
                Err(_) => return Some("sent tunnel data that is not base64".into()),
            },
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
            self.wake.notify_one();
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
        self.wake.notify_one();
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
            .filter(|open| open.close.is_some())
        else {
            return Ok(());
        };
        let broken = match name {
            method::TUNNEL_DATA => open.received(params),
            method::TUNNEL_CREDIT => open.credited(params),
            // The sidecar closed it: removing it drops the pod's connection.
            _ => {
                state.tunnels.remove(&id);
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
    wake: Arc<Notify>,
) {
    let Tunnel {
        mut io, mut ended, ..
    } = tunnel;
    let (message, ours) = tokio::select! {
        biased;
        why = &mut closed => match why {
            Ok(why) => (Some(why), false),
            // Removed from the table; this task is aborted.
            Err(_) => return,
        },
        why = &mut ended => (Some(why), true),
        why = flow(&connection, &lane, id, &mut io, &wake) => (why, true),
    };
    if ours {
        match connection.state().tunnels.get_mut(&id) {
            Some(open) => open.close = None,
            None => return,
        }
    }
    drop(io);
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
    connection.state().tunnels.remove(&id);
}

/// Move bytes both ways until the pod's side ends: `None` at its EOF, or why
/// it failed.
// ponytail: half-duplex per tunnel (a pending write to the pod or wait for a
// lane place holds the other direction), split the io into one task per
// direction if a protocol needs both at once.
async fn flow(
    connection: &Connection,
    lane: &mpsc::Sender<String>,
    id: u64,
    io: &mut Box<dyn TunnelIo>,
    wake: &Notify,
) -> Option<String> {
    let mut buf = vec![0; MAX_TUNNEL_CHUNK_BYTES];
    loop {
        let (inbound, room) = {
            let mut state = connection.state();
            let open = state.tunnels.get_mut(&id)?;
            (
                std::mem::take(&mut open.inbound),
                open.credit.min(MAX_TUNNEL_CHUNK_BYTES),
            )
        };
        if !inbound.is_empty() {
            let written = async {
                io.write_all(&inbound).await?;
                io.flush().await
            };
            if let Err(e) = written.await {
                return Some(format!("srelens could not write to the pod: {e}"));
            }
            let place = lane.reserve().await.ok()?;
            // Returned before the credit is queued, so the sidecar can never
            // be told of room srelens has not made.
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
            continue;
        }
        tokio::select! {
            _ = wake.notified() => {}
            read = io.read(&mut buf[..room]), if room > 0 => {
                let n = match read {
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
    }
}
