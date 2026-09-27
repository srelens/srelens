//! The `logProvider` source (#569): one of an app's log providers, followed
//! for the resource a view shows, in the frames a pod's `logs` source sends.
//!
//! A log backend reached through `network.http` has no push: the host sends a
//! range query for the history, then asks again every
//! [`ProviderTiming::poll`] for what is newer than the last line it sent, for
//! as long as the view is open. The interval is the host's; an app cannot set
//! it. Each query goes through [`Ask::bind`] again, so it is authorized as a
//! read is, with the settings saved at that moment. A query nothing answered
//! (a lost connection, a timeout, a 5xx) sends `reconnecting` and is asked
//! again at the next poll; one that is refused (the allowlist, a 4xx, an
//! answer too large or not Loki's) ends the stream with the reason.
use super::super::network::RequestError;
use super::super::providers::{
    now_nanos, Ask, Entry, ProviderTiming, Subject, DEFAULT_HISTORY, MAX_HISTORY, MAX_LOG_LINES,
    MAX_RANGE,
};
use super::pods::{line_frame, Pending};
use super::{ExtensionStreams, OpenStreamIn, OpenStreamOut, StreamSourceIn};
use serde_json::{json, Value};
use srelens_plugin_host::ProviderKind;
use srelens_streams::app::{StreamEmitter, StreamOwner, StreamWindow};
use srelens_streams::EventSink;
use std::sync::Arc;

/// The history a follow starts with when the view names no `sinceSeconds`.
const DEFAULT_SINCE: i64 = 3600;
const NANOS: i128 = 1_000_000_000;

impl ExtensionStreams {
    /// Open a `logProvider` source: bound and authorized now, so a view hears
    /// a refusal from the open, then followed until the stream ends.
    pub(super) async fn open_log_provider(
        &self,
        sink: Arc<dyn EventSink>,
        window: Option<StreamWindow>,
        input: OpenStreamIn,
    ) -> Result<OpenStreamOut, String> {
        let StreamSourceIn::LogProvider {
            provider,
            resource_kind,
            name,
            tail_lines,
            since_seconds,
            timestamps,
        } = &input.source
        else {
            unreachable!("open_log_provider is called for a logProvider source");
        };
        let tail = tail_lines.unwrap_or(DEFAULT_HISTORY);
        if !(0..=MAX_HISTORY).contains(&tail) {
            return Err(format!(
                "A log follow starts with 0–{MAX_HISTORY} lines of history, not {tail}"
            ));
        }
        let since = since_seconds.unwrap_or(DEFAULT_SINCE);
        if !(1..=MAX_RANGE as i64).contains(&since) {
            return Err(format!(
                "A log follow's history reaches back 1–{MAX_RANGE} seconds, not {since}"
            ));
        }
        let ask = Ask {
            path: self.path.clone(),
            core: self.core.clone(),
            cache: self.cache.clone(),
            secrets: self.secrets.clone(),
            id: input.id.clone(),
            revision: input.revision,
            context: input.context.clone(),
            provider: provider.clone(),
            subject: Subject {
                namespace: input.namespace.clone(),
                resource_kind: resource_kind.clone(),
                name: name.clone(),
            },
        };
        ask.bind(Some(ProviderKind::Logs), None).await?;
        let follow = Follow {
            source: provider.clone(),
            ask,
            tail: tail as usize,
            since: i128::from(since) * NANOS,
            timestamps: *timestamps,
            timing: *self.provider_timing.lock().unwrap(),
            cursor: None,
            at_cursor: Vec::new(),
        };
        let owner = StreamOwner {
            app: input.id.clone(),
            revision: input.revision,
            view: input.view.clone(),
            window,
        };
        let stream = self
            .streams
            .open(
                owner,
                "logProvider",
                sink,
                input.channel.clone(),
                move |tx| follow.run(tx),
            )
            .map_err(|e| e.to_string())?;
        Ok(OpenStreamOut {
            stream,
            channel: input.channel,
        })
    }
}

/// The stream ended; there is nothing left to send.
struct Ended;

fn send(tx: &StreamEmitter, frame: Value) -> Result<(), Ended> {
    tx.data(frame).map_err(|_| Ended)
}

/// One `logProvider` stream's source.
struct Follow {
    ask: Ask,
    /// The provider's id, on every status frame.
    source: String,
    tail: usize,
    /// How far back the history reaches, in nanoseconds.
    since: i128,
    timestamps: bool,
    timing: ProviderTiming,
    /// The time of the newest line sent, in nanoseconds: the next query starts
    /// there, inclusive, since lines may share a timestamp.
    cursor: Option<i128>,
    /// The lines sent at `cursor`, which the next query answers again.
    at_cursor: Vec<(String, String)>,
}

impl Follow {
    fn status(&self, status: &str, message: Option<&str>) -> Value {
        let mut frame = json!({"event": "status", "source": self.source, "status": status});
        if let Some(message) = message {
            frame["message"] = json!(message);
        }
        frame
    }

    /// Sends the entries newer than the cursor, in frames, and moves it.
    fn deliver(&mut self, tx: &StreamEmitter, entries: Vec<Entry>) -> Result<(), Ended> {
        let mut pending = Pending::default();
        for entry in entries {
            let key = (entry.source.clone(), entry.line.clone());
            match self.cursor {
                Some(cursor) if entry.nanos < cursor => continue,
                Some(cursor) if entry.nanos == cursor && self.at_cursor.contains(&key) => continue,
                Some(cursor) if entry.nanos == cursor => self.at_cursor.push(key),
                _ => {
                    self.cursor = Some(entry.nanos);
                    self.at_cursor = vec![key];
                }
            }
            pending.push(line_frame(&entry.source, entry.text(self.timestamps)));
        }
        while let Some(frame) = pending.frame() {
            send(tx, frame)?;
        }
        Ok(())
    }

    async fn run(mut self, tx: StreamEmitter) -> Result<(), String> {
        // Whether the last query was answered, which is what `live` says.
        let mut live = false;
        let end = now_nanos();
        let bound = self.ask.bind(Some(ProviderKind::Logs), None).await?;
        let history = if self.tail == 0 {
            Ok((Vec::new(), false))
        } else {
            self.ask
                .logs(&bound, end - self.since, end, self.tail, true)
                .await
        };
        match history {
            Ok((entries, _)) => {
                if self.deliver(&tx, entries).is_err() {
                    return Ok(());
                }
                self.cursor.get_or_insert(end);
                live = true;
                if send(&tx, self.status("live", None)).is_err() {
                    return Ok(());
                }
            }
            Err(RequestError::Refused(why)) => return Err(why),
            Err(RequestError::Unanswered(why)) => {
                // Nothing was sent: the next query starts where the history would have.
                self.cursor = Some(end - self.since);
                if send(&tx, self.status("reconnecting", Some(&why))).is_err() {
                    return Ok(());
                }
            }
        }
        let mut wait = self.timing.poll;
        loop {
            tokio::time::sleep(wait).await;
            if tx.is_ended() {
                return Ok(());
            }
            let bound = self.ask.bind(Some(ProviderKind::Logs), None).await?;
            let start = self.cursor.unwrap_or(end);
            match self
                .ask
                .logs(&bound, start, now_nanos(), MAX_LOG_LINES, false)
                .await
            {
                Ok((entries, full)) => {
                    if !live {
                        live = true;
                        if send(&tx, self.status("live", None)).is_err() {
                            return Ok(());
                        }
                    }
                    if self.deliver(&tx, entries).is_err() {
                        return Ok(());
                    }
                    wait = if full {
                        self.timing.full_page
                    } else {
                        self.timing.poll
                    };
                }
                Err(RequestError::Refused(why)) => return Err(why),
                Err(RequestError::Unanswered(why)) => {
                    live = false;
                    wait = self.timing.poll;
                    if send(&tx, self.status("reconnecting", Some(&why))).is_err() {
                        return Ok(());
                    }
                }
            }
        }
    }
}
