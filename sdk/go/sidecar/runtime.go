package sidecar

import (
	"math"
	"os"
	"runtime"
	"runtime/debug"

	"github.com/srelens/srelens/sdk/go/protocol"
)

// Go's garbage collector does not know the sandbox's memory limit, and
// inside the Linux sandbox Landlock hides the cgroup files Go would read its
// CPU limit from. So once srelens says its limits, RunStdio sizes the
// runtime to them -- unless the author set GOMEMLIMIT or GOMAXPROCS, which win.

// runtimeKnobs are what sizeRuntime reads and sets; tests pass their own.
type runtimeKnobs struct {
	getenv      func(string) string
	setMemLimit func(int64) int64
	gomaxprocs  func(int) int
}

var processKnobs = runtimeKnobs{getenv: os.Getenv, setMemLimit: debug.SetMemoryLimit, gomaxprocs: runtime.GOMAXPROCS}

// sizeRuntime sets the garbage collector's soft limit to 90% of the memory
// limit, and lowers GOMAXPROCS to the CPU limit, rounded up. It never raises
// GOMAXPROCS.
func sizeRuntime(limits protocol.InitializeLimits, k runtimeKnobs) {
	if k.getenv("GOMEMLIMIT") == "" && limits.MemoryBytes > 0 {
		if soft := limits.MemoryBytes / 10 * 9; soft <= math.MaxInt64 {
			k.setMemLimit(int64(soft))
		}
	}
	if k.getenv("GOMAXPROCS") == "" && limits.CPUs > 0 {
		want := max(1, int(math.Ceil(limits.CPUs)))
		if want < k.gomaxprocs(0) {
			k.gomaxprocs(want)
		}
	}
}
