#!/usr/bin/env bash
# Checks the repository's own scripts in a scratch directory, without the
# network: the reports' scrub (reports/scrub.py).
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
#
# The values below are placeholders, and the shapes the publish scan refuses
# (a machine's model identifier, a sampler's top field) are put together at
# run time, so that this file names none of them.
#
# Prints ok or FAIL for each check and exits 1 when any check failed.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
scratch="$(mktemp -d "${TMPDIR:-/tmp}/scripts-test.XXXXXX")"
trap 'rm -rf "$scratch"' EXIT
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

if [ "$failures" -gt 0 ]; then
  printf '%s of %s checks failed\n' "$failures" "$checks" >&2
  exit 1
fi
printf 'all %s checks passed\n' "$checks"
