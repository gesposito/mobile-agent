#!/usr/bin/env bash
# The notes of a release: the section of CHANGELOG.md headed
# "## <version> — <YYYY-MM-DD>", without its heading and the blank lines
# around it, which the release workflow's publish job gives the release
# (design D2 of the alpha-release change). Before anything is published it
# refuses a version other than the one in Cargo.toml's [workspace.package],
# which the daemon and the MCP server print as theirs, and a section that is
# missing, undated or empty.
#
# Usage: scripts/release-notes.sh <version>
#
# Run from the repository's root. Prints the notes and exits 0; says why on
# standard error and exits 1 when it refuses, 2 on a usage error.
set -euo pipefail

if [ $# -ne 1 ] || [ -z "$1" ]; then
  echo "usage: scripts/release-notes.sh <version>" >&2
  exit 2
fi
version=$1

# The first version key of the [workspace.package] table.
workspace=$(awk '
  /^\[/ { table = $0; next }
  table == "[workspace.package]" && /^version[[:space:]]*=/ {
    sub(/^version[[:space:]]*=[[:space:]]*"/, "")
    sub(/".*$/, "")
    print
    exit
  }
' Cargo.toml)
if [ "$version" != "$workspace" ]; then
  echo "release-notes: the version $version is not the workspace's, ${workspace:-none} in Cargo.toml's [workspace.package], which the daemon and the MCP server print" >&2
  exit 1
fi

heading=$(awk -v version="$version" '$1 == "##" && $2 == version { print; exit }' CHANGELOG.md)
if [ -z "$heading" ]; then
  echo "release-notes: CHANGELOG.md has no section headed ## $version" >&2
  exit 1
fi
date=${heading#"## $version — "}
if [ "$date" = "$heading" ] || ! [[ $date =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}$ ]]; then
  echo "release-notes: CHANGELOG.md heads the section '$heading', not ## $version — <YYYY-MM-DD> with the release's date" >&2
  exit 1
fi

# The lines between the heading and the next second-level one, from the
# first that is not blank; the command substitution drops the blank ones at
# the end.
notes=$(awk -v version="$version" '
  $1 == "##" { if (inside) exit; inside = ($2 == version); next }
  inside { print }
' CHANGELOG.md | sed -e '/[^[:space:]]/,$!d')
if [ -z "$notes" ]; then
  echo "release-notes: the section ## $version of CHANGELOG.md is empty" >&2
  exit 1
fi
printf '%s\n' "$notes"
