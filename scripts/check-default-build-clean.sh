#!/usr/bin/env bash
# Assert that the default cargo build does not resolve the gated secure
# dependency. See docs/SECURE_BUILD.md for context.
#
# The kirk-server crate declares a feature with the same name, so we cannot
# grep the feature name out of `cargo metadata --no-deps`. Instead we resolve
# the full dependency graph (`cargo metadata` with deps) and assert that no
# resolved package carries the secure crate name. We also assert Cargo.lock
# does not name it.
set -euo pipefail

needle="secret-kirk-edge"

# Cargo.lock check: the resolved lock file must not list a [[package]] with
# the secure crate name.
if [ -f Cargo.lock ]; then
  if grep -E '^name = "'"${needle}"'"' Cargo.lock > /dev/null 2>&1; then
    echo "Cargo.lock contains a resolved package named ${needle} — must not be in the default build" >&2
    exit 1
  fi
fi

# Cargo metadata (default features) check: walk resolved packages.
if cargo metadata --format-version 1 2>/dev/null \
    | python3 -c '
import json, sys
needle = sys.argv[1]
data = json.load(sys.stdin)
for p in data.get("packages", []):
    if p.get("name") == needle:
        print(f"resolved package {needle} in metadata")
        sys.exit(1)
sys.exit(0)
' "${needle}"; then
  echo "default build is clean"
else
  echo "Default cargo metadata resolved the secure crate ${needle} — must not be in the default build" >&2
  exit 1
fi
