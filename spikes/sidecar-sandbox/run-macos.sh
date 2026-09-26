#!/bin/sh
# Run the #571 checks on macOS: the unsandboxed baseline, then the Seatbelt backend.
#
# From the repository root, on a Mac with Rust installed:
#
#   sh spikes/sidecar-sandbox/run-macos.sh                     # the checks
#   SEATBELT_TRACE=1 sh spikes/sidecar-sandbox/run-macos.sh    # the checks, plus a trace
#
# Everything printed is also written to spikes/sidecar-sandbox/results/results-macos.txt.
#
# The trace runs the probe once, outside the test harness, under src/seatbelt.sb with its
# `(deny default)` rule replaced by one that allows everything and REPORTS each operation
# the profile does not already allow. The unified log then lists every operation the
# profile is missing, in one run, even when the probe cannot start under the real profile.
# It also runs automatically when the probe could not start.
#
# Exit status:
#   0  seatbelt passed every check it claims (1, 2, 3, 4 and 7)
#   1  the probe ran under the profile, and a claimed check failed (or was INCONCLUSIVE)
#   2  the probe could not start under the profile, so no check says anything about
#      Seatbelt yet; the sandbox denials (and the trace) are printed
# The baseline's failures and checks 5 (memory) and 6 (CPU) are matrix output: macOS is
# not known to enforce either limit, so they are recorded, not claimed.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
claimed="c1_ c2_ c3_ c4_ c7_"
probe_log_filter='sender == "Sandbox" AND eventMessage CONTAINS "probe("'

# run BACKEND LOG [TEST-FILTERS...]: run the checks, print the log, return cargo's status.
run() {
  backend=$1
  log=$2
  shift 2
  status=0
  SPIKE_BACKEND="$backend" cargo test --test checks -- --nocapture --test-threads=1 "$@" >"$log" 2>&1 \
    || status=$?
  grep -Ev '^[[:space:]]*(Finished|Running|running|$)' "$log" || true
  return "$status"
}

# classify STATUS LOG: the verdict on a claimed-checks run (see the exit statuses above).
classify() {
  if [ "$1" = 0 ]; then
    echo "PASS: seatbelt passed every check it claims"
    return 0
  fi
  if grep -qE 'the sidecar must be alive before any check|the sidecar could not start' "$2"; then
    echo "!!!!! the probe could not start under the profile, so no check says anything about Seatbelt yet: see the sandbox denials below"
    return 2
  fi
  echo "!!!!! the seatbelt backend failed a check it claims to enforce"
  return 1
}

# sandbox_log START: the sandbox's log entries for the probe since START (local time).
sandbox_log() {
  log show --style compact --start "$1" --predicate "$probe_log_filter" 2>&1 | tail -n 200 || true
}

# trace: run the probe once under the report-everything variant of the profile, and list
# the operations it reported.
trace() {
  echo "===== SEATBELT_TRACE: the operations src/seatbelt.sb does not already allow"
  if ! grep -qx '(deny default)' "$here/src/seatbelt.sb"; then
    echo "!!!!! src/seatbelt.sb has no '(deny default)' line to replace; trace skipped"
    return
  fi
  work=$(mktemp -d)
  work=$(cd "$work" && pwd -P)
  mkdir -p "$work/scratch" "$work/outside"
  echo "not for the sidecar" >"$work/outside/secret.txt"
  probe="$(cd "$here/target/debug" && pwd -P)/probe"
  requests="$work/requests.jsonl"
  cat >"$requests" <<EOF
{"jsonrpc":"2.0","id":1,"method":"ping","params":{}}
{"jsonrpc":"2.0","id":2,"method":"echo","params":{"n":1}}
{"jsonrpc":"2.0","id":3,"method":"write_file","params":{"path":"$work/scratch/t.txt","text":"x"}}
{"jsonrpc":"2.0","id":4,"method":"read_file","params":{"path":"$work/scratch/t.txt"}}
{"jsonrpc":"2.0","id":5,"method":"read_file","params":{"path":"$work/outside/secret.txt"}}
{"jsonrpc":"2.0","id":6,"method":"write_file","params":{"path":"$work/outside/w.txt","text":"x"}}
{"jsonrpc":"2.0","id":7,"method":"tcp_connect","params":{"addr":"1.1.1.1:443"}}
{"jsonrpc":"2.0","id":8,"method":"resolve","params":{"host":"example.com"}}
{"jsonrpc":"2.0","id":9,"method":"spawn_child","params":{}}
EOF
  start=$(date '+%Y-%m-%d %H:%M:%S')
  ran=0
  # Two spellings of the same rule; the first one sandbox-exec accepts is used.
  for rule in '(allow default (with report))' '(allow (with report) default)'; do
    profile="$work/trace.sb"
    sed "s/^(deny default)\$/$rule/" "$here/src/seatbelt.sb" >"$profile"
    echo "--- trace profile rule: $rule"
    out=$(cd "$work/scratch" && /usr/bin/sandbox-exec -f "$profile" -D "PROBE=$probe" \
      -D "SCRATCH=$work/scratch" "$probe" <"$requests" 2>&1) || true
    echo "$out"
    if echo "$out" | grep -q '"id":1,'; then
      ran=1
      break
    fi
  done
  [ "$ran" = 1 ] || echo "!!!!! neither trace rule ran the probe; the output above says why"
  sleep 3 # let the unified log catch up
  echo "--- every operation the sandbox reported for the probe, with counts"
  log show --style compact --start "$start" --predicate "$probe_log_filter" 2>&1 \
    | sed -nE 's/.*Sandbox: probe\([0-9]+\) //p' | sort | uniq -c | sort -rn || true
  rm -rf "$work"
}

main() {
  echo "srelens sidecar-sandbox spike (#571), macOS run, $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "git: $(git -C "$here" rev-parse --short HEAD 2>/dev/null || echo unknown)"
  echo "macOS: $(sw_vers -productVersion) ($(sw_vers -buildVersion)), arch: $(uname -m), kernel: $(uname -r)"
  echo "rustc: $(rustc --version)"
  if [ -x /usr/bin/sandbox-exec ]; then
    echo "sandbox-exec: present at /usr/bin/sandbox-exec"
  else
    echo "sandbox-exec: MISSING, so the seatbelt backend cannot run"
    echo 2 >"$status_file"
    return
  fi

  cd "$here"
  cargo build --quiet --all-targets
  log=$(mktemp)

  echo "===== SPIKE_BACKEND=none (the unsandboxed baseline: nine checks are expected to fail)"
  run none "$log" || true
  echo "===== SPIKE_BACKEND=seatbelt (all seven checks: the matrix)"
  run seatbelt "$log" || true

  echo "===== SPIKE_BACKEND=seatbelt, claimed checks only ($claimed): these decide the exit status"
  start=$(date '+%Y-%m-%d %H:%M:%S')
  status=0
  # shellcheck disable=SC2086 # $claimed is a list of test-name filters.
  run seatbelt "$log" $claimed || status=$?
  verdict=0
  classify "$status" "$log" || verdict=$?
  rm -f "$log"

  if [ "$verdict" = 2 ]; then
    echo "===== sandbox denials for the probe during the claimed-checks run"
    sleep 3
    sandbox_log "$start"
  fi
  if [ "$verdict" = 2 ] || [ "${SEATBELT_TRACE:-0}" = 1 ]; then
    trace
  fi

  echo "$verdict" >"$status_file"
}

if [ "${RUN_MACOS_LIB:-0}" != 1 ]; then
  # Keep the exit status through the tee: main records it in $status_file, and a run that
  # breaks before recording anything counts as a failure.
  status_file=$(mktemp)
  mkdir -p "$here/results"
  main 2>&1 | tee "$here/results/results-macos.txt"
  verdict=$(cat "$status_file")
  rm -f "$status_file"
  echo "results: spikes/sidecar-sandbox/results/results-macos.txt (exit ${verdict:-1})"
  exit "${verdict:-1}"
fi
