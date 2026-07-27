//! WebSocket connection registry. Tracks live connections per user and the
//! channels each is subscribed to, and routes emitted events to the right
//! connections. A `WsSink` is the `EventSink` a per-user stream manager emits
//! into; it only ever reaches connections owned by that user.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use serde_json::Value;
use srelens_streams::EventSink;
use tokio::sync::mpsc;

/// Bound on each connection's outgoing frame queue. A connection whose reader
/// can't keep up (or has gone away without the read-loop noticing yet) is
/// dropped rather than let its backlog grow without bound.
pub const WS_CONN_QUEUE: usize = 256;

struct Conn {
    user_id: i64,
    tx: mpsc::Sender<String>,
    subs: Mutex<HashSet<String>>,
}

/// Per-server registry of live WebSocket connections.
pub struct WsHub {
    next_id: AtomicU64,
    conns: Mutex<HashMap<u64, Arc<Conn>>>,
}

impl Default for WsHub {
    fn default() -> Self {
        Self::new()
    }
}

/// A server→client data frame: `{"channel": ..., "payload": ...}`.
pub fn data_frame(channel: &str, payload: Value) -> String {
    serde_json::json!({ "channel": channel, "payload": payload }).to_string()
}

/// A subscription acknowledgement: `{"op":"subbed","channel":...}`.
pub fn ack_frame(channel: &str) -> String {
    serde_json::json!({ "op": "subbed", "channel": channel }).to_string()
}

impl WsHub {
    pub fn new() -> Self {
        Self {
            next_id: AtomicU64::new(1),
            conns: Mutex::new(HashMap::new()),
        }
    }

    /// Register a new connection for `user_id`; returns its id and the
    /// bounded receiver the socket write-loop drains to the client.
    pub fn register(&self, user_id: i64) -> (u64, mpsc::Receiver<String>) {
        let (tx, rx) = mpsc::channel(WS_CONN_QUEUE);
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.conns.lock().unwrap().insert(
            id,
            Arc::new(Conn {
                user_id,
                tx,
                subs: Mutex::new(HashSet::new()),
            }),
        );
        (id, rx)
    }

    pub fn unregister(&self, conn_id: u64) {
        self.conns.lock().unwrap().remove(&conn_id);
    }

    /// Whether `conn_id` is still a live, registered connection.
    pub fn has_connection(&self, conn_id: u64) -> bool {
        self.conns.lock().unwrap().contains_key(&conn_id)
    }

    /// How many live connections `user_id` currently has. Used to decide
    /// whether to tear down their stream tasks after a WS disconnect.
    pub fn user_connection_count(&self, user_id: i64) -> usize {
        self.conns
            .lock()
            .unwrap()
            .values()
            .filter(|c| c.user_id == user_id)
            .count()
    }

    pub fn subscribe(&self, conn_id: u64, channel: &str) {
        if let Some(conn) = self.conns.lock().unwrap().get(&conn_id) {
            conn.subs.lock().unwrap().insert(channel.to_string());
        }
    }

    pub fn unsubscribe(&self, conn_id: u64, channel: &str) {
        if let Some(conn) = self.conns.lock().unwrap().get(&conn_id) {
            conn.subs.lock().unwrap().remove(channel);
        }
    }

    /// Send a raw frame to one specific connection (used for acks). If the
    /// connection's queue is full or its receiver is gone, the connection is
    /// closed (removed) so its write side ends instead of stalling forever.
    pub fn deliver_direct(&self, conn_id: u64, frame: String) {
        // Clone the conn out from under the lock first: `try_send` never
        // blocks, but taking the lock again below (to remove on overflow)
        // would deadlock a non-reentrant `std::sync::Mutex` if it were still
        // held here.
        let conn = self.conns.lock().unwrap().get(&conn_id).cloned();
        let Some(conn) = conn else { return };
        if conn.tx.try_send(frame).is_err() {
            self.conns.lock().unwrap().remove(&conn_id);
        }
    }

    /// Send `frame` to every connection of `user_id` subscribed to `channel`.
    /// A connection whose queue is full (or whose receiver is gone) is closed
    /// (removed) so its write side ends instead of stalling forever.
    pub fn deliver(&self, user_id: i64, channel: &str, frame: String) {
        let targets: Vec<(u64, Arc<Conn>)> = self
            .conns
            .lock()
            .unwrap()
            .iter()
            .filter(|(_, c)| c.user_id == user_id && c.subs.lock().unwrap().contains(channel))
            .map(|(id, c)| (*id, c.clone()))
            .collect();

        // Collect overflowed/closed connection ids while iterating, then
        // remove them after the loop — the outer lock is already released by
        // the time we send, so re-locking here can't deadlock.
        let mut overflowed = Vec::new();
        for (id, conn) in targets {
            if conn.tx.try_send(frame.clone()).is_err() {
                overflowed.push(id);
            }
        }
        if !overflowed.is_empty() {
            let mut conns = self.conns.lock().unwrap();
            for id in overflowed {
                conns.remove(&id);
            }
        }
    }
}

/// The `EventSink` a per-user stream manager emits into. Scoped to one user
/// so a manager can never deliver to another user's sockets.
pub struct WsSink {
    pub hub: Arc<WsHub>,
    pub user_id: i64,
}

impl EventSink for WsSink {
    fn emit(&self, channel: &str, payload: Value) {
        self.hub
            .deliver(self.user_id, channel, data_frame(channel, payload));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recv_now(rx: &mut mpsc::Receiver<String>) -> Option<Value> {
        rx.try_recv()
            .ok()
            .map(|s| serde_json::from_str(&s).unwrap())
    }

    #[test]
    fn sink_delivers_only_to_subscribed_connections_of_the_same_user() {
        let hub = Arc::new(WsHub::new());
        let (a, mut rx_a) = hub.register(1);
        let (_b, mut rx_b) = hub.register(2);

        hub.subscribe(a, "watch:pods:1");
        let sink1 = WsSink {
            hub: hub.clone(),
            user_id: 1,
        };
        sink1.emit("watch:pods:1", serde_json::json!([{ "name": "p" }]));

        // User 1's subscribed connection gets the data frame.
        assert_eq!(
            recv_now(&mut rx_a),
            Some(serde_json::json!({
                "channel": "watch:pods:1",
                "payload": [{ "name": "p" }]
            }))
        );
        // User 2's connection gets nothing (isolation).
        assert_eq!(recv_now(&mut rx_b), None);
    }

    #[test]
    fn unsubscribe_and_unregister_stop_delivery() {
        let hub = Arc::new(WsHub::new());
        let (a, mut rx_a) = hub.register(1);
        hub.subscribe(a, "ch");
        hub.unsubscribe(a, "ch");
        let sink = WsSink {
            hub: hub.clone(),
            user_id: 1,
        };
        sink.emit("ch", serde_json::json!(1));
        assert_eq!(recv_now(&mut rx_a), None);

        hub.subscribe(a, "ch");
        hub.unregister(a);
        sink.emit("ch", serde_json::json!(2));
        assert_eq!(recv_now(&mut rx_a), None);
    }

    #[test]
    fn a_users_second_connection_also_receives() {
        let hub = Arc::new(WsHub::new());
        let (a1, mut rx1) = hub.register(7);
        let (a2, mut rx2) = hub.register(7);
        hub.subscribe(a1, "ch");
        hub.subscribe(a2, "ch");
        WsSink {
            hub: hub.clone(),
            user_id: 7,
        }
        .emit("ch", serde_json::json!("x"));
        assert!(recv_now(&mut rx1).is_some());
        assert!(recv_now(&mut rx2).is_some());
    }

    #[test]
    fn user_connection_count_tracks_registrations_and_unregistrations() {
        let hub = WsHub::new();
        let (a1, _rx1) = hub.register(1);
        let (a2, _rx2) = hub.register(1);
        let (_b1, _rx3) = hub.register(2);

        assert_eq!(hub.user_connection_count(1), 2);
        assert_eq!(hub.user_connection_count(2), 1);

        hub.unregister(a1);
        assert_eq!(hub.user_connection_count(1), 1);

        hub.unregister(a2);
        assert_eq!(hub.user_connection_count(1), 0);
    }

    #[test]
    fn overflow_closes_the_connection() {
        let hub = Arc::new(WsHub::new());
        let (a, _rx) = hub.register(1); // receiver kept but never drained
        hub.subscribe(a, "ch");
        let sink = WsSink {
            hub: hub.clone(),
            user_id: 1,
        };
        for i in 0..(super::WS_CONN_QUEUE + 5) {
            sink.emit("ch", serde_json::json!(i));
        }
        // The connection was dropped after the queue filled.
        assert!(!hub.has_connection(a));
    }
}
