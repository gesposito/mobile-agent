#!/usr/bin/env bash
# The four npm packages of a version, from the release's three tarballs
# (design D7 of the alpha-release change): one platform package per target,
# @mobileagent/cli-<platform>-<arch>, holding the tarball's three binaries in
# bin/ and its THIRD-PARTY-NOTICES.md, and @mobileagent/cli, the launcher
# whose optional dependencies they are. The release workflow's npm-package
# job runs it on the tarballs its build jobs attested, after verifying their
# attestations again; the maintainer runs it the same way on a release's
# downloaded tarballs. Nothing is built: the binaries are the tarballs'.
#
#   npm/pack.sh <version> <directory of the tarballs> <output directory>
#
# The directory holds mobile-agent-<version>-<target>.tar.gz and its
# .sha256 for each target, as the build jobs write them. Every tarball is
# checked against its sum and its binaries against its SHA256SUMS before
# anything is packed, so that a missing or altered one packs nothing. Each
# package is staged from its directory here with the repository's LICENSE,
# its version and the launcher's optional dependencies set to <version>,
# and must list exactly its files in `npm pack --dry-run`; then `npm pack`
# writes it to the output directory, with a SHA256SUMS of the four, and the
# list of each package is printed. The checkout is not changed.
set -euo pipefail

if [ $# -ne 3 ]; then
  echo "usage: npm/pack.sh <version> <directory of the tarballs> <output directory>" >&2
  exit 2
fi
version=$1
dist=$(cd "$2" && pwd)
mkdir -p "$3"
out=$(cd "$3" && pwd)
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# No check for a newer npm, which would reach the network.
export npm_config_update_notifier=false
root=$(dirname "$here")

if ! [[ $version =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z.-]+)?$ ]]; then
  echo "npm/pack.sh: the version must be a semantic version without the v, such as 0.1.0-alpha.1, not $version" >&2
  exit 1
fi

# Each platform package and the release's target whose tarball fills it, in
# the order the packages are published, the launcher last.
platforms=(darwin-arm64:aarch64-apple-darwin linux-x64:x86_64-unknown-linux-gnu linux-arm64:aarch64-unknown-linux-gnu)
commands=(mobile-agent mobile-agent-daemon mobile-agent-mcp)

stage=$(mktemp -d "${TMPDIR:-/tmp}/npm-pack.XXXXXX")
trap 'rm -rf "${stage:?}"' EXIT

fail() {
  echo "npm/pack.sh: $*" >&2
  exit 1
}

# 1. Every tarball, its sum and its binaries' sums, before anything is packed.
for entry in "${platforms[@]}"; do
  target=${entry#*:}
  name="mobile-agent-$version-$target"
  [ -f "$dist/$name.tar.gz" ] || fail "$dist/$name.tar.gz is missing"
  [ -f "$dist/$name.tar.gz.sha256" ] || fail "$dist/$name.tar.gz.sha256 is missing"
  (cd "$dist" && shasum -a 256 -c "$name.tar.gz.sha256" >&2) ||
    fail "$name.tar.gz does not match $name.tar.gz.sha256"
  mkdir "$stage/$target"
  tar -xzf "$dist/$name.tar.gz" -C "$stage/$target"
  (cd "$stage/$target/$name" && shasum -a 256 -c SHA256SUMS >&2) ||
    fail "the files of $name.tar.gz do not match its SHA256SUMS"
done

# Reads `npm pack --json` on standard input and prints the package's files:
# with `check`, each path, with a * after an executable one; with `print`,
# the tarball's name and size, then each file's mode, size and path.
files() {
  # shellcheck disable=SC2016 # the script is JavaScript, whose ${} are its own
  node -e '
    let text = "";
    process.stdin.on("data", (chunk) => (text += chunk)).on("end", () => {
      const [packed] = JSON.parse(text);
      const files = packed.files.slice().sort((a, b) => (a.path < b.path ? -1 : 1));
      if (process.argv[1] === "check") {
        for (const file of files) console.log(file.path + (file.mode & 0o111 ? " *" : ""));
      } else {
        console.log(`${packed.filename}, ${packed.size} bytes, ${files.length} files:`);
        for (const file of files) {
          console.log(`  ${(file.mode & 0o777).toString(8)} ${String(file.size).padStart(10)} ${file.path}`);
        }
      }
    });' "$1"
}

# Checks that `npm pack --dry-run` of the staged package in $1 lists the
# intended files, the further arguments, then packs it into the output
# directory and prints its list.
pack() {
  local dir=$1 intended listing
  shift
  intended=$(printf '%s\n' "$@" | LC_ALL=C sort)
  listing=$( (cd "$dir" && npm pack --dry-run --json --ignore-scripts 2> /dev/null) | files check | LC_ALL=C sort)
  if [ "$listing" != "$intended" ]; then
    printf 'npm/pack.sh: npm pack --dry-run of %s lists\n%s\nwhere it should list\n%s\n' \
      "${dir##*/}" "$listing" "$intended" >&2
    exit 1
  fi
  (cd "$dir" && npm pack --json --ignore-scripts --pack-destination "$out" 2> /dev/null) | files print
}

# 2. The platform packages: the template, the tarball's binaries and notices, the license, the version.
tarballs=()
for entry in "${platforms[@]}"; do
  platform=${entry%%:*}
  target=${entry#*:}
  from="$stage/$target/mobile-agent-$version-$target"
  dir="$stage/cli-$platform"
  cp -R "$here/cli-$platform" "$dir"
  mkdir "$dir/bin"
  for command in "${commands[@]}"; do
    cp "$from/$command" "$dir/bin/$command"
    chmod 755 "$dir/bin/$command"
  done
  cp "$from/THIRD-PARTY-NOTICES.md" "$root/LICENSE" "$dir/"
  (cd "$dir" && npm pkg set "version=$version")
  pack "$dir" LICENSE README.md THIRD-PARTY-NOTICES.md package.json \
    "bin/mobile-agent *" "bin/mobile-agent-daemon *" "bin/mobile-agent-mcp *"
  tarballs+=("mobileagent-cli-$platform-$version.tgz")
done

# 3. The launcher, its optional dependencies at the same version.
dir="$stage/cli"
cp -R "$here/cli" "$dir"
cp "$root/LICENSE" "$dir/"
dependencies=()
for entry in "${platforms[@]}"; do
  dependencies+=("optionalDependencies.@mobileagent/cli-${entry%%:*}=$version")
done
(cd "$dir" && npm pkg set "version=$version" "${dependencies[@]}")
pack "$dir" LICENSE README.md package.json shim.js \
  "bin/mobile-agent.js *" "bin/mobile-agent-daemon.js *" "bin/mobile-agent-mcp.js *"
tarballs+=("mobileagent-cli-$version.tgz")

# 4. The sums of the four.
cd "$out"
for tarball in "${tarballs[@]}"; do
  [ -f "$tarball" ] || fail "npm pack wrote no $tarball"
done
shasum -a 256 "${tarballs[@]}" > SHA256SUMS
