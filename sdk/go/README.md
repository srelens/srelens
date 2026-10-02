# sdk/go: the Go SDK for srelens sidecars

An executable srelens app runs as a sidecar that speaks JSON-RPC to srelens over its stdin and stdout ([protocol](../../docs/extensions/sidecar-protocol.md)). This package does the protocol for you; you write handlers.

```go
package main

import (
	"context"
	"encoding/json"
	"os"

	"github.com/srelens/srelens/sdk/go/sidecar"
)

type greetInput struct {
	Name string `json:"name"`
}

func main() {
	s := sidecar.New("scanner", "1.0.0")
	sidecar.Operation(s, "greet", func(ctx context.Context, in greetInput) (map[string]string, error) {
		if in.Name == "" {
			return nil, sidecar.InvalidParams("missing field `name`")
		}
		return map[string]string{"greeting": "Hello, " + in.Name}, nil
	})
	sidecar.Operation(s, "pods", func(ctx context.Context, in struct{ Cluster string }) (json.RawMessage, error) {
		cc, err := sidecar.NewCallContext(in.Cluster, nil)
		if err != nil {
			return nil, err // answered -32602
		}
		return sidecar.HostFrom(ctx).Read(ctx, cc, "pods")
	})
	sidecar.Stream(s, "count", func(ctx context.Context, in struct{ To int }, frames *sidecar.Frames) error {
		for n := 0; n < in.To; n++ {
			if err := frames.Send(n); err != nil {
				return err
			}
		}
		return nil
	})
	os.Exit(s.RunStdio())
}
```

The module is `github.com/srelens/srelens/sdk/go`, with Go 1.25 or later. It uses the standard library only. It stays in this repository, unpublished, until the protocol is stable; [the example](../examples/hello-world/go) depends on it with a `replace`.

## What the SDK does for you

- **Lifecycle.**
  - It answers `initialize`, picking the newest API version both sides speak (or `-32001`), and `activate`, `health`, `deactivate` and `shutdown`.
  - `health` is answered on the reader goroutine, through a lane the writer takes ahead of queued frames. A handler that blocks or spins cannot starve it: Go preempts goroutines, and a goroutine blocked in a system call gives up its thread.
- **Handlers.**
  - Register every `Operation` and `Stream` before `Run` or `RunStdio`. Registering one while the sidecar runs is a data race.
  - Each request and each stream runs in its own goroutine, with its params decoded into your type.
  - A handler's `ctx` carries what `initialize` said. `sidecar.DataDir(ctx)` is the directory the sidecar may write. `sidecar.APIVersion(ctx)` is the API version both sides agreed on. `sidecar.Limits(ctx)` holds srelens's limits, such as `MaxConcurrentRequests`, `RequestTimeoutMs` and `MemoryBytes`.
  - Params that do not decode are answered `-32602`, and an unknown method `-32601`.
  - `encoding/json` leaves a missing field at its zero value, so check the fields you need and return `sidecar.InvalidParams`.
- **What a handler returns** is the answer:
  - a `*sidecar.Error` (`InvalidParams`, `Internal`, `NewError(code, msg).WithData(v)`), even wrapped, is answered as it says;
  - a `*sidecar.ContextError` is answered `-32602`;
  - any other error is answered `-32603` with its text;
  - a panic is recovered, answered `-32603` ("the handler for `x` panicked"), and logged with its stack. The sidecar goes on.
- **Cancellation.** A handler's `ctx` is done when:
  - srelens cancels it: `context.Cause(ctx)` is `sidecar.ErrCancelled`, and a cancelled request is answered `-32800` for you;
  - the session ends: `ErrSessionEnded`;
  - the handler returns: `ErrHandlerReturned`, so a goroutine you left running sees `ctx.Done()`.

  The SDK cannot stop your code for you: pass `ctx` on, check it, and return.
- **Calls to srelens.**
  - `sidecar.HostFrom(ctx).Read`, `.Resource` and `.Action`. Each names its cluster with a `CallContext` from `sidecar.NewCallContext`: srelens has no current cluster, and adds none to an operation's input, so declare one and pass it on.
  - Every field is checked before sending, as srelens checks it. A bad one is `sidecar.ErrInvalidCall`, naming the field, and nothing is sent.
  - At most `min(8, maxConcurrentRequests)` calls are in flight; the rest wait.
  - When the `ctx` you pass ends first, the call returns `ctx.Err()`. If srelens had already seen it, it is sent a `$/cancelRequest`, and the slot stays taken until srelens answers.
  - srelens's refusals match `sidecar.ErrConsentDenied`, `ErrCapabilityFailed`, `ErrInvalidParams` and `ErrCallCancelled` through `errors.Is`. `errors.As` gives the `*sidecar.HostError` with its code and message. `ErrSessionEnded` means the session ended first.
- **Streams.**
  - The ack goes out before any frame. The stream ends in exactly one `stream/close`, or `stream/error` when the handler returns an error, and nothing follows it.
  - `Frames.Send` fails with `ErrStreamCancelled`, `ErrStreamFinished` (a `Frames` kept past its handler), `ErrSessionEnded`, `ErrFrameTooLarge`, or the JSON error.
- **Limits.** No line goes over the protocol's 4 MiB. A result that would is answered with an error, and a frame that would is refused to your handler.
- **Logging.** `RunStdio` makes `slog`'s default logger write `LEVEL target: message key=value` to stderr, which srelens keeps in the app's log at that level.
  - `target` is the sidecar's name, or a record's own `target` attribute.
  - Each line is cut to the 4 KiB srelens keeps.
  - `s.SetLogLevel(slog.LevelDebug)` lowers the level from Info.
- **The runtime, sized to the sandbox.** Once srelens says its limits, `RunStdio`:
  - sets the garbage collector's soft limit (`debug.SetMemoryLimit`) to 90% of the memory limit;
  - lowers `GOMAXPROCS` to the CPU limit.

  Setting `GOMEMLIMIT` or `GOMAXPROCS` yourself overrides either.
- **Exit.** `RunStdio` returns 0 after `shutdown` or when stdin ends, and 1 if the pipe failed or srelens wrote a line that is not JSON-RPC. End `main` with `os.Exit(s.RunStdio())`.

## The sandbox, for authors

- **stdout is the protocol.** Never write to it: no `fmt.Println`. Log with `slog`, or the standard `log` package, which `RunStdio` routes to the same handler.
- **No network and no subprocesses:** no `net`, no `os/exec`. Ask srelens for data through `HostFrom(ctx)`.
- **On Linux, set a file's mode through an open file** (`f.Chmod`), never by path (`os.Chmod`): the sandbox refuses changing a mode by path.
- **Write under `sidecar.DataDir(ctx)`,** which is also the working directory. `os.TempDir()` is not writable on Windows.
- **The environment holds only what srelens names.**
- **Memory is capped.** Linux kills the process past the limit, and on Windows allocation fails. The soft limit above makes the collector work harder before either happens.

## Testing your sidecar

- **Unit-test a handler by calling it.** With `context.Background()`, `HostFrom(ctx)` returns a host whose calls fail with `sidecar.ErrNoSession`, and `DataDir(ctx)` is `""`.
- **Test the whole sidecar with `s.Run(ctx, reader, writer)`,** which serves any pipe. `sdk/go/sidecar/harness_test.go` is a fake srelens to copy; it checks every line against the committed schema.
- **Run it under srelens's real supervisor,** as `sdk/examples/hello-world/rust/tests/` does for this SDK's example.

## Regenerating the protocol types

`protocol/protocol_gen.go` is generated from [`schemas/sidecar-protocol.v0.1.json`](../../schemas/sidecar-protocol.v0.1.json). After the schema changes, run `go generate ./...` here. CI fails when the file is stale.
