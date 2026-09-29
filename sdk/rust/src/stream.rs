use serde::Serialize;
use srelens_sidecar_protocol::{method, Notification, StreamDataParams};
use tokio_util::sync::CancellationToken;

use crate::outbox::{Outbox, Unsent};
use crate::StreamClosed;

/// Where a stream handler sends its frames.
#[derive(Clone)]
pub struct Frames {
    stream: u64,
    outbox: Outbox,
    cancel: CancellationToken,
}

impl Frames {
    pub(crate) fn new(stream: u64, outbox: Outbox, cancel: CancellationToken) -> Frames {
        Frames {
            stream,
            outbox,
            cancel,
        }
    }

    /// The id srelens gave the stream.
    pub fn id(&self) -> u64 {
        self.stream
    }

    /// Send one frame. Fails once srelens cancelled the stream or the session
    /// ended, or when the frame is over the message limit.
    pub async fn send<T: Serialize>(&self, data: &T) -> Result<(), StreamClosed> {
        if self.cancel.is_cancelled() {
            return Err(StreamClosed::Cancelled);
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
}
