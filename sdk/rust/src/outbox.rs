//! Everything the sidecar writes goes through one writer task, so lines
//! never interleave, and each is checked against the protocol's size limit
//! before it is queued: a line over it would break the framing, and srelens
//! stops a sidecar that does that. Lifecycle answers have a lane of their
//! own that the writer takes first, so `health` is never answered behind a
//! queue of stream frames.

use serde::Serialize;
use srelens_sidecar_protocol::MAX_MESSAGE_BYTES;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::{mpsc, oneshot};

/// Lines waiting for the writer. The writer drains as fast as srelens reads.
const QUEUE: usize = 64;

/// Lifecycle answers waiting for the writer, which takes them ahead of the
/// [`QUEUE`].
const LIFECYCLE: usize = 8;

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
pub(crate) struct Outbox {
    general: mpsc::Sender<Line>,
    lifecycle: mpsc::Sender<String>,
}

impl Outbox {
    pub(crate) fn new() -> (Outbox, Writer) {
        let (general, queued) = mpsc::channel(QUEUE);
        let (lifecycle, lifecycle_lines) = mpsc::channel(LIFECYCLE);
        let outbox = Outbox { general, lifecycle };
        let writer = Writer {
            general: queued,
            lifecycle: lifecycle_lines,
        };
        (outbox, writer)
    }

    /// Queue one message as one line, behind every line queued before it.
    pub(crate) async fn send(&self, message: &impl Serialize) -> Result<(), Unsent> {
        let line = line(message)?;
        self.general
            .send(Line::Text(line))
            .await
            .map_err(|_| Unsent::Closed)
    }

    /// Wait for room for one line on the general lane, queuing nothing yet.
    /// Waiting for room is the only part of [`Outbox::send`] that waits:
    /// filling it does not, so a caller can decide whether to queue its line
    /// at all at the last moment, under a lock of its own.
    pub(crate) async fn room(&self) -> Result<Room<'_>, Unsent> {
        self.general
            .reserve()
            .await
            .map(Room)
            .map_err(|_| Unsent::Closed)
    }

    /// Queue the answer to `activate`, `health` or `deactivate`, which the
    /// writer takes ahead of everything [`Outbox::send`] queued.
    pub(crate) async fn send_lifecycle(&self, message: &impl Serialize) -> Result<(), Unsent> {
        let line = line(message)?;
        self.lifecycle.send(line).await.map_err(|_| Unsent::Closed)
    }

    /// Tell the writer there is nothing more to write, and wait for it to
    /// flush, shut down and stop; a line queued after this call is never
    /// written. If the writer is already gone (it failed, or was already
    /// closed), there is nothing to wait for.
    pub(crate) async fn close(&self) {
        let (done, closed) = oneshot::channel();
        if self.general.send(Line::Close(done)).await.is_ok() {
            let _ = closed.await;
        }
    }
}

/// Room for one line on the general lane ([`Outbox::room`]). Dropped
/// unfilled, it is given back.
pub(crate) struct Room<'a>(mpsc::Permit<'a, Line>);

impl Room<'_> {
    /// Queue `message` as one line here, at once, behind every line queued
    /// before. Over the limit, it is not queued, and the room is given back.
    pub(crate) fn send(self, message: &impl Serialize) -> Result<(), Unsent> {
        let line = line(message)?;
        self.0.send(Line::Text(line));
        Ok(())
    }
}

/// `message` as one line, if it is within the protocol's limit.
fn line(message: &impl Serialize) -> Result<String, Unsent> {
    let line = serde_json::to_string(message).expect("protocol messages are plain JSON");
    if line.len() > MAX_MESSAGE_BYTES {
        return Err(Unsent::TooLarge(line.len()));
    }
    Ok(line)
}

pub(crate) struct Writer {
    general: mpsc::Receiver<Line>,
    lifecycle: mpsc::Receiver<String>,
}

impl Writer {
    pub(crate) async fn run<W: AsyncWrite + Unpin>(mut self, mut out: W) -> std::io::Result<()> {
        loop {
            let line = tokio::select! {
                // Polled in order: a lifecycle answer goes first, whatever
                // the general lane holds.
                biased;
                Some(text) = self.lifecycle.recv() => Line::Text(text),
                line = self.general.recv() => match line {
                    Some(line) => line,
                    None => break,
                },
            };
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;

    // A writer that took lines in any order but the lanes' would pass this
    // half the time, so it runs on many outboxes.
    #[tokio::test]
    async fn a_lifecycle_line_is_written_ahead_of_a_full_queue() {
        for _ in 0..20 {
            let (outbox, writer) = Outbox::new();
            for n in 0..QUEUE {
                outbox.send(&json!({"frame": n})).await.unwrap();
            }
            tokio::time::timeout(
                Duration::from_secs(1),
                outbox.send_lifecycle(&json!({"health": "ok"})),
            )
            .await
            .expect("a lifecycle line is queued even when the queue is full")
            .unwrap();
            drop(outbox);
            let mut written = Vec::new();
            writer.run(&mut written).await.unwrap();
            let written = String::from_utf8(written).unwrap();
            let lines: Vec<&str> = written.lines().collect();
            assert_eq!(lines.len(), QUEUE + 1);
            assert_eq!(lines[0], r#"{"health":"ok"}"#, "the first line written");
        }
    }
}
