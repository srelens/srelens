#!/bin/sh
# Run the #571 checks under every Linux backend, inside a Docker container.
#
# From the repository root:
#
#   docker run --rm --privileged -v "$PWD/spikes/sidecar-sandbox:/spike:ro" \
#     rust:1-bookworm sh /spike/run-linux.sh
#
# --privileged is needed for two things only: a writable /sys/fs/cgroup (the cgroup
# backends) and the user namespaces bubblewrap creates. Landlock and seccomp need neither;
# the second pass below runs those two layers in an unprivileged container to show it
# (pass UNPRIVILEGED=1 and drop --privileged).
set -eu

# Copy the sources only: /spike/target may hold a Windows build.
mkdir -p /work
cp -r /spike/Cargo.toml /spike/Cargo.lock /spike/src /spike/tests /work/
cd /work
export CARGO_TARGET_DIR=/target

echo "kernel: $(uname -r)"
if [ "${UNPRIVILEGED:-0}" = 1 ]; then
  backends="none landlock seccomp"
else
  # cgroup v2's "no internal processes" rule: the container's own processes must leave the
  # root before it can hand controllers to children.
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
for backend in $backends; do
  echo "===== SPIKE_BACKEND=$backend"
  SPIKE_BACKEND="$backend" cargo test --test checks -- --nocapture --test-threads=1 2>&1 \
    | grep -Ev '^\s*(Finished|Running|running|$)' || true
done
