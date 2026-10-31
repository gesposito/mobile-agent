#!/usr/bin/env bash
# Checks the repository's own scripts in a scratch directory, without the
# network: the reports' scrub (reports/scrub.py), the publish scan
# (scripts/check-public.sh), its scan of the history
# (scripts/check-history.sh) and the tests' summary
# (scripts/test-summary.sh).
#
#   scripts/test.sh
#
#   1. A sampler line keeps its loads, its launch time, its busy list and its
#      CPU shares, and loses the process names of its top field, names with
#      digits, spaces and parentheses included.
#   2. A host row and a report's hardware field keep the chip and lose the
#      machine's model identifier.
#   3. The endpoint security product, named in a private terms file and shown
#      to be the product by the reports themselves, becomes "the endpoint
#      security software" in prose and leaves the host rows' parenthesis.
#   4. A private term the reports do not show to be the product is refused
#      with the file, the line and the term's line in the list, never the
#      value; --check changes nothing and exits 1 when a scrub is due, 0 once
#      it is done.
#   5. The publish scan passes a repository of clean staged files and refuses
#      one value per pattern planted in a staged file, naming the file and the
#      pattern: machine paths, iOS identifiers and their prefix, Android
#      serials, Apple team ids, emails and Mac model identifiers, and a file
#      whose name holds one; placeholders for a team, a serial and an emulator
#      pass, as do GitHub's own support and noreply addresses.
#      An author's address in the third-party notices and their template
#      passes.
#   6. It refuses a value of the private terms file in any case, naming the
#      term by its line in the list and never printing it, except in
#      LICENSE's copyright line and in CODEOWNERS' owners; without the file it
#      says so and checks the patterns alone, and a list it is pointed at that
#      does not exist is an error.
#   7. With gitleaks installed, it refuses a credential gitleaks knows, and
#      .gitleaks.toml's placeholder token passes in its own file only.
#   8. The history scan (scripts/check-history.sh) passes a clean history and
#      refuses, in commits the tip no longer holds, an identifier, naming the
#      blob's path, the pattern and the commit, a private term by its line in
#      the list, a machine path in a commit message, an identifier in a file's
#      name, and, with gitleaks installed, a credential.
#   9. The tests' summary counts a workspace's tests per crate, the crate of
#      each test binary taken from cargo's JSON for the build and that of
#      each doc-test run from its library's name, through cargo's colors:
#      the tests that ran, passed, failed and were ignored and the seconds
#      the binaries took, with a total; then the tests that failed, each with
#      its crate, and the targets cargo listed as failed, which include a
#      binary that crashed before its result line; a run without a failure
#      lists neither.
#
# The values below are placeholders, and the shapes the publish scan refuses
# (a machine's model identifier, a sampler's top field, a device identifier,
# an email) are put together at run time, so that this file names none of
# them.
#
# Prints ok or FAIL for each check and exits 1 when any check failed.
#
# The single-quoted dollar signs below are literal on purpose: texts written
# into files, and scripts that bash -c expands itself.
# shellcheck disable=SC2016
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/scripts-test.XXXXXX")"
trap 'rm -rf "${scratch:?}"' EXIT
checks=0
failures=0

pass() {
  checks=$((checks + 1))
  printf 'ok: %s\n' "$1"
}

fail() {
  checks=$((checks + 1))
  failures=$((failures + 1))
  printf 'FAIL: %s\n' "$1" >&2
}

# expect <description> <command...>: one check, passed when the command succeeds.
expect() {
  local description=$1
  shift
  if "$@"; then pass "$description"; else fail "$description"; fi
}

# same <file> <expected text>: the file holds exactly the text.
same() {
  local got
  got="$(cat "$1")"
  if [ "$got" = "$2" ]; then return 0; fi
  printf 'expected:\n%s\ngot:\n%s\n' "$2" "$got" >&2
  return 1
}

scrub() {
  python3 "$root/reports/scrub.py" "$@"
}

open='['
top="top=$open"
model="Mac"
model+="99,9"

# 1. Samplers.
mkdir -p "$scratch/reports/run"
cat > "$scratch/reports/run/loads.txt" <<EOF
# run: before
2026-10-11T09:00:10Z load1=1.95 load5=3.44 load15=5.53 launch_ms=6.1 busy=[] ${top}placeholder-daemon 34.7%, placeholder scanner 31.0%, Python 10.0%]
2026-10-11T09:00:30Z load1=4.75 load5=3.48 load15=5.14 launch_ms=4.2 busy=[xcodebuild 7.9%] ${top}next-server (v16.3.1) 9.5%, placeholder_cache-64276d258d99749f 6.8%, placeholder: db test ::1(56321) idle 6.1%]
2026-10-11T09:00:51Z load1=2.18 load5=3.29 load15=5.38 launch_ms=4.9 busy=[] ${top}placeholder-daemon 98.9%]
EOF
: > "$scratch/terms.txt"
expect "the scrub of a sampler succeeds" \
  scrub --terms "$scratch/terms.txt" "$scratch/reports/run/loads.txt"
expect "a sampler line keeps the loads, launch time, busy list and CPU shares without the process names" \
  same "$scratch/reports/run/loads.txt" "# run: before
2026-10-11T09:00:10Z load1=1.95 load5=3.44 load15=5.53 launch_ms=6.1 busy=[] ${top}34.7%, 31.0%, 10.0%]
2026-10-11T09:00:30Z load1=4.75 load5=3.48 load15=5.14 launch_ms=4.2 busy=[xcodebuild 7.9%] ${top}9.5%, 6.8%, 6.1%]
2026-10-11T09:00:51Z load1=2.18 load5=3.29 load15=5.38 launch_ms=4.9 busy=[] ${top}98.9%]"

# 2. The host's model.
cat > "$scratch/reports/run.md" <<EOF
| Host | Apple M1 Max ($model), 10 cores (8 performance, 2 efficiency), 64 GB, macOS 26.6.2 (25G83). |
EOF
cat > "$scratch/reports/run/report.json" <<EOF
{"host": {"hardware": "Apple M1 Max ($model)", "cpu_cores": 10}}
EOF
cat > "$scratch/reports/run/report.md" <<EOF
| Host hardware | Apple M1 Max ($model) |
EOF
expect "the scrub of a host's rows succeeds" \
  scrub --terms "$scratch/terms.txt" "$scratch/reports/run.md" "$scratch/reports/run/report.json" \
  "$scratch/reports/run/report.md"
expect "a host row keeps the chip, cores, memory and macOS version without the model" \
  same "$scratch/reports/run.md" "| Host | Apple M1 Max, 10 cores (8 performance, 2 efficiency), 64 GB, macOS 26.6.2 (25G83). |"
expect "a report's hardware field is the chip" \
  same "$scratch/reports/run/report.json" '{"host": {"hardware": "Apple M1 Max", "cpu_cores": 10}}'
expect "a report's hardware row is the chip" \
  same "$scratch/reports/run/report.md" "| Host hardware | Apple M1 Max |"

# 3. The endpoint security product, from the private terms.
printf '# placeholder terms\nplaceholder-name\nPlaceholder Shield\n' > "$scratch/terms.txt"
cat > "$scratch/reports/prose.md" <<'EOF'
| Host | Apple M1 Max, 64 GB. An endpoint security extension (Placeholder Shield) inspects process starts. |
- Placeholder Shield scanning each new process, above all as each run started.
The busiest were the emulator, then Placeholder Shield's scanner, and the daemon.
The busiest were the fixture, the Simulator's processes and Placeholder Shield. No build ran.
The host's endpoint security (Placeholder Shield) inspects process starts.
EOF
expect "--check exits 1 while a scrub is due" \
  bash -c '! python3 "$1/reports/scrub.py" --check --terms "$2/terms.txt" "$2/reports/prose.md" > /dev/null 2>&1' _ "$root" "$scratch"
expect "--check writes nothing" \
  grep -q 'Placeholder Shield' "$scratch/reports/prose.md"
expect "the scrub of the product's prose succeeds" \
  scrub --terms "$scratch/terms.txt" "$scratch/reports/prose.md"
expect "the product becomes the endpoint security software in prose" \
  same "$scratch/reports/prose.md" "| Host | Apple M1 Max, 64 GB. An endpoint security extension inspects process starts. |
- The endpoint security software scanning each new process, above all as each run started.
The busiest were the emulator, then the endpoint security software's scanner, and the daemon.
The busiest were the fixture, the Simulator's processes and the endpoint security software. No build ran.
The host's endpoint security inspects process starts."
expect "--check exits 0 once the scrub is done" \
  scrub --check --terms "$scratch/terms.txt" "$scratch/reports/prose.md"

# 4. A private term that is not the product.
printf 'Written by placeholder-name on the host.\n' > "$scratch/reports/other.md"
status=0
scrub --terms "$scratch/terms.txt" "$scratch/reports/prose.md" "$scratch/reports/other.md" \
  > "$scratch/out.txt" 2>&1 || status=$?
expect "a private term that is not the product is refused" test "$status" = 1
expect "the refusal names the file, its line and the term's line in the list" \
  grep -q 'other.md:1: private term on line 2 of' "$scratch/out.txt"
expect "the refusal never prints the value" \
  bash -c '! grep -q placeholder-name "$1"' _ "$scratch/out.txt"
expect "the refused file is left as it was" \
  same "$scratch/reports/other.md" "Written by placeholder-name on the host."

# 5. The publish scan, in a repository of its own.
repo="$scratch/repo"
mkdir -p "$repo/scripts"
cp "$root/scripts/check-public.sh" "$root/scripts/public-patterns.sh" "$repo/scripts/"
cp "$root/.gitleaks.toml" "$repo/"
git -C "$repo" init -q
printf 'Build with `cargo build`; write $HOME, $TMPDIR or <repo> instead of a path.\n' > "$repo/notes.md"
git -C "$repo" add -A
mkdir -p "$scratch/home"
# scan [VAR=value...]: the scan in the repository, with HOME holding no
# private terms file, its output in out.txt and its status in $status.
scan() {
  status=0
  (cd "$repo" && env HOME="$scratch/home" MOBILE_AGENT_PRIVATE_TERMS= "$@" scripts/check-public.sh) \
    > "$scratch/out.txt" 2>&1 || status=$?
}
# unstage <file...>: the files out of the index and the checkout.
unstage() {
  local file
  for file in "$@"; do
    git -C "$repo" rm -q -f --cached -- "$file"
    rm -f "${repo:?}/${file:?}"
  done
}
# planted <description> <pattern name> <text> [file]: the text staged in a
# file is refused, the output naming the file, the line and the pattern.
planted() {
  local description=$1 name=$2 text=$3 file=${4:-planted.md}
  mkdir -p "$(dirname "$repo/$file")"
  printf 'Before the value.\n%s\n' "$text" > "$repo/$file"
  git -C "$repo" add -- "$file"
  scan
  if [ "$status" = 1 ] && grep -q -F "$file:2: $name" "$scratch/out.txt"; then
    pass "$description"
  else
    fail "$description"
    cat "$scratch/out.txt" >&2
  fi
  unstage "$file"
}

scan
expect "the scan passes clean staged files" test "$status" = 0
expect "without a private terms file the scan says it checks the patterns alone" \
  grep -q 'no private terms file' "$scratch/out.txt"

users="/Users/"
homes="/home/"
claude="/.claude"
tmp_claude="/tmp/claude-"
folders="/var/"
worktrees="mobile-agent"
hex16="01234567"
hex16+="89ABCDEF"
prefix="0000"
prefix+="9ABC"
android_id="0a1b2c3d"
android_id+="4e5f6789"
team="AB12CD"
team+="34EF"
identity="ZX98YW"
identity+="76VU"
planted "a macOS home directory is refused" "a macOS home directory" "see ${users}placeholderuser/code/app"
planted "a Linux home directory is refused" "a Linux home directory" "see ${homes}placeholderuser/code/app"
planted "Claude Code's directory under the home is refused" "Claude Code's per-user directory" "see ~${claude}/settings.json"
planted "a Claude Code scratch directory is refused" "a Claude Code scratch directory" "under ${tmp_claude}1234/session"
planted "the macOS per-user temporary directory is refused" "the macOS per-user temporary directory" "under ${folders}folders/xy/abc/T/"
planted "the worktrees' directory is refused" "the directory of this repository's worktrees" "in ${worktrees}-wt/branch"
planted "an iOS device identifier is refused" "an iOS device identifier" "the phone ${prefix}-${hex16}"
planted "an iOS device identifier's prefix is refused" "an iOS device identifier's prefix" "identifier prefix ${prefix}"
planted "an Android serial after adb -s is refused" "an Android device serial" "adb -s ${android_id} shell getprop"
planted "an Android serial in a devices listing is refused" "an Android device serial" \
  "${android_id}  android  android-device  Placeholder  device"
planted "an Apple team id is refused" "an Apple team id" "MOBILE_AGENT_IOS_TEAM=${team}"
planted "a signing identity's team id is refused" "an Apple team id" "Apple Development: placeholder (${identity})"
at="@"
planted "an email address is refused" "an email address" "write to someone${at}placeholder-mail.com"
printf 'Signed-off-by: placeholder[bot] <support%sgithub.com>, from noreply%sgithub.com\n' "$at" "$at" \
  > "$repo/github.md"
git -C "$repo" add github.md
scan
expect "GitHub's own support and noreply addresses pass" test "$status" = 0
unstage github.md
model="Mac"
model+="BookPro99,9"
planted "a Mac model identifier is refused" "a Mac model identifier" "measured on a ${model}"

mkdir -p "$repo/probes"
printf 'nothing else\n' > "$repo/probes/${prefix}-${hex16}.txt"
git -C "$repo" add probes
scan
expect "a file whose name holds a device identifier is refused" \
  grep -q -F "probes/${prefix}-${hex16}.txt: an iOS device identifier in the file's name" "$scratch/out.txt"
unstage "probes/${prefix}-${hex16}.txt"

printf 'MOBILE_AGENT_IOS_TEAM=TEAMPLACE1 and (PLACEHOLD0), emulator-5554, adb -s "$serial", NEXUS5X-SERIAL\n' \
  > "$repo/placeholders.md"
git -C "$repo" add placeholders.md
scan
expect "placeholders for a team, a serial and an emulator pass" test "$status" = 0
unstage placeholders.md

# Third parties' notices keep their authors' addresses as they wrote them.
printf 'Copyright (C) 2018 Placeholder <author%splaceholder-mail.com>\n' "$at" > "$repo/THIRD-PARTY-NOTICES.md"
printf 'Copyright (C) 2018 Placeholder <author%splaceholder-mail.com>\n' "$at" > "$repo/about.hbs"
git -C "$repo" add THIRD-PARTY-NOTICES.md about.hbs
scan
expect "an author's address in the third-party notices and their template passes" test "$status" = 0
unstage THIRD-PARTY-NOTICES.md about.hbs

# 6. The private terms.
printf '# placeholder terms\nplaceholder-private-value\n' > "$scratch/private-terms.txt"
printf 'Written by Placeholder-Private-Value.\n' > "$repo/planted.md"
git -C "$repo" add planted.md
scan MOBILE_AGENT_PRIVATE_TERMS="$scratch/private-terms.txt"
expect "a private term is refused in any case" test "$status" = 1
expect "the refusal names the term by its line in the list" \
  grep -q -F 'planted.md:1: private term on line 2 of' "$scratch/out.txt"
expect "the refusal never prints the term" \
  bash -c '! grep -q -i placeholder-private-value "$1"' _ "$scratch/out.txt"
scan
expect "without the list the same file passes" test "$status" = 0
unstage planted.md
scan MOBILE_AGENT_PRIVATE_TERMS="$scratch/missing.txt"
expect "a private terms file named but missing is an error" test "$status" = 2

mkdir -p "$repo/.github"
printf 'MIT License\n\nCopyright (c) 2026 placeholder-private-value\n' > "$repo/LICENSE"
printf '* @placeholder-private-value\n' > "$repo/.github/CODEOWNERS"
git -C "$repo" add LICENSE .github/CODEOWNERS
scan MOBILE_AGENT_PRIVATE_TERMS="$scratch/private-terms.txt"
expect "LICENSE's copyright line and CODEOWNERS' owners may hold a private term" test "$status" = 0
printf 'MIT License\n\nCopyright (c) 2026 placeholder-private-value\nKept by placeholder-private-value.\n' \
  > "$repo/LICENSE"
printf '* @placeholder-private-value\n# placeholder-private-value\n' > "$repo/.github/CODEOWNERS"
scan MOBILE_AGENT_PRIVATE_TERMS="$scratch/private-terms.txt"
expect "LICENSE's other lines may not" grep -q -F 'LICENSE:4: private term' "$scratch/out.txt"
expect "CODEOWNERS' comments may not" grep -q -F '.github/CODEOWNERS:2: private term' "$scratch/out.txt"
unstage LICENSE .github/CODEOWNERS

# 7. gitleaks, when installed.
if command -v gitleaks > /dev/null 2>&1; then
  key="AKIA"
  key+="QYLPMN5HGT7RZ4WX"
  printf 'aws_access_key_id = %s\n' "$key" > "$repo/planted.md"
  git -C "$repo" add planted.md
  scan
  expect "a credential gitleaks knows is refused" grep -q 'gitleaks: aws-access-token at planted.md' "$scratch/out.txt"
  unstage planted.md
  token='token: "01234567'
  token+='89abcdef"'
  mkdir -p "$repo/crates/android/src/agent"
  printf '%s\n' "$token" > "$repo/crates/android/src/agent/records.rs"
  git -C "$repo" add crates/android/src/agent/records.rs
  scan
  expect "the placeholder token passes in the records' tests" test "$status" = 0
  printf '%s\n' "$token" > "$repo/elsewhere.rs"
  git -C "$repo" add elsewhere.rs
  scan
  expect "the placeholder token elsewhere is refused" \
    grep -q 'gitleaks: generic-api-key at elsewhere.rs' "$scratch/out.txt"
  unstage crates/android/src/agent/records.rs elsewhere.rs
else
  printf 'skip: gitleaks is not installed\n'
fi

# 8. The history scan, over the commits of a repository of its own.
history="$scratch/history"
mkdir -p "$history/scripts"
cp "$root/scripts/check-history.sh" "$root/scripts/public-patterns.sh" "$history/scripts/"
cp "$root/.gitleaks.toml" "$history/"
git -C "$history" init -q
commit() {
  git -C "$history" add -A
  git -C "$history" -c user.name=placeholder -c user.email=placeholder@example.com \
    commit -q -m "$1"
}
# history_scan [VAR=value...]: the scan of every commit, its output in
# out.txt and its status in $status.
history_scan() {
  status=0
  (cd "$history" && env HOME="$scratch/home" MOBILE_AGENT_PRIVATE_TERMS= "$@" scripts/check-history.sh) \
    > "$scratch/out.txt" 2>&1 || status=$?
}
printf 'Build with `cargo build`.\n' > "$history/notes.md"
commit "docs: the first note"
history_scan
expect "the history scan passes a clean history" test "$status" = 0
expect "the history scan counts the blobs and commits it read" \
  grep -q 'check-history: 1 commit' "$scratch/out.txt"

printf 'the phone %s-%s\n' "$prefix" "$hex16" > "$history/probe.txt"
commit "docs: a probe"
rm -f "${history:?}/probe.txt"
commit "docs: the probe without its identifier"
history_scan
expect "an identifier in a commit the tip no longer holds is refused" test "$status" = 1
expect "the refusal names the blob's path and the pattern" \
  grep -q -F "probe.txt:1: an iOS device identifier" "$scratch/out.txt"
expect "the refusal names the commit that brought the blob" \
  grep -q -F "$(git -C "$history" rev-parse --short HEAD~1)" "$scratch/out.txt"

git -C "$history" reset -q --hard HEAD~2
printf 'Written by Placeholder-Private-Value.\n' > "$history/old.md"
commit "docs: an old line"
printf 'Written by the maintainer.\n' > "$history/old.md"
commit "docs: the line without the name"
history_scan MOBILE_AGENT_PRIVATE_TERMS="$scratch/private-terms.txt"
expect "a private term in an old blob is refused by its line in the list" \
  grep -q -F 'old.md:1: private term on line 2 of' "$scratch/out.txt"
expect "the history scan never prints the term" \
  bash -c '! grep -q -i placeholder-private-value "$1"' _ "$scratch/out.txt"

git -C "$history" reset -q --hard HEAD~2
printf 'more\n' >> "$history/notes.md"
commit "docs: notes from ${users}placeholderuser/code"
history_scan
expect "a machine path in a commit message is refused" \
  grep -q -E "^[0-9a-f]{7,}: a macOS home directory in the message" "$scratch/out.txt"

git -C "$history" reset -q --hard HEAD~1
printf 'a flow\n' > "$history/flow-$prefix-$hex16.yaml"
commit "docs: a flow"
git -C "$history" mv "flow-$prefix-$hex16.yaml" flow-placeholder.yaml
commit "docs: the flow renamed"
history_scan
expect "an identifier in a file name the tip no longer has is refused" \
  grep -q -F "flow-$prefix-$hex16.yaml: an iOS device identifier in the file's name" "$scratch/out.txt"

git -C "$history" reset -q --hard HEAD~2
if command -v gitleaks > /dev/null 2>&1; then
  printf 'aws_access_key_id = %s\n' "$key" > "$history/creds.md"
  commit "docs: a key"
  rm -f "${history:?}/creds.md"
  commit "docs: no key"
  history_scan
  expect "a credential gitleaks knows in an old commit is refused" \
    grep -q 'gitleaks: aws-access-token at creds.md' "$scratch/out.txt"
fi

# 9. The tests' summary.
esc=$(printf '\033')
summary_dir="$scratch/summary"
mkdir -p "$summary_dir"
deps=/work/placeholder-repo/target/debug/deps
{
  printf '{"reason":"compiler-artifact","package_id":"path+file:///work/placeholder-repo/crates/core#0.1.0","target":{"kind":["lib"],"name":"core"},"executable":null}\n'
  printf '{"reason":"compiler-artifact","package_id":"path+file:///work/placeholder-repo/crates/core#0.1.0","target":{"kind":["lib"],"name":"core"},"executable":"%s/core-0123456789abcdef"}\n' "$deps"
  printf '{"reason":"compiler-artifact","package_id":"path+file:///work/placeholder-repo/crates/core#0.1.0","target":{"kind":["test"],"name":"devices"},"executable":"%s/devices-1111111111111111"}\n' "$deps"
  printf '{"reason":"compiler-artifact","package_id":"path+file:///work/placeholder-repo/crates/core/reader#placeholder-reader@0.1.0","target":{"kind":["lib"],"name":"placeholder_reader"},"executable":"%s/placeholder_reader-2222222222222222"}\n' "$deps"
  printf '{"reason":"compiler-artifact","package_id":"path+file:///work/placeholder-repo/crates/cli#0.1.0","target":{"kind":["bin"],"name":"placeholder-cli"},"executable":"%s/placeholder_cli-3333333333333333"}\n' "$deps"
  printf '{"reason":"compiler-artifact","package_id":"path+file:///work/placeholder-repo/crates/cli#0.1.0","target":{"kind":["test"],"name":"crashed"},"executable":"%s/crashed-4444444444444444"}\n' "$deps"
  printf '{"reason":"build-finished","success":true}\n'
} > "$summary_dir/binaries.json"
cat > "$summary_dir/tests.log" <<EOF
${esc}[1m${esc}[92m   Compiling${esc}[0m core v0.1.0 (/work/placeholder-repo/crates/core)
${esc}[1m${esc}[92m    Finished${esc}[0m \`test\` profile [unoptimized + debuginfo] target(s) in 1m 02s
${esc}[1m${esc}[92m     Running${esc}[0m unittests src/lib.rs (target/debug/deps/core-0123456789abcdef)

running 3 tests
test model::a_test ... ok
test model::an_ignored_test ... ignored, a measurement
test model::another_test ... ok

test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.20s

${esc}[1m${esc}[92m     Running${esc}[0m tests/devices.rs (target/debug/deps/devices-1111111111111111)

running 3 tests
test a_test_on_the_device ... ok
test a_test_that_failed ... FAILED
test another_test_on_the_device ... ok

failures:

---- a_test_that_failed stdout ----

thread 'a_test_that_failed' panicked at crates/core/tests/devices.rs:10:5:
the device answered nothing

failures:
    a_test_that_failed

test result: FAILED. 2 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 61.50s

${esc}[1m${esc}[91merror${esc}[0m: test failed, to rerun pass \`-p core --test devices\`
${esc}[1m${esc}[92m     Running${esc}[0m unittests src/lib.rs (target/debug/deps/placeholder_reader-2222222222222222)

running 1 test
test reads ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.11s

${esc}[1m${esc}[92m     Running${esc}[0m unittests src/main.rs (target/debug/deps/placeholder_cli-3333333333333333)

running 4 tests
test args::one ... ok
test args::two ... ok
test args::three ... ok
test args::four ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s

${esc}[1m${esc}[92m     Running${esc}[0m tests/crashed.rs (target/debug/deps/crashed-4444444444444444)

running 2 tests
${esc}[1m${esc}[91merror${esc}[0m: test failed, to rerun pass \`-p cli --test crashed\`

Caused by:
  process didn't exit successfully: \`target/debug/deps/crashed-4444444444444444\` (signal: 6, SIGABRT: process abort signal)
${esc}[1m${esc}[92m   Doc-tests${esc}[0m core

running 2 tests
test crates/core/src/lib.rs - model (line 3) ... ok
test crates/core/src/lib.rs - model::Thing (line 9) ... ignored

test result: ok. 1 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.60s

${esc}[1m${esc}[92m   Doc-tests${esc}[0m placeholder_reader

running 0 tests

test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s

${esc}[1m${esc}[91merror${esc}[0m: 2 targets failed:
    \`-p core --test devices\`
    \`-p cli --test crashed\`
EOF
status=0
"$root/scripts/test-summary.sh" "$summary_dir/tests.log" "$summary_dir/binaries.json" \
  > "$summary_dir/summary.md" 2> "$scratch/out.txt" || status=$?
expect "the tests' summary of a run with failures succeeds" test "$status" = 0
expect "the summary counts each crate's tests and seconds, then lists the failed tests and targets" \
  same "$summary_dir/summary.md" '| Crate | Ran | Passed | Failed | Ignored | Seconds |
|---|---:|---:|---:|---:|---:|
| core | 6 | 5 | 1 | 2 | 62.3 |
| placeholder-reader | 1 | 1 | 0 | 0 | 0.1 |
| cli | 4 | 4 | 0 | 0 | 0.1 |
| Total | 11 | 10 | 1 | 2 | 62.5 |

Failed tests:

- core: `a_test_that_failed`

Targets cargo listed as failed, which include any binary that crashed before its result line:

- `-p core --test devices`
- `-p cli --test crashed`'
cat > "$summary_dir/green.log" <<EOF
${esc}[1m${esc}[92m     Running${esc}[0m unittests src/lib.rs (target/debug/deps/core-0123456789abcdef)

running 1 test
test model::a_test ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.30s

EOF
"$root/scripts/test-summary.sh" "$summary_dir/green.log" "$summary_dir/binaries.json" \
  > "$summary_dir/green.md" 2> "$scratch/out.txt" || true
expect "a run without a failure lists no failed test or target" \
  same "$summary_dir/green.md" '| Crate | Ran | Passed | Failed | Ignored | Seconds |
|---|---:|---:|---:|---:|---:|
| core | 1 | 1 | 0 | 0 | 0.3 |
| Total | 1 | 1 | 0 | 0 | 0.3 |'

if [ "$failures" -gt 0 ]; then
  printf '%s of %s checks failed\n' "$failures" "$checks" >&2
  exit 1
fi
printf 'all %s checks passed\n' "$checks"
