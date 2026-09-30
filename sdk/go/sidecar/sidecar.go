// Package sidecar is the srelens sidecar SDK for Go: it serves an executable
// app's operations and streams to srelens over stdin and stdout, as
// docs/extensions/sidecar-protocol.md describes.
package sidecar

import (
	"context"
	"encoding/json"
	"fmt"
	"io"

	"github.com/srelens/srelens/sdk/go/protocol"
)

// Sidecar is a sidecar: its name, its version, and the handlers it serves.
type Sidecar struct {
	name, version string
	operations    map[string]operationFunc
}

// New is a sidecar called name, at version, serving nothing yet.
func New(name, version string) *Sidecar {
	return &Sidecar{name: name, version: version, operations: map[string]operationFunc{}}
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

// operationFunc runs one operation on its raw params; its raw result.
type operationFunc func(ctx context.Context, params json.RawMessage) (json.RawMessage, error)

// register panics when name cannot name an operation or a stream, or is
// taken: a programming error, found the first time the sidecar starts.
func (s *Sidecar) register(name string) {
	if !protocol.IsIdentifier(name) || protocol.IsReserved(name) {
		panic(fmt.Sprintf("sidecar: `%s` cannot name an operation or a stream: use 1 to 64 ASCII letters, "+
			"digits and hyphens, as the manifest does, and none of srelens's own methods", name))
	}
	if _, taken := s.operations[name]; taken {
		panic(fmt.Sprintf("sidecar: `%s` is registered twice", name))
	}
}

// Operation serves the request name: srelens's request runs handler with its
// params decoded into In, and answers with its Out, or with its error (see
// Error). Params that do not decode into In are answered -32602 without
// running handler. encoding/json leaves a missing field at its zero value,
// so check the fields the operation needs and return InvalidParams when one
// is missing. The handler's ctx is done when srelens cancels the request
// (cause ErrCancelled), the session ends (ErrSessionEnded), or the handler
// returns (ErrHandlerReturned).
func Operation[In, Out any](s *Sidecar, name string, handler func(context.Context, In) (Out, error)) {
	s.register(name)
	s.operations[name] = func(ctx context.Context, params json.RawMessage) (json.RawMessage, error) {
		var in In
		if err := decode(params, &in); err != nil {
			return nil, InvalidParams(err.Error())
		}
		out, err := handler(ctx, in)
		if err != nil {
			return nil, err
		}
		result, err := json.Marshal(out)
		if err != nil {
			return nil, Internal("the result could not be serialized: " + err.Error())
		}
		return result, nil
	}
}

// decode reads params into v; absent params are null.
func decode(params json.RawMessage, v any) error {
	if len(params) == 0 {
		params = json.RawMessage("null")
	}
	return json.Unmarshal(params, v)
}
