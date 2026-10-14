// Command hello-world is the smallest srelens sidecar in Go: an operation, a
// call to srelens on the cluster the caller names, and a stream. Declare
// these operations in the app's manifest (docs/extensions/manifest.md,
// "Executable apps").
package main

import (
	"context"
	"encoding/json"
	"log/slog"
	"os"

	"github.com/srelens/srelens/sdk/go/sidecar"
)

type greetInput struct {
	Name string `json:"name"`
}

type greeting struct {
	Greeting string `json:"greeting"`
}

// greet is `greet {name}`.
func greet(_ context.Context, in greetInput) (greeting, error) {
	if in.Name == "" {
		// encoding/json leaves a missing field empty, so the handler says so.
		return greeting{}, sidecar.InvalidParams("missing field `name`")
	}
	slog.Info("greeting " + in.Name)
	return greeting{Greeting: "Hello, " + in.Name}, nil
}

type podsInput struct {
	// srelens adds no cluster to an operation's input: the operation
	// declares one, and names it in every call it makes.
	Cluster   string  `json:"cluster"`
	Namespace *string `json:"namespace"`
}

// pods is `pods {cluster, namespace?}`: the app's pods reader, through srelens.
func pods(ctx context.Context, in podsInput) (json.RawMessage, error) {
	cc, err := sidecar.NewCallContext(in.Cluster, in.Namespace)
	if err != nil {
		return nil, err
	}
	return sidecar.HostFrom(ctx).Read(ctx, cc, "pods")
}

type countInput struct {
	To uint64 `json:"to"`
}

// count is the stream `count {to}`: 0 up to to, stopping when srelens
// cancels it.
func count(ctx context.Context, in countInput, frames *sidecar.Frames) error {
	for n := uint64(0); n < in.To; n++ {
		if ctx.Err() != nil {
			return nil
		}
		if err := frames.Send(n); err != nil {
			return err
		}
	}
	return nil
}

func main() {
	s := sidecar.New("hello-world", "0.1.0")
	sidecar.Operation(s, "greet", greet)
	sidecar.Operation(s, "pods", pods)
	sidecar.Stream(s, "count", count)
	os.Exit(s.RunStdio())
}
