#!/bin/sh
# Fills each fuzz target's corpus with the examples and fixtures the unit tests already hold
# valid, so the fuzzer starts from documents that pass every check. Nothing is copied into
# the repository: fuzz/corpus is ignored, and its seeds are the files the tests read.
#
#   sh fuzz/seed.sh
set -eu

root=$(cd "$(dirname "$0")/.." && pwd)
corpus="$root/fuzz/corpus"
fixtures="$root/crates/registry/tests/fixtures"

mkdir -p "$corpus/manifest" "$corpus/catalog" "$corpus/signed-manifest" "$corpus/inventory"
cp "$root"/examples/extensions/*.json "$fixtures/argocd-manifest.json" "$corpus/manifest/"
cp "$fixtures/extension-catalog.json" "$corpus/catalog/"
cp "$fixtures"/extension-inventory*.json "$corpus/inventory/"

# signed-manifest reads a length byte, the signature, then the manifest. \100 is 64.
{
  printf '\100'
  cat "$fixtures/argocd-manifest.sig" "$fixtures/argocd-manifest.json"
} >"$corpus/signed-manifest/argocd"
