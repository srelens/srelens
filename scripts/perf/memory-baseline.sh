#!/usr/bin/env bash
# Resident-memory baseline for issue #31.
#
# Sums RSS across a running app's WHOLE process tree. That total is the only
# fair comparison here: an Electron app spreads its footprint over a main
# process plus renderer/GPU/utility helpers, and a Tauri app over its binary
# plus the OS WebView's own processes. Measuring one process would flatter
# whichever architecture hides more of itself in children.
#
# Processes are collected three ways, unioned:
#   1. roots     — executables whose basename matches the app name
#   2. children  — every descendant of those roots, by PPID
#   3. --include — processes matching an extra pattern (see the macOS note)
#
# ON macOS THIS SCRIPT CANNOT ATTRIBUTE THE WEBVIEW BY ITSELF. WKWebView's
# helpers (com.apple.WebKit.WebContent / .GPU / .Networking) are XPC services
# adopted by launchd: their parent is PID 1, not the app, and their argv names
# no client. They are neither descendants nor name matches, yet they hold the
# bulk of a Tauri app's memory — omitting them understates the Tauri side and
# flatters srelens. Measure on a machine where NO OTHER WebKit app is running
# (no Safari, no other Tauri/WebKit app) and pass them in explicitly:
#
#   scripts/perf/memory-baseline.sh srelens --include com.apple.WebKit
#
# The script refuses to report a macOS total while unattributed WebKit
# processes exist and --include was not given, rather than printing a number
# that is quietly too low.
#
# Usage:
#   scripts/perf/memory-baseline.sh srelens --include com.apple.WebKit
#   scripts/perf/memory-baseline.sh Freelens --settle 30 --samples 5
set -euo pipefail

usage() { sed -n '2,31p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

APP="${1:-}"
[ -z "$APP" ] && usage 1
shift
SETTLE=10
SAMPLES=5
INCLUDE=""
while [ $# -gt 0 ]; do
  case "$1" in
    --settle) SETTLE="$2"; shift 2 ;;
    --samples) SAMPLES="$2"; shift 2 ;;
    --include) INCLUDE="$2"; shift 2 ;;
    -h|--help) usage 0 ;;
    *) echo "unknown argument: $1" >&2; exit 1 ;;
  esac
done

APP_LOWER="$(echo "$APP" | tr '[:upper:]' '[:lower:]')"

# Root PIDs: matched on the executable's BASENAME, never on arguments. An
# argument match (`pgrep -f`) is wrong here — this repo's own path contains
# "srelens", so every shell and build tool run from it would match.
root_pids() {
  ps -A -o pid=,comm= 2>/dev/null | awk -v app="$APP_LOWER" '
    { pid = $1; $1 = ""; sub(/^ +/, ""); n = split(tolower($0), parts, "/")
      if (index(parts[n], app) > 0) print pid }'
}

# Roots plus every descendant, so Electron's differently-named helpers and
# Linux WebKitGTK's WebKitWebProcess (both real children) are counted.
tree_pids() {
  local roots frontier next pid children
  roots="$(root_pids)"
  [ -z "$roots" ] && return
  echo "$roots"
  frontier="$roots"
  while [ -n "$(echo "$frontier" | tr -d '[:space:]')" ]; do
    next=""
    for pid in $frontier; do
      children="$(ps -A -o pid=,ppid= 2>/dev/null | awk -v p="$pid" '$2 == p { print $1 }')"
      [ -n "$children" ] && next="$next $children"
    done
    [ -z "$(echo "$next" | tr -d '[:space:]')" ] && break
    echo "$next" | tr ' ' '\n' | grep -v '^$'
    frontier="$next"
  done
}

# Extra processes named by --include, matched against the full command.
#
# The measurement tooling must never measure itself: with the documented
# `--include com.apple.WebKit`, that pattern is present in THIS script's own
# argv (and in the shell that launched it, and any CI wrapper around it), so a
# naive match adds the sampler's RSS to every reading. Self, the launching
# shell, and anything whose command mentions this script are excluded.
SELF_NAME="$(basename "$0")"
include_pids() {
  [ -z "$INCLUDE" ] && return
  ps -A -o pid=,args= 2>/dev/null | awk -v pat="$INCLUDE" -v self="$$" -v parent="$PPID" -v script="$SELF_NAME" '
    { pid = $1; $1 = ""
      if (pid == self || pid == parent) next
      if (index($0, script) > 0) next
      if (index($0, pat) > 0) print pid }'
}

all_pids() {
  { tree_pids; include_pids; } | grep -v '^$' | sort -u -n
}

# Unattributable WebKit XPC services: launchd-parented, argv names no client.
orphan_webkit_pids() {
  ps -A -o pid=,ppid=,comm= 2>/dev/null | awk '
    $2 == 1 && index($0, "com.apple.WebKit") > 0 { print $1 }'
}

# `ps` reports RSS in KiB on both macOS and Linux — no per-platform branch.
total_rss_mib() {
  local pids
  pids="$(all_pids | tr '\n' ',' | sed 's/,$//')"
  [ -z "$pids" ] && { echo ""; return; }
  ps -o rss= -p "$pids" 2>/dev/null | awk '{sum += $1} END {printf "%.1f", sum / 1024}'
}

if [ -z "$(root_pids)" ]; then
  echo "No running process matches '$APP'. Start it first, then re-run." >&2
  exit 1
fi

# Refuse to publish a knowingly-low macOS number. Called before the settle
# delay AND before every sample: a WebView is created lazily, so helpers can
# appear after the app root is already visible — passing the guard once at
# startup would then wave through the exact understated total it exists to
# prevent.
assert_webkit_attributable() {
  [ "$(uname -s)" != "Darwin" ] && return 0
  # Coverage is what matters, not whether --include was passed: a narrow
  # pattern like `--include WebContent` picks up the renderer while silently
  # dropping the GPU and Networking services, which is the same understated
  # total the guard exists to catch. So check which orphans are ACTUALLY in
  # the collected set and complain about the ones that are not.
  collected=" $(all_pids | tr '\n' ' ') "
  orphans=""
  for orphan_pid in $(orphan_webkit_pids); do
    case "$collected" in
      *" $orphan_pid "*) ;;
      *) orphans="$orphans $orphan_pid" ;;
    esac
  done
  orphans="$(echo "$orphans" | tr ' ' '\n' | grep -v '^$' || true)"
  if [ -n "$orphans" ]; then
    orphan_mib="$(ps -o rss= -p "$(echo "$orphans" | tr '\n' ',' | sed 's/,$//')" 2>/dev/null \
      | awk '{sum += $1} END {printf "%.1f", sum / 1024}')"
    {
      echo "REFUSING TO REPORT: $(echo "$orphans" | wc -l | tr -d ' ') launchd-parented"
      echo "com.apple.WebKit process(es) are running and NOT in the counted set,"
      echo "holding ${orphan_mib} MiB this script cannot attribute (PPID 1, no"
      echo "client in argv):"
      echo "$orphans" | while read -r p; do
        [ -n "$p" ] && ps -o pid=,comm= -p "$p" 2>/dev/null | sed 's/^/  /'
      done
      echo
      echo "For a Tauri app those ARE its WebView, and excluding them understates it."
      echo "Quit every other WebKit app (Safari included), then re-run with a"
      echo "pattern that covers all of them:"
      echo "  $0 $APP --include com.apple.WebKit"
    } >&2
    exit 2
  fi
  return 0
}

assert_webkit_attributable

echo "Settling for ${SETTLE}s before sampling '$APP'..." >&2
sleep "$SETTLE"

readings=()
snapshots=()
for _ in $(seq "$SAMPLES"); do
  # Liveness is checked on the ROOTS, not on the aggregate reading. With
  # --include, launchd-owned WebKit helpers outlive the app they served, so a
  # total stays nonempty after the app has quit — and the median would then
  # describe orphaned helpers rather than the application.
  [ -z "$(root_pids)" ] && { echo "'$APP' exited while sampling — discarding." >&2; exit 1; }
  # Helpers may have appeared during the settle delay or between samples.
  assert_webkit_attributable
  # The PID set is captured ONCE per sample and the reading taken from that
  # exact set, so the audit below describes the processes that actually
  # produced these numbers — not whatever happens to be running at print time.
  snapshot="$(all_pids)"
  [ -z "$snapshot" ] && { echo "'$APP' exited while sampling — discarding." >&2; exit 1; }
  reading="$(ps -o rss= -p "$(echo "$snapshot" | tr '\n' ',' | sed 's/,$//')" 2>/dev/null \
    | awk '{sum += $1} END {printf "%.1f", sum / 1024}')"
  [ -z "$reading" ] && { echo "'$APP' exited while sampling — discarding." >&2; exit 1; }
  readings+=("$reading")
  snapshots+=("$(echo "$snapshot" | tr '\n' ' ')")
  sleep 1
done
# The settle delay and sampling window are both long enough for a crash to
# land after the final reading, so confirm the app is still up at the end too.
[ -z "$(root_pids)" ] && { echo "'$APP' exited during sampling — discarding." >&2; exit 1; }

median="$(printf '%s\n' "${readings[@]}" | sort -n \
  | awk '{a[NR]=$1} END {print (NR % 2) ? a[(NR+1)/2] : (a[NR/2] + a[NR/2+1]) / 2}')"

echo "app:        $APP"
echo "processes:  $(all_pids | wc -l | tr -d ' ')"
echo "samples:    ${readings[*]}"
echo "median RSS: ${median} MiB"
echo "counted:"
# Every process that contributed to a reading, so contamination (a second
# instance, a stray headless server, an unrelated WebKit client) is visible
# instead of averaged in. A process present in only SOME samples is flagged:
# it moved the numbers without being there throughout, which is exactly the
# transient contamination a re-query at print time would have hidden.
sampled_pids="$(printf '%s\n' "${snapshots[@]}" | tr ' ' '\n' | grep -v '^$' | sort -u -n)"
for pid in $sampled_pids; do
  seen=0
  for snap in "${snapshots[@]}"; do
    case " $snap " in *" $pid "*) seen=$((seen + 1)) ;; esac
  done
  detail="$(ps -o rss=,comm= -p "$pid" 2>/dev/null || true)"
  if [ -z "$detail" ]; then
    printf '  %-8s %13s  (exited during sampling) [%d/%d samples]\n' "$pid" "-" "$seen" "$SAMPLES"
  else
    echo "$detail" | awk -v pid="$pid" -v seen="$seen" -v total="$SAMPLES" '
      { rss = $1; $1 = ""; sub(/^ +/, "")
        flag = (seen == total) ? "" : sprintf(" [%d/%d samples]", seen, total)
        printf "  %-8s %8.1f MiB  %s%s\n", pid, rss / 1024, $0, flag }'
  fi
done
