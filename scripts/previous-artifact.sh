#!/usr/bin/env bash
# Downloads the artifact <name> of this repository's workflow run <run id>
# into <dir>, through GitHub's REST API with gh: the previous scheduled run's
# report.json and report.md, which .github/workflows/benchmarks.yml compares
# a run with. Exits 1, saying why on stderr, when the run has no artifact of
# that name that has not expired.
#
# Usage: scripts/previous-artifact.sh <run id> <name> <dir>
#
# Needs GITHUB_REPOSITORY (owner/name, which Actions sets) and a token for gh
# in GH_TOKEN, one that may read the repository's actions.
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: scripts/previous-artifact.sh <run id> <name> <dir>" >&2
  exit 2
fi
run=$1
name=$2
dir=$3

id=$(gh api "repos/$GITHUB_REPOSITORY/actions/runs/$run/artifacts?per_page=100" \
  --jq ".artifacts[] | select(.name == \"$name\" and (.expired | not)) | .id" | head -n 1)
if [ -z "$id" ]; then
  echo "run $run has no unexpired artifact named $name" >&2
  exit 1
fi
mkdir -p "$dir"
gh api "repos/$GITHUB_REPOSITORY/actions/artifacts/$id/zip" > "$dir/artifact.zip"
unzip -q -o "$dir/artifact.zip" -d "$dir"
rm "$dir/artifact.zip"
