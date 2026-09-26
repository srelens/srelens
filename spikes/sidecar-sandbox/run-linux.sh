#!/bin/sh
# Run the #571 checks under every Linux backend, inside a Docker container.
#
# From the repository root (bash, zsh or PowerShell):
#
#   docker run --rm --privileged -v "$PWD/spikes/sidecar-sandbox:/spike" rust:1-bookworm sh /spike/run-linux.sh
#
# Everything printed is also written to spikes/sidecar-sandbox/results/results-linux.txt.
# The sources are copied out of /spike before building; the only thing written back is the
# results file.
#
# --privileged is needed for two things only: a writable /sys/fs/cgroup (the cgroup
# backends) and the user namespaces bubblewrap creates. Landlock and seccomp need neither.
# To show that, run the unprivileged pass too; it runs only none, landlock and seccomp and
# writes results-linux-unprivileged.txt:
#
#   docker run --rm -e UNPRIVILEGED=1 -v "$PWD/spikes/sidecar-sandbox:/spike" rust:1-bookworm sh /spike/run-linux.sh
#
# Exit status: non-zero if the recommended backend (landlock+seccomp+cgroup) fails any
# check, or if the run itself breaks. The other backends are expected to fail some checks;
# their output is the matrix, not a failure of this script.
set -eu

required="landlock+seccomp+cgroup"
if [ "${UNPRIVILEGED:-0}" = 1 ]; then
  name=results-linux-unprivileged.txt
else
  name=results-linux.txt
fi
status_file=$(mktemp)

main() {
  echo "srelens sidecar-sandbox spike (#571), Linux run, $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "kernel: $(uname -r), arch: $(uname -m)"
  echo "rustc: $(rustc --version)"
  if [ -r /sys/kernel/security/lsm ]; then echo "LSMs: $(cat /sys/kernel/security/lsm)"; fi

  # Copy the sources only: /spike/target may hold a build for another OS.
  mkdir -p /work
  cp -r /spike/Cargo.toml /spike/Cargo.lock /spike/src /spike/tests /work/
  cd /work
  export CARGO_TARGET_DIR=/target

  if [ "${UNPRIVILEGED:-0}" = 1 ]; then
    echo "mode: unprivileged container (no --privileged, no --security-opt)"
    backends="none landlock seccomp"
  else
    echo "mode: privileged container"
    # cgroup v2's "no internal processes" rule: the container's own processes must leave
    # the root before it can hand controllers to children.
    mkdir -p /sys/fs/cgroup/init
    while read -r pid; do
      echo "$pid" > /sys/fs/cgroup/init/cgroup.procs 2>/dev/null || true
    done < /sys/fs/cgroup/cgroup.procs
    echo "+memory +cpu" > /sys/fs/cgroup/cgroup.subtree_control
    echo "cgroup controllers delegated: $(cat /sys/fs/cgroup/cgroup.subtree_control)"
    apt-get update -qq >/dev/null && apt-get install -y -qq bubblewrap >/dev/null
    echo "bubblewrap: $(bwrap --version)"
    backends="none landlock seccomp cgroup landlock+seccomp+cgroup bwrap"
  fi

  cargo build --quiet --all-targets
  failed=0
  for backend in $backends; do
    echo "===== SPIKE_BACKEND=$backend"
    log=$(mktemp)
    status=0
    SPIKE_BACKEND="$backend" cargo test --test checks -- --nocapture --test-threads=1 >"$log" 2>&1 \
      || status=$?
    grep -Ev '^\s*(Finished|Running|running|$)' "$log" || true
    rm -f "$log"
    if [ "$backend" = "$required" ] && [ "$status" -ne 0 ]; then
      echo "!!!!! the required backend $required failed (cargo test exit $status)"
      failed=1
    fi
  done
  echo "$failed" > "$status_file"
}

# Keep the exit status through the tee: main records it in $status_file, and a run that
# breaks before recording anything counts as a failure.
if mkdir -p /spike/results 2>/dev/null && touch "/spike/results/$name" 2>/dev/null; then
  out="/spike/results/$name"
  shown="spikes/sidecar-sandbox/results/$name"
else
  out="/tmp/$name"
  shown="$out inside the container (/spike is read-only, so nothing was written back)"
fi
main 2>&1 | tee "$out"
chown "$(stat -c %u:%g /spike)" /spike/results "$out" 2>/dev/null || true
status=$(cat "$status_file")
rm -f "$status_file"
echo "results: $shown (exit ${status:-1})"
exit "${status:-1}"
