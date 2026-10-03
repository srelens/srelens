# SDKs for executable extensions

An executable app runs as a sidecar that speaks JSON-RPC to srelens over its stdin and
stdout ([protocol](../docs/extensions/sidecar-protocol.md)). What is here helps a third
party write one.

Executable apps are a preview. They run out of the box on Windows. On Linux they are not out of the box:
they need the launcher `srelens-sandbox-launch`, Landlock and a delegated cgroup v2
directory, set up by hand ([what is needed](../docs/extensions/manifest.md#where-executable-apps-run)). On macOS they do not run yet. srelens refuses to start any sidecar until its memory and CPU watchdog has
been checked with Seatbelt on a macOS 27 Mac.

| Path | What | Status |
|---|---|---|
| `protocol/` | `srelens-sidecar-protocol`: the wire's constants and a type for every message. srelens builds its own messages from it, and [`schemas/sidecar-protocol.v0.1.json`](../schemas/sidecar-protocol.v0.1.json) is generated from it; generating it (the `schema()` function and the `JsonSchema` derives, so schemars) is the crate's optional `schema` feature, off by default, so a sidecar does not build it | here |
| `rust/` | The Rust SDK: lifecycle, calls to the host with an explicit context, streams, cancellation, logging | here |
| `go/` | The Go SDK, with its protocol types generated from the schema | here |
| `examples/hello-world/` | A minimal sidecar in each language, run under the real supervisor by one suite | Rust and Go here |

Everything here stays in this repository, unpublished, until the protocol is stable.
