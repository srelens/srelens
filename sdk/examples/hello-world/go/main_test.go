package main

import (
	"context"
	"errors"
	"testing"

	"github.com/srelens/srelens/sdk/go/sidecar"
)

// Handlers are plain functions: a unit test calls them directly.
func TestGreetGreetsAndRefusesAMissingName(t *testing.T) {
	got, err := greet(context.Background(), greetInput{Name: "srelens"})
	if err != nil || got.Greeting != "Hello, srelens" {
		t.Fatalf("%+v %v", got, err)
	}
	var refused *sidecar.Error
	if _, err := greet(context.Background(), greetInput{}); !errors.As(err, &refused) || refused.Code != -32602 {
		t.Fatalf("%v", err)
	}
}

// Outside a session there is no srelens to call.
func TestPodsNeedsASessionAndACluster(t *testing.T) {
	if _, err := pods(context.Background(), podsInput{Cluster: "kind-dev"}); !errors.Is(err, sidecar.ErrNoSession) {
		t.Fatalf("%v", err)
	}
	var ce *sidecar.ContextError
	if _, err := pods(context.Background(), podsInput{Cluster: " "}); !errors.As(err, &ce) {
		t.Fatalf("%v", err)
	}
}
