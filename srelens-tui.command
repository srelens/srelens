#!/bin/bash
cd "$(dirname "$0")"
if [ -f "./target/release/srelens-tui" ]; then
    ./target/release/srelens-tui "$@"
else
    ./target/debug/srelens-tui "$@"
fi
