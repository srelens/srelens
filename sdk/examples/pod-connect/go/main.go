// Command pod-connect is a srelens sidecar that dials a pod: one operation,
// `fetch`, sends an HTTP request through a pod tunnel and returns the first
// line of the reply. The manifest's k8s.connectPod binding is what lets it
// (docs/extensions/manifest.md, "Pod connections").
package main

import (
	"bufio"
	"context"
	"fmt"
	"os"
	"strings"
	"time"

	"github.com/srelens/srelens/sdk/go/sidecar"
)

// connectBinding is the name of the manifest's k8s.connectPod binding.
const connectBinding = "connect"

type fetchInput struct {
	Cluster   string `json:"cluster"` // the operation declares one: srelens adds none
	Namespace string `json:"namespace"`
	Object    string `json:"object"` // the Deployment whose pods are in scope
	Pod       string `json:"pod"`
}

type reply struct {
	Line string `json:"line"`
}

// fetch is `fetch {cluster, namespace, object, pod}`.
func fetch(ctx context.Context, in fetchInput) (reply, error) {
	cc, err := sidecar.NewCallContext(in.Cluster, &in.Namespace)
	if err != nil {
		return reply{}, err
	}
	conn, err := sidecar.HostFrom(ctx).Dial(ctx, cc, sidecar.PodTarget{Capability: connectBinding, Object: in.Object, Pod: in.Pod})
	if err != nil {
		return reply{}, err
	}
	defer conn.Close()
	// A pod that never answers must not hold the operation.
	_ = conn.SetDeadline(time.Now().Add(10 * time.Second))
	if _, err := fmt.Fprint(conn, "GET / HTTP/1.0\r\n\r\n"); err != nil {
		return reply{}, err
	}
	line, err := bufio.NewReader(conn).ReadString('\n')
	if err != nil {
		return reply{}, err
	}
	return reply{Line: strings.TrimRight(line, "\r\n")}, nil
}

func newSidecar() *sidecar.Sidecar {
	s := sidecar.New("pod-connect", "0.1.0")
	sidecar.Operation(s, "fetch", fetch)
	return s
}

func main() { os.Exit(newSidecar().RunStdio()) }
