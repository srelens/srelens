package protocol

import (
	"fmt"
	"slices"
	"strings"
)

// The checks srelens's broker holds a call's fields to
// (crates/plugin-host/src/sidecar/broker.rs), built on the rules the schema
// states (fieldShapes, generated).

// IsIdentifier reports whether v can be a binding (capability) or action
// name: 1 to 64 ASCII letters, digits and hyphens.
func IsIdentifier(v string) bool { return fits("HostReadParams.capability", v) }

// IsBindingNames checks the generated array bounds and each item's shape.
func IsBindingNames(values []string) bool {
	const key = "HostBindingAvailabilityParams.bindings"
	shape, ok := arrayShapes[key]
	if !ok {
		panic("protocol: the schema states no rule for " + key)
	}
	if len(values) < shape.min || len(values) > shape.max {
		return false
	}
	seen := make(map[string]bool, len(values))
	for _, value := range values {
		if !fits(key+".items", value) || (shape.unique && seen[value]) {
			return false
		}
		seen[value] = true
	}
	return true
}

// IsObjectName reports whether v can be a Kubernetes object name: 1 to 253
// ASCII letters, digits, dots and hyphens, and not "." or "..".
func IsObjectName(v string) bool { return fits("HostResourceParams.name", v) }

// IsToken reports whether v can be a uid or resourceVersion: 1 to 128
// printable ASCII characters.
func IsToken(v string) bool { return fits("HostActionParams.uid", v) }

// IsNamespace reports whether v can be a Kubernetes namespace name.
func IsNamespace(v string) bool { return fits("CallContext.namespace", v) }

// MaxClusterIDBytes is the longest clusterId srelens takes, in bytes as UTF-8.
var MaxClusterIDBytes = fieldShapes["CallContext.clusterId"].maxLength

// IsClusterID reports whether v can name a cluster: not blank, in at most
// MaxClusterIDBytes bytes. Blank is what srelens's Rust str::trim trims,
// Unicode White_Space, which strings.TrimSpace trims too.
func IsClusterID(v string) bool {
	return strings.TrimSpace(v) != "" && len(v) <= MaxClusterIDBytes
}

func fits(key, v string) bool {
	s, ok := fieldShapes[key]
	if !ok {
		panic("protocol: the schema states no rule for " + key)
	}
	return len(v) <= s.maxLength && s.pattern.MatchString(v) && !slices.Contains(s.not, v)
}

// IsReserved reports whether name is srelens's own, which no operation or
// stream may take: a method srelens sends, or anything under "$/" or
// "stream/".
func IsReserved(name string) bool {
	if strings.HasPrefix(name, "$/") || strings.HasPrefix(name, "stream/") {
		return true
	}
	direction, ok := MethodDirections[name]
	return ok && (direction == "hostToSidecar" || direction == "both")
}

// ContextError is why NewCallContext refused: the field and, for a
// namespace, the value.
type ContextError struct {
	Field string // "clusterId" or "namespace"
	Value string
}

func (e *ContextError) Error() string {
	if e.Field == "clusterId" {
		return fmt.Sprintf("`clusterId` must name a cluster, in at most %d bytes", MaxClusterIDBytes)
	}
	value := []rune(e.Value)
	if len(value) > 64 {
		value = value[:64]
	}
	return fmt.Sprintf("`namespace` must be a Kubernetes namespace name, or none; not %q", string(value))
}

// NewCallContext is the cluster and namespace one call names, held to the
// rules srelens checks. A nil namespace is none: every namespace, or a
// cluster-scoped kind.
func NewCallContext(clusterID string, namespace *string) (CallContext, error) {
	if !IsClusterID(clusterID) {
		return CallContext{}, &ContextError{Field: "clusterId"}
	}
	if namespace == nil {
		return CallContext{ClusterID: clusterID}, nil
	}
	if !IsNamespace(*namespace) {
		return CallContext{}, &ContextError{Field: "namespace", Value: *namespace}
	}
	ns := *namespace
	return CallContext{ClusterID: clusterID, Namespace: &ns}, nil
}
