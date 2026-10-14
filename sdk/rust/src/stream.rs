use serde::Serialize;
use srelens_sidecar_protocol::{method, Notification, StreamDataParams};
use std::sync::Arc;
use tokio::sync::RwLock;
use tokio_util::sync::CancellationToken;

use crate::outbox::{Outbox, Unsent};
use crate::StreamClosed;

/// Where a stream handler sends its frames.
#[derive(Clone)]
pub struct Frames {
    stream: u64,
    outbox: Outbox,
    cancel: CancellationToken,
    /// Whether the handler is still running. Each frame holds it for reading
    /// while it is queued, and [`Frames::finish`] takes it for writing, so a
    /// clone kept past the handler can never queue a frame after the
    /// stream's terminal one.
    open: Arc<RwLock<bool>>,
}

impl Frames {
    pub(crate) fn new(stream: u64, outbox: Outbox, cancel: CancellationToken) -> Frames {
        Frames {
            stream,
            outbox,
            cancel,
            open: Arc::new(RwLock::new(true)),
        }
    }

    /// The id srelens gave the stream.
    pub fn id(&self) -> u64 {
        self.stream
    }

    /// Send one frame. Fails once srelens cancelled the stream, its handler
    /// returned, or the session ended, or when the frame is over the message
    /// limit.
    pub async fn send<T: Serialize>(&self, data: &T) -> Result<(), StreamClosed> {
        if self.cancel.is_cancelled() {
            return Err(StreamClosed::Cancelled);
        }
        let open = self.open.read().await;
        if !*open {
            return Err(StreamClosed::Finished);
        }
        let data = serde_json::to_value(data).map_err(|e| StreamClosed::Invalid(e.to_string()))?;
        let frame = Notification::new(
            method::STREAM_DATA,
            serde_json::to_value(StreamDataParams {
                stream: self.stream,
                data,
            })
            .expect("plain JSON"),
        );
        match self.outbox.send(&frame).await {
            Ok(()) => Ok(()),
            Err(Unsent::TooLarge(bytes)) => Err(StreamClosed::TooLarge(bytes)),
            Err(Unsent::Closed) => Err(StreamClosed::Ended),
        }
    }

    /// The handler returned: refuse every later frame, from any clone. Waits
    /// for a frame already being queued, so the terminal frame queued after
    /// this is the stream's last line.
    pub(crate) async fn finish(&self) {
        *self.open.write().await = false;
    }
}
