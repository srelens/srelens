#!/bin/sh
# Run the #571 checks on macOS: the unsandboxed baseline, then the Seatbelt backend.
#
# UNVERIFIED: written on Windows and never run; this is the first macOS run. From the
# repository root, on a Mac with Rust installed through rustup:
#
#   sh spikes/sidecar-sandbox/run-macos.sh
#
# Everything printed is also written to spikes/sidecar-sandbox/results/results-macos.txt.
#
# Exit status: non-zero if the seatbelt backend fails a check it claims to enforce (1, 2,
# 3, 4 and 7), or if the run itself breaks. The baseline is expected to fail nine checks.
# Checks 5 (memory) and 6 (CPU) are recorded, not claimed: macOS is not known to enforce
# either, so their result is matrix output and does not fail the script.
set -eu

here=$(cd "$(dirname "$0")" && pwd)
claimed="c1_ c2_ c3_ c4_ c7_"
status_file=$(mktemp)

run() {
  backend=$1
  shift
  log=$(mktemp)
  status=0
  SPIKE_BACKEND="$backend" cargo test --test checks -- --nocapture --test-threads=1 "$@" >"$log" 2>&1 \
    || status=$?
  grep -Ev '^[[:space:]]*(Finished|Running|running|$)' "$log" || true
  rm -f "$log"
  return "$status"
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
    echo 1 > "$status_file"
    return
  fi

  cd "$here"
  cargo build --quiet --all-targets

  echo "===== SPIKE_BACKEND=none (the unsandboxed baseline: nine checks are expected to fail)"
  run none || true
  echo "===== SPIKE_BACKEND=seatbelt (all seven checks: the matrix)"
  run seatbelt || true
  echo "===== SPIKE_BACKEND=seatbelt, claimed checks only ($claimed): these decide the exit status"
  failed=0
  # shellcheck disable=SC2086 # $claimed is a list of test-name filters.
  run seatbelt $claimed || failed=1
  if [ "$failed" = 1 ]; then
    echo "!!!!! the seatbelt backend failed a check it claims to enforce"
  fi

  echo "===== sandbox log: denials recorded for the probe in the last 15 minutes (may be empty)"
  log show --style compact --last 15m \
    --predicate 'sender == "Sandbox" AND eventMessage CONTAINS "probe"' 2>&1 | tail -n 200 || true

  echo "$failed" > "$status_file"
}

# Keep the exit status through the tee: main records it in $status_file, and a run that
# breaks before recording anything counts as a failure.
mkdir -p "$here/results"
main 2>&1 | tee "$here/results/results-macos.txt"
status=$(cat "$status_file")
rm -f "$status_file"
echo "results: spikes/sidecar-sandbox/results/results-macos.txt (exit ${status:-1})"
exit "${status:-1}"
