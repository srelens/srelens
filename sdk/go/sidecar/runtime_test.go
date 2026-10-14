package sidecar

import (
	"testing"

	"github.com/srelens/srelens/sdk/go/protocol"
)

type knobs struct {
	env      map[string]string
	memLimit int64
	procs    int
	setProcs []int
}

func (k *knobs) runtime() runtimeKnobs {
	return runtimeKnobs{
		getenv:      func(name string) string { return k.env[name] },
		setMemLimit: func(n int64) int64 { k.memLimit = n; return 0 },
		gomaxprocs: func(n int) int {
			current := k.procs
			if n > 0 {
				k.setProcs = append(k.setProcs, n)
				k.procs = n
			}
			return current
		},
	}
}

func TestTheRuntimeIsSizedToSrelenssLimits(t *testing.T) {
	k := &knobs{procs: 8}
	sizeRuntime(protocol.InitializeLimits{MemoryBytes: 256 << 20, CPUs: 1}, k.runtime())
	if k.memLimit != (256<<20)/10*9 {
		t.Fatalf("memory limit %d", k.memLimit)
	}
	if len(k.setProcs) != 1 || k.setProcs[0] != 1 {
		t.Fatalf("GOMAXPROCS set to %v", k.setProcs)
	}
}

func TestFractionalCPUsRoundUp(t *testing.T) {
	k := &knobs{procs: 8}
	sizeRuntime(protocol.InitializeLimits{CPUs: 1.5}, k.runtime())
	if len(k.setProcs) != 1 || k.setProcs[0] != 2 {
		t.Fatalf("GOMAXPROCS set to %v", k.setProcs)
	}
}

func TestGOMAXPROCSIsNeverRaised(t *testing.T) {
	k := &knobs{procs: 1}
	sizeRuntime(protocol.InitializeLimits{CPUs: 4}, k.runtime())
	if len(k.setProcs) != 0 {
		t.Fatalf("GOMAXPROCS set to %v", k.setProcs)
	}
}

func TestTheAuthorsSettingsWin(t *testing.T) {
	k := &knobs{procs: 8, env: map[string]string{"GOMEMLIMIT": "1GiB", "GOMAXPROCS": "4"}}
	sizeRuntime(protocol.InitializeLimits{MemoryBytes: 256 << 20, CPUs: 1}, k.runtime())
	if k.memLimit != 0 || len(k.setProcs) != 0 {
		t.Fatalf("memory %d, procs %v", k.memLimit, k.setProcs)
	}
}
