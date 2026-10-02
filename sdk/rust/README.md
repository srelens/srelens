# srelens-sidecar: the Rust SDK

Write the sidecar of an executable app in Rust. srelens starts it in the OS
sandbox and talks JSON-RPC to it over stdin and stdout
([protocol](../../docs/extensions/sidecar-protocol.md)). This crate is that
conversation, so a sidecar is its handlers:

```rust
use srelens_sidecar::{CallContext, Context, Error, Frames, Sidecar};

#[tokio::main]
async fn main() -> std::process::ExitCode {
    Sidecar::new("scanner", env!("CARGO_PKG_VERSION"))
        .operation("greet", |_ctx: Context, input: GreetInput| async move {
            Ok::<_, Error>(Greeting { greeting: format!("Hello, {}", input.name) })
        })
        .operation("pods", |ctx: Context, input: PodsInput| async move {
            let context = CallContext::new(input.cluster, input.namespace.as_deref())?;
            Ok::<_, Error>(ctx.host().read(&context, "pods").await?)
        })
        .stream("count", |_ctx: Context, input: CountInput, frames: Frames| async move {
            for n in 0..input.to { frames.send(&n).await?; }
            Ok(())
        })
        .run_stdio()
        .await
}
```

The whole example is [`sdk/examples/hello-world/rust`](../examples/hello-world/rust).
Declare the operations in the app's manifest
([Executable apps](../../docs/extensions/manifest.md#executable-apps)).

## What the SDK does for you

- **Lifecycle.** It answers `initialize` (and picks the API version), `activate`, `deactivate`, `health` and `shutdown`. `health` is answered from the reader loop, so it gets through alongside handlers that are merely awaiting — not ones that are blocking their thread; see below.
- **Handlers.**
  - Each request runs on its own task, with its input read into your type. Input that does not fit is answered `-32602` without running the handler.
  - A handler's `Error` is the answer. A panic is answered `-32603`, and the sidecar keeps running.
  - A stream ends in exactly one closing frame, even when its own error message is too large to send whole: that frame is replaced with one that says so instead. Nothing follows it: a `Frames` clone kept past the handler is refused (`StreamClosed::Finished`).
- **Cancellation.**
  - When srelens cancels a request or a stream, `ctx.cancelled()` resolves and `ctx.is_cancelled()` turns true.
  - A cancelled request is answered `-32800` for you. A cancelled stream's `send` fails.
  - The SDK does not stop the handler for you: check `ctx.is_cancelled()` or select on `ctx.cancelled()` and return.
- **Calls to srelens.**
  - The calls are `ctx.host().read`, `.resource` and `.action`. Each names its cluster with a `CallContext`.
  - srelens adds no cluster to an operation's input: declare one, and pass it on.
  - Every field is checked before the call is sent, against the shape srelens holds it to: `clusterId` is not blank and at most 4096 bytes, `namespace` is a Kubernetes namespace name or `None`, `capability` and `action` are identifiers (1 to 64 ASCII letters, digits and hyphens), `name` is an object name (1 to 253 ASCII letters, digits, dots and hyphens, and not `.` or `..`), and `uid` and `resourceVersion` are 1 to 128 printable ASCII characters. A call with a field srelens would refuse fails with `HostError::InvalidCall`, naming the field. Nothing is sent and no slot is taken.
  - A `CallContext` written as a struct literal is checked too, as `CallContext::new` checks its arguments. `CallContext::validate` runs the same check on one you hold.
  - At most 8 calls are in flight, which is srelens's limit, or fewer when the `maxConcurrentRequests` srelens sends in `initialize` is lower (0 is read as 8); the rest wait.
  - Dropping a call's future cancels it at srelens once srelens has seen the request; dropped while still waiting for a slot, its place is freed at once instead, since srelens never saw it.
  - Refusals come back as `HostError`: `ConsentDenied`, `CapabilityFailed`, `InvalidParams` (srelens refused the params), `InvalidCall` (the SDK refused the call before sending it), `Cancelled`, `Rpc` or `Disconnected`. `TooLarge` is only a backstop: every field has a length limit far under the message limit, so a call that passes the checks is never too large to send.
- **Limits.** No line goes over the protocol's 4 MiB. A result that would be larger is answered with an error instead, and a frame that would be larger is refused to your handler, as is one that cannot be serialized (`StreamClosed::Invalid`).
- **Logging.**
  - Use the `log` crate. Records go to stderr as `LEVEL target: message`, which srelens keeps in the app's log at that level.
  - Each line is cut to the 4 KiB srelens keeps.
  - A panic is logged as an error.
- **Exit.** The process exits after `shutdown`, or when srelens goes away, having flushed everything queued first — the session never cuts off its own last line. It exits 0 once that is done, and 1 if reading or writing the pipe to srelens failed, or srelens broke the protocol.

## The sandbox, for authors

- **Never block a handler's thread.** Run CPU-bound or blocking work with `tokio::task::spawn_blocking`, or srelens's health check can starve and srelens will restart the sidecar.
- **Stdout is the protocol.** Never write to it (`println!`, `print!`, or a dependency that does). Log with the `log` crate, which goes to stderr.
- **No network and no subprocesses.** To reach a cluster or the outside, go through `ctx.host()` and the capabilities your manifest declares.
- **Scratch files.** Write only under `ctx.data_dir()`, which is also the working directory. `std::env::temp_dir()` is not writable on Windows.
- **File modes (Linux).** Set a file's mode through an open file (`File::set_permissions`), never by path (`std::fs::set_permissions`): the seccomp filter refuses the path form.
- **Environment.** It holds only what srelens names, so don't rely on `HOME` or `PATH`.
- **Memory.** It is capped. Linux kills the process at the limit; on Windows, an allocation fails.

## Testing your sidecar

`Sidecar::run(reader, writer)` serves any async pipes. Drive it in a test with
`tokio::io::duplex`, acting as srelens: see this crate's `tests/common/mod.rs`
for a fake host you can copy. To run the real binary under srelens's own
supervisor, see the hello-world's `tests/`.

Everything here stays in this repository, unpublished, until the protocol is
stable (#576).
