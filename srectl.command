#!/bin/bash
cd "$(dirname "$0")"
if [ -f "./target/release/srectl" ]; then
    ./target/release/srectl "$@"
else
    ./target/debug/srectl "$@"
fi
