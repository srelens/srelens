package sidecar

import (
	"context"

	"github.com/srelens/srelens/sdk/go/protocol"
)

// CallContext is the cluster and namespace one call to srelens names.
type CallContext = protocol.CallContext

// ContextError is why NewCallContext refused. A handler that returns it is
// answered -32602.
type ContextError = protocol.ContextError

// NewCallContext is the cluster and namespace a call names, held to the
// rules srelens checks: a cluster that is not blank, in at most 4096 bytes,
// and a Kubernetes namespace name or nil for none.
func NewCallContext(clusterID string, namespace *string) (CallContext, error) {
	return protocol.NewCallContext(clusterID, namespace)
}

func sharedFrom(ctx context.Context) *shared {
	sh, _ := ctx.Value(sessionKey{}).(*shared)
	return sh
}

// DataDir is the one directory the sidecar may write, which is also its
// working directory. Write scratch files here: os.TempDir is not writable on
// Windows. It is "" for a context the SDK did not make.
func DataDir(ctx context.Context) string {
	if sh := sharedFrom(ctx); sh != nil {
		return sh.dataDir
	}
	return ""
}

// Limits are the limits srelens runs the sidecar under; zero for a context
// the SDK did not make.
func Limits(ctx context.Context) protocol.InitializeLimits {
	if sh := sharedFrom(ctx); sh != nil {
		return sh.limits
	}
	return protocol.InitializeLimits{}
}

// APIVersion is the sidecar API version srelens and the sidecar agreed on;
// "" for a context the SDK did not make.
func APIVersion(ctx context.Context) string {
	if sh := sharedFrom(ctx); sh != nil {
		return sh.apiVersion
	}
	return ""
}
