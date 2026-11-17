# pod-connect, in Go

A srelens sidecar that dials a pod, built on [`sdk/go`](../../../go). One operation:

- `fetch {cluster, namespace, object, pod}` connects to port 80 of `pod`, sends `GET / HTTP/1.0`, and answers `{"line": "<first line of the reply>"}`.

`Host.Dial` returns a `net.Conn`, so the example reads and writes it like any socket; it sets a deadline so a silent pod cannot hold the operation, and closes the conn.

`extension.json` is the manifest. It binds `k8s.listDeployments` and a `k8s.connectPod` binding scoped to that reader's objects, so `object` is a Deployment and `pod` one of its pods. A pod of another Deployment is refused. The binding and its permission are what let the program connect: see [pod connections](../../../../docs/extensions/manifest.md#pod-connections), and [the sidecar protocol](../../../../docs/extensions/sidecar-protocol.md#network) for the tunnel itself.

Test it from this directory. The tests play srelens over pipes, so no cluster is needed:

```bash
go test ./...
```

To run it, build the binary into the layout the manifest names (`bin/<platform>/pod-connect`), pack the directory with `pack-extension`, and install the package from Settings → Apps ([packages](../../../../docs/extensions/packages.md)).
