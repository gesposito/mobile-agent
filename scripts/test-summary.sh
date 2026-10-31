#!/usr/bin/env bash
# Prints, as markdown, what a `cargo test --workspace` run did per crate: the
# tests that ran, passed, failed and were ignored, and the seconds their
# binaries took, from each binary's result line, with a total; then the tests
# that failed, each with its crate, and the targets cargo listed as failed,
# which include a binary that crashed before its result line. A run without a
# failure prints the table alone. .github/workflows/device-tests.yml writes it
# to each job's summary.
#
# Usage: scripts/test-summary.sh <test output> <build messages>
#
# <test output> is what the run printed, stdout and stderr together; cargo's
# colors are dropped. <build messages> is what
# `cargo test --workspace --no-run --message-format=json` printed for the same
# build: it names the package of each test binary, which the run's
# "Running tests/<file>.rs" lines do not, and of each library, whose name a
# "Doc-tests" line gives. Needs jq.
set -euo pipefail

if [ "$#" -ne 2 ]; then
  echo "usage: scripts/test-summary.sh <test output> <build messages>" >&2
  exit 2
fi
output=$1
messages=$2

# One line per test binary, "binary <file name> <package>", and per library,
# "library <name> <package>". A package id is a Package ID Spec: the package's
# name is the part after the "#" when a version follows it there, else the
# last segment of the path.
packages=$(jq -r '
  select(.reason == "compiler-artifact")
  | (.package_id | split("#")) as [$url, $fragment]
  | ($fragment // "") as $fragment
  | (if ($fragment | contains("@")) then ($fragment | split("@")[0]) else ($url | split("/") | last) end) as $package
  | (if .executable then "binary \(.executable | split("/") | last) \($package)" else empty end),
    (if (.target.kind | index("lib")) then "library \(.target.name) \($package)" else empty end)
' "$messages")

awk '
  function start(name) {
    crate = name
    if (!(name in seen)) {
      seen[name] = 1
      order[++crates] = name
    }
  }
  FILENAME == ARGV[1] {
    if ($1 == "binary") binary[$2] = $3
    else if ($1 == "library") library[$2] = $3
    next
  }
  { gsub(/\033\[[0-9;]*m/, "") }
  # "     Running tests/devices.rs (target/debug/deps/devices-<hash>)"
  /^ *Running .*\)$/ {
    count = split($NF, parts, "/")
    file = parts[count]
    sub(/\)$/, "", file)
    start((file in binary) ? binary[file] : file)
    listing = 0
    next
  }
  # "   Doc-tests <library>"
  /^ *Doc-tests [^ ]+$/ {
    start(($2 in library) ? library[$2] : $2)
    listing = 0
    next
  }
  # "test result: ok. 2 passed; 0 failed; 1 ignored; 0 measured; 0 filtered out; finished in 0.20s"
  /^test result: / {
    if (crate == "") start("(no crate)")
    for (i = 2; i <= NF; i++) {
      if ($i ~ /^passed;?$/) passed[crate] += $(i - 1)
      else if ($i ~ /^failed;?$/) failed[crate] += $(i - 1)
      else if ($i ~ /^ignored;?$/) ignored[crate] += $(i - 1)
      else if ($i == "in") {
        time = $(i + 1)
        sub(/s$/, "", time)
        seconds[crate] += time
      }
    }
    next
  }
  /^test .* \.\.\. FAILED$/ {
    name = $0
    sub(/^test /, "", name)
    sub(/ \.\.\. FAILED$/, "", name)
    failures[++failed_tests] = "- " crate ": `" name "`"
    next
  }
  # "error: 2 targets failed:", then one indented "`-p <package> --test <name>`" per target
  /^error: [0-9]+ targets? failed:$/ {
    listing = 1
    next
  }
  listing && /^ +`.*`$/ {
    target = $0
    sub(/^ +/, "", target)
    targets[++failed_targets] = "- " target
    next
  }
  { listing = 0 }
  END {
    print "| Crate | Ran | Passed | Failed | Ignored | Seconds |"
    print "|---|---:|---:|---:|---:|---:|"
    for (i = 1; i <= crates; i++) {
      name = order[i]
      p = passed[name] + 0
      f = failed[name] + 0
      g = ignored[name] + 0
      s = seconds[name] + 0
      printf "| %s | %d | %d | %d | %d | %.1f |\n", name, p + f, p, f, g, s
      all_passed += p
      all_failed += f
      all_ignored += g
      all_seconds += s
    }
    printf "| Total | %d | %d | %d | %d | %.1f |\n", all_passed + all_failed, all_passed, all_failed, all_ignored, all_seconds
    if (failed_tests > 0) {
      print ""
      print "Failed tests:"
      print ""
      for (i = 1; i <= failed_tests; i++) print failures[i]
    }
    if (failed_targets > 0) {
      print ""
      print "Targets cargo listed as failed, which include any binary that crashed before its result line:"
      print ""
      for (i = 1; i <= failed_targets; i++) print targets[i]
    }
  }
' <(printf '%s\n' "$packages") "$output"
