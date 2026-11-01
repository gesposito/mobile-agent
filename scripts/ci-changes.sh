#!/usr/bin/env bash
# CI's change classifier: code when the diff between two commits touches a
# file outside the documentation set, documentation alone otherwise. The
# changes job of .github/workflows/ci.yml runs it on a pull request's merge
# commit against its first parent, the base branch's tip, and the Linux and
# macOS jobs skip a change of documentation alone.
#
# The documentation set: Markdown files anywhere (*.md), docs/, openspec/,
# the root's LICENSE files and .github/ISSUE_TEMPLATE/. No build, test or
# script reads them, which is what lets them skip the build and the tests; a
# Markdown file that a build or a test came to read would have to leave the
# set. Everything else is code: the workflows, the scripts, the crates, the
# fixtures, the agents, Cargo's and the tools' files, and the reports' data
# beside their Markdown. A rename counts as a deletion and an addition, so
# a file moved out of the code is code, and a diff that changes nothing is
# code, so that CI runs everything when there is nothing to classify.
#
# Usage: scripts/ci-changes.sh <base> <head>
#
# Prints code=true or code=false on stdout, the line a step appends to
# $GITHUB_OUTPUT, and on stderr each path that made the change code, as
# `code: <path>`, or the count of documentation files. Exits 2 without two
# arguments and with git's status when git fails, printing no answer.
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: scripts/ci-changes.sh <base> <head>" >&2
  exit 2
fi

# documentation <path>: whether the path is in the documentation set.
documentation() {
  case $1 in
    *.md | docs/* | openspec/* | .github/ISSUE_TEMPLATE/*) return 0 ;;
    */*) return 1 ;;
    LICENSE*) return 0 ;;
  esac
  return 1
}

scratch="$(mktemp -d "${TMPDIR:-/tmp}/ci-changes.XXXXXX")"
trap 'rm -rf "${scratch:?}"' EXIT
# Into a file first, so that a failing git fails the script before any answer.
git diff --no-renames --name-only -z "$1" "$2" -- > "$scratch/paths"

documents=0
code=()
while IFS= read -r -d '' path; do
  if documentation "$path"; then
    documents=$((documents + 1))
  else
    code+=("$path")
  fi
done < "$scratch/paths"

if [ "${#code[@]}" -gt 0 ]; then
  printf 'code: %s\n' "${code[@]}" >&2
  echo "code=true"
elif [ "$documents" -eq 0 ]; then
  echo "ci-changes: the diff changes no file, which counts as code" >&2
  echo "code=true"
else
  echo "ci-changes: $documents documentation files and no code" >&2
  echo "code=false"
fi
