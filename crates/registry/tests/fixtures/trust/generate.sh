#!/bin/sh
# Writes the test trust root and the signed test catalog with scripts/extensions/trust.mjs,
# so the tests that read them also check that script's output. Every key here comes from a
# seed in this file: anyone can sign with it, so nothing but a test may trust it. The seeds
# match `trust::testing` in crates/registry/src/extensions/trust.rs.
#
#   sh crates/registry/tests/fixtures/trust/generate.sh
set -eu

here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../../.." && pwd)
trust="node $root/scripts/extensions/trust.mjs"
seed() { printf 'seed:'; i=0; while [ $i -lt 32 ]; do printf '%s' "$1"; i=$((i + 1)); done; }

root_a=$(seed a1)
root_b=$(seed a2)
catalog=$(seed c1)
example=$(seed e1)

$trust root --version 1 \
  --root "$root_a" --root "$root_b" --root-threshold 2 \
  --catalog "$catalog" --catalog-threshold 1 \
  --sign "$root_a" --sign "$root_b" >"$here/root.json"

# srelens, with the key every release before #559 was signed with.
$trust publisher --id srelens --name srelens --key "$here/srelens-apps.pub" \
  --namespace org.srelens --sign "$catalog" >"$here/srelens.json"
$trust publisher --id example --name "Example Labs" --key "$example" \
  --namespace com.example-labs --sign "$catalog" >"$here/example.json"
# The package tests' publisher (#562), whose seed they sign with. Shipped with the test root,
# as the srelens delegation is, so the package tests need no catalog.
$trust publisher --id test-publisher --name "Test Publisher" \
  --key "$here/../packages/test-publisher.pub" --namespace test.signed \
  --sign "$catalog" >"$here/test-publisher.json"
$trust bundle "$here/srelens.json" "$here/test-publisher.json" >"$here/publishers.json"

# A release in Example Labs' namespace, signed with a signature that names its key, as
# releases since #559 are.
sed 's/"org.srelens.argocd"/"com.example-labs.argocd"/' "$here/../argocd-0.3.0-manifest.json" \
  >"$here/example-release.json"
$trust release --sign "$example" "$here/example-release.json" >"$here/example-release.json.sig"

# The unsigned fixture's entries, signed for a host that trusts the test root. It expires
# in 2100, so a test that reads it does not start failing on a date.
$trust catalog --in "$here/../extension-catalog.json" \
  --publisher "$here/srelens.json" --publisher "$here/example.json" \
  --version 1 --expires 2100-01-01T00:00:00Z --sign "$catalog" \
  >"$here/../extension-catalog.signed.json"
