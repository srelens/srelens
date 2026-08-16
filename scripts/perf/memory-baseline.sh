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
include_pids() {
  [ -z "$INCLUDE" ] && return
  ps -A -o pid=,args= 2>/dev/null | awk -v pat="$INCLUDE" '
    { pid = $1; $1 = ""; if (index($0, pat) > 0) print pid }'
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

# Refuse to publish a knowingly-low macOS number.
if [ "$(uname -s)" = "Darwin" ] && [ -z "$INCLUDE" ]; then
  orphans="$(orphan_webkit_pids)"
  if [ -n "$orphans" ]; then
    orphan_mib="$(ps -o rss= -p "$(echo "$orphans" | tr '\n' ',' | sed 's/,$//')" 2>/dev/null \
      | awk '{sum += $1} END {printf "%.1f", sum / 1024}')"
    {
      echo "REFUSING TO REPORT: $(echo "$orphans" | wc -l | tr -d ' ') launchd-parented"
      echo "com.apple.WebKit processes are running, holding ${orphan_mib} MiB that this"
      echo "script cannot attribute to an app (PPID 1, no client in argv)."
      echo
      echo "For a Tauri app those ARE its WebView, and excluding them understates it."
      echo "Quit every other WebKit app (Safari included), then re-run with:"
      echo "  $0 $APP --include com.apple.WebKit"
    } >&2
    exit 2
  fi
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

median="$(printf '%s\n' "${readings[@]}" | sort -n \
  | awk '{a[NR]=$1} END {print (NR % 2) ? a[(NR+1)/2] : (a[NR/2] + a[NR/2+1]) / 2}')"

echo "app:        $APP"
echo "processes:  $(all_pids | wc -l | tr -d ' ')"
echo "samples:    ${readings[*]}"
echo "median RSS: ${median} MiB"
echo "counted:"
# Every counted process, so contamination (a second instance, a stray headless
# server, an unrelated WebKit client) is visible instead of averaged in.
for pid in $(all_pids); do
  ps -o pid=,rss=,comm= -p "$pid" 2>/dev/null \
    | awk '{ pid = $1; rss = $2; $1 = ""; $2 = ""; sub(/^ +/, ""); printf "  %-8s %8.1f MiB  %s\n", pid, rss / 1024, $0 }'
done
