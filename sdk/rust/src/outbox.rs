//! Everything the sidecar writes goes through one queue to one writer task,
//! so lines never interleave, and each is checked against the protocol's
//! size limit before it is queued: a line over it would break the framing,
//! and srelens stops a sidecar that does that.

use serde::Serialize;
use srelens_sidecar_protocol::MAX_MESSAGE_BYTES;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

/// Lines waiting for the writer. The writer drains as fast as srelens reads.
const QUEUE: usize = 64;

/// Why a message was not queued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Unsent {
    /// It serialized to this many bytes, over [`MAX_MESSAGE_BYTES`].
    TooLarge(usize),
    /// The session has ended.
    Closed,
}

enum Line {
    Text(String),
    /// There is nothing more to write: flush, shut the writer down (so the
    /// reader on the other end sees the sidecar's output end), then ack.
    Close(oneshot::Sender<()>),
}

#[derive(Clone)]
pub(crate) struct Outbox(mpsc::Sender<Line>);

impl Outbox {
    pub(crate) fn new() -> (Outbox, Writer) {
        let (sender, lines) = mpsc::channel(QUEUE);
        (Outbox(sender), Writer(lines))
    }

    /// Queue one message as one line.
    pub(crate) async fn send(&self, message: &impl Serialize) -> Result<(), Unsent> {
        let line = serde_json::to_string(message).expect("protocol messages are plain JSON");
        if line.len() > MAX_MESSAGE_BYTES {
            return Err(Unsent::TooLarge(line.len()));
        }
        self.0
            .send(Line::Text(line))
            .await
            .map_err(|_| Unsent::Closed)
    }

    /// Tell the writer there is nothing more to write, and wait for it to
    /// flush, shut down and stop; a line queued after this call is never
    /// written. If the writer is already gone (it failed, or was already
    /// closed), there is nothing to wait for.
    pub(crate) async fn close(&self) {
        let (done, closed) = oneshot::channel();
        if self.0.send(Line::Close(done)).await.is_ok() {
            let _ = closed.await;
        }
    }
}

pub(crate) struct Writer(mpsc::Receiver<Line>);

impl Writer {
    pub(crate) async fn run<W: AsyncWrite + Unpin>(mut self, mut out: W) -> std::io::Result<()> {
        while let Some(line) = self.0.recv().await {
            match line {
                Line::Text(mut text) => {
                    text.push('\n');
                    out.write_all(text.as_bytes()).await?;
                    out.flush().await?;
                }
                Line::Close(done) => {
                    out.flush().await?;
                    out.shutdown().await?;
                    let _ = done.send(());
                    return Ok(());
                }
            }
        }
        out.shutdown().await
    }
}

/// "4 MiB", for sentences.
pub(crate) fn limit_text() -> String {
    format!("{} MiB", MAX_MESSAGE_BYTES / (1024 * 1024))
}
