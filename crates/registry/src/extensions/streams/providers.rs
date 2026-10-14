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
use srelens_kube::logs::Line;
use srelens_plugin_host::ProviderKind;
use srelens_streams::app::{StreamEmitter, StreamOwner, StreamWindow};
use srelens_streams::EventSink;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

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
            at_cursor: HashMap::new(),
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
    /// The time of the newest line sent, in nanoseconds; `None` until the history
    /// is answered. The next query starts there, inclusive, since lines may share a
    /// timestamp.
    cursor: Option<i128>,
    /// How many of each line were sent at `cursor`, which the next query answers
    /// again: counted, so two identical lines at one instant are both sent, once.
    at_cursor: HashMap<(String, String), usize>,
}

impl Follow {
    fn status(&self, status: &str, message: Option<&str>) -> Value {
        let mut frame = json!({"event": "status", "source": self.source, "status": status});
        if let Some(message) = message {
            frame["message"] = json!(message);
        }
        frame
    }

    /// Sends the entries (oldest first) not sent yet, in frames, moves the cursor,
    /// and says how many it sent.
    fn deliver(&mut self, tx: &StreamEmitter, entries: Vec<Entry>) -> Result<usize, Ended> {
        let mut pending = Pending::default();
        // How many of each line this answer holds at the cursor so far.
        let mut seen: HashMap<(String, String), usize> = HashMap::new();
        let mut sent = 0;
        for entry in entries {
            let key = (entry.source.clone(), entry.line.clone());
            match self.cursor {
                Some(cursor) if entry.nanos < cursor => continue,
                Some(cursor) if entry.nanos == cursor => {
                    let count = seen.entry(key.clone()).or_default();
                    *count += 1;
                    if *count <= self.at_cursor.get(&key).copied().unwrap_or(0) {
                        continue;
                    }
                    *self.at_cursor.entry(key).or_default() += 1;
                }
                _ => {
                    self.cursor = Some(entry.nanos);
                    self.at_cursor.clear();
                    seen.clear();
                    seen.insert(key.clone(), 1);
                    self.at_cursor.insert(key, 1);
                }
            }
            pending.push(line_frame(
                &entry.source,
                Line::whole(entry.text(self.timestamps)),
            ));
            sent += 1;
        }
        while let Some(frame) = pending.frame() {
            send(tx, frame)?;
        }
        Ok(sent)
    }

    async fn run(mut self, tx: StreamEmitter) -> Result<(), String> {
        // Whether the last query was answered, which is what `live` says.
        let mut live = false;
        let mut wait = Duration::ZERO;
        loop {
            tokio::time::sleep(wait).await;
            if tx.is_ended() {
                return Ok(());
            }
            wait = self.timing.poll;
            let bound = self.ask.bind(Some(ProviderKind::Logs), None).await?;
            // Short of now: the newest instants are still filling in.
            let end = now_nanos() - self.timing.lag.as_nanos() as i128;
            let history = self.cursor.is_none();
            let asked = match self.cursor {
                // No history asked for: follow from here, and say `live` only once a
                // query has been answered.
                None if self.tail == 0 => {
                    self.cursor = Some(end);
                    continue;
                }
                // The newest lines of the window, asked again until one answers.
                None => {
                    self.ask
                        .logs(&bound, end - self.since, end, self.tail, true)
                        .await
                }
                Some(start) if start >= end => continue,
                Some(start) => {
                    self.ask
                        .logs(&bound, start, end, MAX_LOG_LINES, false)
                        .await
                }
            };
            match asked {
                Ok((entries, full)) => {
                    let Ok(sent) = self.deliver(&tx, entries) else {
                        return Ok(());
                    };
                    if history {
                        self.cursor.get_or_insert(end);
                    } else if full {
                        if sent == 0 {
                            // A whole page of lines already sent, all at the cursor:
                            // more than a page share that instant. Move past it, or
                            // every query would answer the same page.
                            self.cursor = self.cursor.map(|cursor| cursor + 1);
                            self.at_cursor.clear();
                        }
                        wait = self.timing.full_page;
                    }
                    if !live {
                        live = true;
                        if send(&tx, self.status("live", None)).is_err() {
                            return Ok(());
                        }
                    }
                }
                Err(RequestError::Unanswered(why)) => {
                    live = false;
                    if send(&tx, self.status("reconnecting", Some(&why))).is_err() {
                        return Ok(());
                    }
                }
                // Even the fewest lines were too large, or the request may not go.
                Err(RequestError::Refused(why) | RequestError::TooLarge(why)) => return Err(why),
            }
        }
    }
}
