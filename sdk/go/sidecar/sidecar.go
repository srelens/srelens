// Package sidecar is the srelens sidecar SDK for Go: it serves an executable
// app's operations and streams to srelens over stdin and stdout, as
// docs/extensions/sidecar-protocol.md describes.
package sidecar

import (
	"context"
	"io"
)

// Sidecar is a sidecar: its name, its version, and the handlers it serves.
type Sidecar struct {
	name, version string
}

// New is a sidecar called name, at version, serving nothing yet.
func New(name, version string) *Sidecar {
	return &Sidecar{name: name, version: version}
}

// Run serves srelens over r and w until srelens shuts the sidecar down, r
// ends, or ctx is done. It closes w when it returns, if w is an io.Closer, so
// the other end reads to its end. The error is nil after shutdown or the end
// of r; otherwise it is a failed read or write, ErrProtocol for a line that
// is not JSON-RPC, or ctx's error. Tests use Run; a sidecar's main uses
// RunStdio.
func (s *Sidecar) Run(ctx context.Context, r io.Reader, w io.Writer) error {
	return serve(ctx, s, r, w)
}
