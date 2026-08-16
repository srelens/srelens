#!/usr/bin/env bash
# Resident-memory baseline for issue #31.
#
# Sums RSS across a running app's WHOLE process tree. That total is the only
# fair comparison here: an Electron app spreads its footprint over a main
# process plus renderer/GPU/utility helpers, and a Tauri app over its binary
# plus the OS WebView's own processes. Measuring one process would flatter
# whichever architecture hides more of itself in children.
#
# Usage:
#   scripts/perf/memory-baseline.sh srelens
#   scripts/perf/memory-baseline.sh Freelens
#   scripts/perf/memory-baseline.sh srelens --settle 30 --samples 5
#
# Reports the median of --samples readings taken a second apart, after waiting
# --settle seconds — startup allocation and lazy JIT make a single immediate
# reading unrepresentative. Run it twice: once idle, once with the scenario
# under test (clusters connected, watches live) already set up.
set -euo pipefail

usage() { sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

APP="${1:-}"
[ -z "$APP" ] && usage 1
shift
SETTLE=10
SAMPLES=5
while [ $# -gt 0 ]; do
  case "$1" in
    --settle) SETTLE="$2"; shift 2 ;;
    --samples) SAMPLES="$2"; shift 2 ;;
    -h|--help) usage 0 ;;
    *) echo "unknown argument: $1" >&2; exit 1 ;;
  esac
done

# Every PID whose EXECUTABLE is the app — matched on the program path, never
# on arguments. Matching command lines (`pgrep -f`) is wrong here: any shell,
# editor, or build tool run from a directory named after the app matches too,
# and each one silently inflates the total. Matching the executable path still
# catches Electron's helper binaries, which live under the app bundle.
# Matched on the executable's BASENAME, not its directory: this repo lives in
# a path containing "srelens", so a path match pulls in every unrelated tool
# built or run from here (esbuild, node, ...). A basename match still catches
# Electron's "<App> Helper (Renderer)" binaries.
app_pids() {
  ps -A -o pid=,comm= 2>/dev/null | awk -v app="$(echo "$APP" | tr '[:upper:]' '[:lower:]')" '
    {
      pid = $1
      $1 = ""
      sub(/^ +/, "")
      path = tolower($0)
      n = split(path, parts, "/")
      if (index(parts[n], app) > 0) print pid
    }' || true
}

# The matched processes, for the operator to eyeball. Anything unexpected here
# (a headless MCP server, a second instance) invalidates the reading.
print_matched() {
  ps -A -o pid=,rss=,comm= 2>/dev/null | awk -v app="$(echo "$APP" | tr '[:upper:]' '[:lower:]')" '
    {
      pid = $1; rss = $2
      $1 = ""; $2 = ""
      sub(/^ +/, "")
      path = tolower($0)
      n = split(path, parts, "/")
      if (index(parts[n], app) > 0) printf "  %-8s %8.1f MiB  %s\n", pid, rss / 1024, $0
    }'
}

# Total RSS in MiB across those PIDs. `ps` reports RSS in KiB on both macOS
# and Linux, which is why this needs no per-platform branch.
total_rss_mib() {
  local pids
  pids="$(app_pids | tr '\n' ',' | sed 's/,$//')"
  [ -z "$pids" ] && { echo ""; return; }
  ps -o rss= -p "$pids" 2>/dev/null | awk '{sum += $1} END {printf "%.1f", sum / 1024}'
}

if [ -z "$(app_pids)" ]; then
  echo "No running process matches '$APP'. Start it first, then re-run." >&2
  exit 1
fi

echo "Settling for ${SETTLE}s before sampling '$APP'..." >&2
sleep "$SETTLE"

readings=()
for _ in $(seq "$SAMPLES"); do
  reading="$(total_rss_mib)"
  [ -z "$reading" ] && { echo "'$APP' exited while sampling." >&2; exit 1; }
  readings+=("$reading")
  sleep 1
done

process_count="$(app_pids | wc -l | tr -d ' ')"
median="$(printf '%s\n' "${readings[@]}" | sort -n | awk '{a[NR]=$1} END {print (NR % 2) ? a[(NR+1)/2] : (a[NR/2] + a[NR/2+1]) / 2}')"

echo "app:        $APP"
echo "processes:  $process_count"
echo "samples:    ${readings[*]}"
echo "median RSS: ${median} MiB"
echo "counted:"
print_matched
