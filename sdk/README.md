# SDKs for executable extensions

An executable app runs as a sidecar that speaks JSON-RPC to srelens over its stdin and
stdout ([protocol](../docs/extensions/sidecar-protocol.md)). What is here helps a third
party write one.

| Path | What | Status |
|---|---|---|
| `protocol/` | `srelens-sidecar-protocol`: the wire's constants and a type for every message. srelens builds its own messages from it, and [`schemas/sidecar-protocol.v0.1.json`](../schemas/sidecar-protocol.v0.1.json) is generated from it | here |
| `rust/` | The Rust SDK: lifecycle, calls to the host with an explicit context, streams, cancellation, logging | [#576](https://github.com/srelens/srelens/issues/576) |
| `go/` | The Go SDK, generated from the schema | [#576](https://github.com/srelens/srelens/issues/576) |
| `examples/hello-world/` | A minimal sidecar in each language | [#576](https://github.com/srelens/srelens/issues/576) |

Everything here stays in this repository, unpublished, until the protocol is stable.
