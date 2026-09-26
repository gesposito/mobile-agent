#!/usr/bin/env bash
# Builds every reference fixture: runs each fixtures/<name>/build.sh from its
# own directory, in name order. Every script runs even when an earlier one
# fails, and the exit status is non-zero if any of them failed.
set -euo pipefail

cd "$(dirname "$0")"
shopt -s nullglob
scripts=(*/build.sh)

if [ "${#scripts[@]}" -eq 0 ]; then
  echo "fixtures/*/build.sh not present, skipping"
  exit 0
fi

failed=()
for script in "${scripts[@]}"; do
  fixture=${script%/build.sh}
  echo "==> fixtures/$script"
  if ! (cd "$fixture" && ./build.sh); then
    echo "fixtures/$script failed" >&2
    failed+=("$fixture")
  fi
done

if [ "${#failed[@]}" -gt 0 ]; then
  echo "fixture builds failed: ${failed[*]}" >&2
  exit 1
fi
