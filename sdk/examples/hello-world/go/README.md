# hello-world, in Go

The smallest srelens sidecar in Go, built on [`sdk/go`](../../../go):

- `greet {name}` answers `{"greeting": "Hello, <name>"}`;
- `pods {cluster, namespace?}` reads the app's `pods` reader through srelens, on the cluster the caller names;
- the stream `count {to}` sends `0` up to `to`.

It is the Go twin of [the Rust example](../rust). One end-to-end suite runs both under srelens's real supervisor, unconfined and inside the OS sandbox. From the repository root:

```bash
go build -C sdk/examples/hello-world/go -o "$PWD/target/hello-world-go" .
SRELENS_HELLO_WORLD_GO="$PWD/target/hello-world-go" cargo test -p srelens-sidecar-hello-world --test supervised -- --ignored
```

Declaring these operations is the app manifest's job ([executable apps](../../../../docs/extensions/manifest.md#executable-apps)).
