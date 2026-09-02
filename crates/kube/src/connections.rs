//! `k8s.podConnections` — what a pod is actually connected to, read out of its
//! own `/proc/net/tcp`.
//!
//! ## Why this exists
//!
//! [`crate::topology`] has two sources of flow and neither is a connection.
//! Configuration says what a workload was BUILT to reach; a Prometheus says
//! what a mesh MEASURED. This is the third: a cluster with no mesh, no
//! Prometheus and no eBPF agent still knows exactly who its pods are talking
//! to, because the kernel keeps a table of it and every pod can read its own.
//!
//! Nothing is installed. The reads go through `pods/exec`, which is the same
//! permission the shell already uses.
//!
//! ## What it is not
//!
//! **A rate.** `/proc/net/tcp` is a snapshot of connections open at this
//! instant, not traffic over a window. Two pods holding a pooled connection
//! and never sending a byte look identical to two exchanging thousands of
//! requests a second, and a short-lived request that opened and closed between
//! two reads is invisible. So an edge from here carries a CONNECTION COUNT and
//! never a rate — anything else would be a number the kernel did not report.
//!
//! **Layer 7.** There is no status code and no latency in a socket table.
//!
//! **Free.** This is one exec per pod. That is a real cost and a real
//! intrusion — a `kubectl exec` shows up in the audit log of every pod it
//! touches — which is why it is opt-in, capped, and never the default.
//!
//! ## What a failure means
//!
//! A distroless pod has no `cat`, a locked-down one may refuse exec outright,
//! and a pod that is not running cannot be read at all. Every one of those is
//! ordinary. They are reported as `unreadable` rather than swallowed: a reader
//! looking at a graph with half the pods missing needs to know it is half, and
//! silently drawing fewer edges would say "these pods talk to nothing".

use std::collections::BTreeMap;
use std::sync::Arc;

use tokio::io::AsyncReadExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::AttachParams;
use kube::Api;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use srelens_capability::{Annotations, Capability, CapabilityError};

use crate::client_cache::ClientCache;

/// One remote endpoint a pod is connected to, and how many sockets are open to
/// it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Peer {
    pub address: String,
    pub port: u16,
    /// Open sockets to that address and port. A count, never a rate — see the
    /// module docs.
    pub count: u32,
}

/// The socket states worth reading.
///
/// `01` is ESTABLISHED and is the only one that means "talking to". A listener
/// (`0A`) has no remote to name, and a socket in `TIME_WAIT` or `CLOSE_WAIT` is
/// a conversation that has already ended — drawing those would put edges on the
/// diagram for things a pod used to talk to, which ages badly and silently.
const ESTABLISHED: &str = "01";

/// Decode one `/proc/net` address, which is hex and NOT in network order.
///
/// IPv4 is a single 32-bit word written little-endian, so `0100007F` is
/// `127.0.0.1` and not `1.0.0.127`. IPv6 is four such words. Getting this
/// backwards produces addresses that look plausible and match nothing, which
/// is the worst kind of wrong.
pub fn decode_address(hex: &str) -> Option<String> {
    match hex.len() {
        8 => {
            let word = u32::from_str_radix(hex, 16).ok()?;
            let [a, b, c, d] = word.to_le_bytes();
            Some(format!("{a}.{b}.{c}.{d}"))
        }
        32 => {
            let mut bytes = [0u8; 16];
            for (i, chunk) in hex.as_bytes().chunks(8).enumerate() {
                let word = u32::from_str_radix(std::str::from_utf8(chunk).ok()?, 16).ok()?;
                bytes[i * 4..i * 4 + 4].copy_from_slice(&word.to_le_bytes());
            }
            // An IPv4-mapped address (`::ffff:a.b.c.d`) is how a dual-stack
            // node reports an ordinary v4 connection, and it has to come back
            // as the v4 address or it will match no pod IP in the cluster.
            if bytes[..10].iter().all(|b| *b == 0) && bytes[10] == 0xff && bytes[11] == 0xff {
                return Some(format!("{}.{}.{}.{}", bytes[12], bytes[13], bytes[14], bytes[15]));
            }
            Some(std::net::Ipv6Addr::from(bytes).to_string())
        }
        _ => None,
    }
}

/// Whether an address is worth reporting as a peer.
///
/// Loopback is the pod talking to itself — a sidecar, a health check, a mesh
/// proxy — and `0.0.0.0` is a listener with no peer at all. Neither is a
/// dependency.
fn interesting(address: &str) -> bool {
    !(address.starts_with("127.") || address == "0.0.0.0" || address == "::" || address == "::1")
}

/// Whether THIS end dialled, from the two port numbers.
///
/// A socket table does not record who opened a connection, and without this it
/// shows up from both ends: reading the client gives `storefront -> prometheus`
/// and reading the server gives `prometheus -> storefront`, which is the same
/// conversation drawn twice and one of them backwards. Found by running it —
/// the reverse edge is not something a unit test would have thought to look
/// for.
///
/// The dialler holds an ephemeral local port and connects to a service port, so
/// the lower remote port is the one being served. It is a heuristic and it is
/// the one every netstat-style tool uses; where it is wrong — two ephemeral
/// ports, a service listening high — it drops an edge rather than inventing a
/// backwards one, which is the right way to be wrong.
fn we_dialled(local_port: u16, remote_port: u16) -> bool {
    remote_port < local_port
}

/// Read one or more `/proc/net/tcp`-shaped tables into peers.
///
/// Tolerant of what it is handed: the header line, blank lines and any row it
/// cannot parse are skipped rather than failing the read. This is exec output
/// from an arbitrary container image, and half an answer about a pod's
/// connections is worth more than none.
pub fn parse_proc_net_tcp(text: &str) -> Vec<Peer> {
    let mut counts: BTreeMap<(String, u16), u32> = BTreeMap::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // sl, local_address, rem_address, st, …
        if fields.len() < 4 || !fields[0].ends_with(':') {
            continue;
        }
        if fields[3] != ESTABLISHED {
            continue;
        }
        let Some((hex_address, hex_port)) = fields[2].split_once(':') else {
            continue;
        };
        let Some(address) = decode_address(hex_address) else {
            continue;
        };
        let Ok(port) = u16::from_str_radix(hex_port, 16) else {
            continue;
        };
        let Some(local_port) = fields[1]
            .split_once(':')
            .and_then(|(_, hex)| u16::from_str_radix(hex, 16).ok())
        else {
            continue;
        };
        if !interesting(&address) || !we_dialled(local_port, port) {
            continue;
        }
        *counts.entry((address, port)).or_insert(0) += 1;
    }
    counts
        .into_iter()
        .map(|((address, port), count)| Peer { address, port, count })
        .collect()
}

/// Read one pod's socket table.
///
/// `cat` rather than a shell: a shell is the thing distroless images most often
/// lack, and this needs no globbing or redirection. Both tables are asked for
/// in one exec — a dual-stack node reports some connections in each, and two
/// execs to learn one pod's peers would double a cost that is already the
/// reason this is opt-in.
pub async fn read_pod_connections(
    client: kube::Client,
    namespace: &str,
    pod: &str,
) -> Result<Vec<Peer>, String> {
    let api: Api<Pod> = Api::namespaced(client, namespace);
    let params = AttachParams::default().stdout(true).stderr(false);
    let mut attached = api
        .exec(pod, vec!["cat", "/proc/net/tcp", "/proc/net/tcp6"], &params)
        .await
        .map_err(|e| e.to_string())?;
    let mut stdout = attached.stdout().ok_or("exec produced no stdout")?;
    let mut text = String::new();
    stdout
        .read_to_string(&mut text)
        .await
        .map_err(|e| e.to_string())?;
    // The status frame is taken but not required: `cat` exits non-zero when
    // only `/proc/net/tcp6` is missing, on a node with IPv6 disabled, and the
    // v4 table it already printed is a perfectly good answer.
    let _ = attached.take_status();
    Ok(parse_proc_net_tcp(&text))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ConnectionsIn {
    pub context: String,
    pub namespace: String,
    /// The pods to read. Named explicitly rather than discovered here: this is
    /// one exec each, and the caller is the one that knows how many it is
    /// willing to pay for.
    pub pods: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct PodConnections {
    pub pod: String,
    pub peers: Vec<Peer>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConnectionsOut {
    pub connections: Vec<PodConnections>,
    /// Pods that could not be read, and why.
    ///
    /// Reported rather than dropped: a distroless image with no `cat`, a pod
    /// that refuses exec and a pod that is not running are all ordinary, and a
    /// reader looking at a graph missing half its pods has to be able to tell
    /// that from a graph where those pods genuinely talk to nothing.
    pub unreadable: Vec<Unreadable>,
}

#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
pub struct Unreadable {
    pub pod: String,
    pub reason: String,
}

/// `k8s.podConnections` — the established TCP connections of the named pods.
pub fn pod_connections_capability(cache: Arc<ClientCache>) -> Capability {
    Capability::typed::<ConnectionsIn, ConnectionsOut, _, _>(
        "k8s.podConnections",
        "read the established TCP connections of pods, from their own /proc/net/tcp",
        // Read-only in effect — it runs `cat` on a path that cannot be written
        // — but it is an exec, and an exec is in the audit log of every pod it
        // touches. Callers must ask for it deliberately.
        Annotations::READ_ONLY,
        move |input: ConnectionsIn| {
            let cache = cache.clone();
            async move {
                let client = cache
                    .get(&input.context)
                    .await
                    .map_err(CapabilityError::Handler)?;
                let mut connections = Vec::new();
                let mut unreadable = Vec::new();
                for pod in &input.pods {
                    match read_pod_connections(client.clone(), &input.namespace, pod).await {
                        Ok(peers) => connections.push(PodConnections {
                            pod: pod.clone(),
                            peers,
                        }),
                        // One pod refusing must not cost the rest: a namespace
                        // with a single distroless sidecar would otherwise
                        // report nothing at all.
                        Err(reason) => unreadable.push(Unreadable {
                            pod: pod.clone(),
                            reason,
                        }),
                    }
                }
                Ok(ConnectionsOut {
                    connections,
                    unreadable,
                })
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn capability_has_expected_id() {
        let cap = pod_connections_capability(ClientCache::new(PathBuf::from("/x")));
        assert_eq!(cap.id, "k8s.podConnections");
    }

    #[test]
    fn an_ipv4_address_is_little_endian() {
        // The one that bites: read the other way round this is `1.0.0.127`,
        // which looks like an address and matches nothing in the cluster.
        assert_eq!(decode_address("0100007F").as_deref(), Some("127.0.0.1"));
        assert_eq!(decode_address("0A800A0A").as_deref(), Some("10.10.128.10"));
        assert_eq!(decode_address("00000000").as_deref(), Some("0.0.0.0"));
        assert_eq!(decode_address("nonsense"), None);
        assert_eq!(decode_address("0100"), None);
    }

    #[test]
    fn an_ipv4_mapped_v6_address_comes_back_as_v4() {
        // How a dual-stack node reports an ordinary v4 connection. Left as
        // `::ffff:10.1.2.3` it would match no pod IP in the cluster.
        assert_eq!(
            decode_address("0000000000000000FFFF00000302010A").as_deref(),
            Some("10.1.2.3"),
        );
    }

    #[test]
    fn a_real_v6_address_stays_v6() {
        // `2001:db8::1`, as four little-endian words: the first is the bytes
        // `20 01 0d b8` written backwards, which is the whole trap in this
        // format.
        assert_eq!(
            decode_address("B80D01200000000000000000 01000000".replace(' ', "").as_str()).as_deref(),
            Some("2001:db8::1"),
        );
    }

    #[test]
    fn only_established_sockets_become_peers() {
        // A listener has no remote to name, and a socket in TIME_WAIT is a
        // conversation that has already ended — drawing those puts edges on the
        // diagram for things a pod used to talk to.
        let table = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000     0        0 1
   1: 0A800A0A:CFA2 0B800A0A:1F90 01 00000000:00000000 00:00000000 00000000     0        0 2
   2: 0A800A0A:CFA3 0C800A0A:1F90 06 00000000:00000000 00:00000000 00000000     0        0 3
";
        let peers = parse_proc_net_tcp(table);
        assert_eq!(
            peers,
            vec![Peer { address: "10.10.128.11".into(), port: 8080, count: 1 }],
        );
    }

    #[test]
    fn only_the_end_that_dialled_reports_the_connection() {
        // Found by running it against a real cluster: without this the same
        // conversation appears from both ends and one of them is backwards —
        // `promstub -> storefront` for a connection storefront opened.
        //
        // The client's row: an ephemeral local port, the service port remote.
        let client = "   0: 0A800A0A:A56B 0B800A0A:2382 01 0 0 0 0 0 0 0 1";
        assert_eq!(
            parse_proc_net_tcp(client),
            vec![Peer { address: "10.10.128.11".into(), port: 9090, count: 1 }],
        );
        // The server's row for the very same connection, which must say nothing.
        let server = "   0: 0B800A0A:2382 0A800A0A:A56B 01 0 0 0 0 0 0 0 1";
        assert_eq!(parse_proc_net_tcp(server), vec![]);
    }

    #[test]
    fn sockets_to_the_same_place_are_counted_not_repeated() {
        // A connection pool is many sockets to one peer, and that is one
        // dependency held open several times — the count is the interesting
        // part, and a repeated edge would be noise.
        let table = "\
   0: 0A800A0A:A56B 0B800A0A:1F90 01 0 0 0 0 0 0 0 1
   1: 0A800A0A:A56C 0B800A0A:1F90 01 0 0 0 0 0 0 0 2
   2: 0A800A0A:A56D 0B800A0A:1F90 01 0 0 0 0 0 0 0 3
";
        let peers = parse_proc_net_tcp(table);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].count, 3);
    }

    #[test]
    fn a_pod_talking_to_itself_is_not_a_dependency() {
        // Loopback is a sidecar, a health check or a mesh proxy.
        let table = "   0: 0100007F:1F90 0100007F:CFA2 01 0 0 0 0 0 0 0 1";
        assert_eq!(parse_proc_net_tcp(table), vec![]);
    }

    #[test]
    fn rubbish_is_skipped_rather_than_failing_the_read() {
        // This is exec output from an arbitrary container image: half an answer
        // about a pod's connections is worth more than none.
        let table = "\
cat: /proc/net/tcp6: No such file or directory
  sl  local_address rem_address   st
not a row at all
   1: 0A800A0A:CFA2 0B800A0A:1F90 01 0 0 0 0 0 0 0 2
   2: badhex:CFA3 alsobad:1F90 01 0 0 0 0 0 0 0 3
";
        let peers = parse_proc_net_tcp(table);
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].address, "10.10.128.11");
    }

    #[test]
    fn both_tables_in_one_read_are_merged() {
        // `cat /proc/net/tcp /proc/net/tcp6` prints two tables back to back,
        // each with its own header.
        let table = "\
  sl  local_address rem_address   st
   0: 0A800A0A:CFA2 0B800A0A:1F90 01 0 0 0 0 0 0 0 1
  sl  local_address rem_address   st
   0: 0000000000000000FFFF00000A800A0A:CFA3 0000000000000000FFFF00000D800A0A:0050 01 0 0 0 0 0 0 0 2
";
        let peers = parse_proc_net_tcp(table);
        assert_eq!(peers.len(), 2);
        assert!(peers.iter().any(|p| p.address == "10.10.128.11" && p.port == 8080));
        assert!(peers.iter().any(|p| p.address == "10.10.128.13" && p.port == 80));
    }
}
