#!/bin/sh
# Tests run-macos.sh's verdict logic without a Mac: sources the script in library mode
# (RUN_MACOS_LIB=1 defines its functions and runs nothing) and feeds `classify` logs shaped
# like real runs. Runs anywhere with a POSIX sh:
#
#   sh spikes/sidecar-sandbox/test-run-macos.sh
#
# classify STATUS LOG prints a verdict and returns 0 (every claimed check passed), 1 (the
# sidecar ran and a claimed check failed) or 2 (the probe never started under the profile,
# so no check says anything about Seatbelt).
set -eu

here=$(cd "$(dirname "$0")" && pwd)
RUN_MACOS_LIB=1
# shellcheck source=run-macos.sh
. "$here/run-macos.sh"

failed=0
log=$(mktemp)

expect() {
  name=$1
  want=$2
  status=$3
  got=0
  classify "$status" "$log" >/dev/null || got=$?
  if [ "$got" = "$want" ]; then
    echo "ok   $name"
  else
    echo "FAIL $name: expected $want, got $got"
    failed=1
  fi
}

# The maintainer's first Mac run (c7ff2875): every test died at the liveness ping.
cat >"$log" <<'EOF'
test c1_read_kubeconfig_is_denied ... sandbox-launch: RLIMIT_DATA = 134217728: refused: Invalid argument (os error 22)
sandbox-launch: RLIMIT_AS = 134217728: refused: Invalid argument (os error 22)
sandbox-launch: RLIMIT_CPU = 60: set
thread 'c1_read_kubeconfig_is_denied' panicked at tests/checks.rs:42:5:
[seatbelt] the sidecar must be alive before any check; ping got Stopped("exited: signal: 6 (SIGABRT)")
test result: FAILED. 0 passed; 5 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.40s
EOF
expect "the probe never started (liveness ping failed)" 2 101

# The launcher itself could not start the sidecar.
cat >"$log" <<'EOF'
thread 'c7_json_rpc_over_stdio_works' panicked at tests/checks.rs:38:10:
[seatbelt] the sidecar could not start: No such file or directory (os error 2)
test result: FAILED. 0 passed; 5 failed; 0 ignored; 0 measured; 6 filtered out; finished in 0.10s
EOF
expect "the sidecar could not be launched" 2 101

# The sidecar ran, and a claimed check failed.
cat >"$log" <<'EOF'
test c3_dns_resolution_is_denied ... [seatbelt] resolve example.com: Ok(Object {"addrs": Array [String("104.20.23.154")]})
thread 'c3_dns_resolution_is_denied' panicked at tests/checks.rs:66:9:
[seatbelt] resolve example.com must be denied, got Ok(Object {"addrs": Array [String("104.20.23.154")]})
test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 6 filtered out; finished in 2.10s
EOF
expect "a claimed check failed" 1 101

# A check that could not tell (no network on the Mac) is a failure, not a pass and not a
# start failure.
cat >"$log" <<'EOF'
thread 'c3_tcp_connect_to_internet_is_denied' panicked at tests/checks.rs:51:9:
INCONCLUSIVE: TCP connect to 1.1.1.1:443: the host itself could not connect to 1.1.1.1:443 (no network?) (Network is unreachable (os error 51)), so a failure inside the sandbox would say nothing about the sandbox
test result: FAILED. 4 passed; 1 failed; 0 ignored; 0 measured; 6 filtered out; finished in 2.10s
EOF
expect "an inconclusive check" 1 101

# Every claimed check passed.
cat >"$log" <<'EOF'
test c7_json_rpc_over_stdio_works ... [seatbelt] 50 JSON-RPC round trips over stdio
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 6 filtered out; finished in 3.20s
EOF
expect "every claimed check passed" 0 0

rm -f "$log"
exit "$failed"
