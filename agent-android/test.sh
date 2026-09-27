#!/usr/bin/env bash
# On-device smoke test for the agent jar built by build.sh.
#
# Pushes dist/mobile-agent-android.jar to /data/local/tmp, starts it with
# app_process as the shell user and checks the banner contract: `--version` and
# no arguments print exactly one line `mobile-agent-android <VERSION> protocol=<n>`,
# where <VERSION> is the content of the VERSION file, checked on its own first,
# and <n> is Protocol.VERSION in the codec, and exit 0; any other argument
# prints `unknown argument` to stderr and exits 2, and `serve` without a socket
# name prints a usage line and exits 2. It also checks that no agent process
# outlives its run.
#
# Targets the only connected device, or the one named by ANDROID_SERIAL.
# ANDROID_HOME defaults to $HOME/Library/Android/sdk.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
android_home="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
adb_bin="$android_home/platform-tools/adb"
jar="$here/dist/mobile-agent-android.jar"
remote_dir=/data/local/tmp
remote_jar="$remote_dir/mobile-agent-android.jar"
main_class=dev.mobileagent.agent.Main
# The banner's protocol revision is the codec's Protocol.VERSION.
protocol="$(sed -n 's/^ *const val VERSION = \([0-9][0-9]*\)$/\1/p' "$here/src/dev/mobileagent/agent/codec/Protocol.kt")"
# Enforced on the device, so a hung agent fails the test instead of stalling it.
run_timeout_s=30

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

adb() {
  "$adb_bin" ${ANDROID_SERIAL:+-s "$ANDROID_SERIAL"} "$@"
}

[[ -x $adb_bin ]] || fail "adb not found at $adb_bin; set ANDROID_HOME to the Android SDK"
[[ -f $jar ]] || fail "agent jar not found at $jar; run ./build.sh first"

members="$(unzip -Z1 "$jar")" || fail "cannot list the members of $jar"
[[ $members == "classes.dex" ]] ||
  fail "$jar must contain only classes.dex, found: $(tr '\n' ' ' <<<"$members")"
echo "ok: $(basename "$jar") holds only classes.dex"

# The shipped dex is shrunk (task 4.10): without R8 the Kotlin stdlib alone
# dexes to about 2.3 MB.
dex_limit=300000
dex_bytes="$(unzip -Z -l "$jar" classes.dex | awk '$NF == "classes.dex" { print $4 }')"
[[ $dex_bytes =~ ^[0-9]+$ ]] || fail "cannot read the size of classes.dex in $jar"
((dex_bytes < dex_limit)) || fail "classes.dex is $dex_bytes bytes, not under $dex_limit"
echo "ok: classes.dex is $dex_bytes bytes, under $dex_limit"

[[ $protocol =~ ^[0-9]+$ ]] || fail "cannot read Protocol.VERSION from the codec"
# The agent's version is the content of VERSION, which build.sh compiles into
# the banner and the hello; the script names no version of its own.
version="$(<"$here/VERSION")"
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] ||
  fail "VERSION must hold one semantic version, found '$version'"
expected="mobile-agent-android $version protocol=$protocol"

tmp="$(mktemp -d)"
pushed=false
cleanup() {
  rm -rf "$tmp"
  if $pushed; then
    adb shell rm -f "$remote_jar" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

state="$(adb get-state 2>"$tmp/err")" ||
  fail "no usable device: $(<"$tmp/err") (connect one, or choose one with ANDROID_SERIAL)"
[[ $state == device ]] || fail "device state is '$state', expected 'device'"
serial="$(adb get-serialno)"
grep -qx shell_v2 <<<"$(adb features)" ||
  fail "$serial lacks the adb shell v2 protocol, so exit statuses cannot be checked"

pushed=true
push_log="$(adb push "$jar" "$remote_jar" 2>&1)" || fail "adb push to $serial failed: $push_log"
echo "ok: pushed to $serial:$remote_jar"

# Runs the agent on the device with the given arguments. Its stdout and stderr
# land in $tmp/out and $tmp/err; the return status is the agent's exit status,
# which the shell v2 protocol carries back, or 124 when the timeout fired.
run_agent() {
  adb shell "CLASSPATH=$remote_jar timeout $run_timeout_s app_process $remote_dir $main_class $*" \
    >"$tmp/out" 2>"$tmp/err"
}

# expect_banner LABEL ARGS...: the run prints exactly the banner line and exits 0.
# The version in the banner is checked first and on its own, so that a jar
# built from another VERSION is reported as such.
expect_banner() {
  local label=$1 status=0 out lines reported
  shift
  run_agent "$@" || status=$?
  [[ $status -ne 124 ]] || fail "$label: agent still running after ${run_timeout_s}s"
  [[ $status -eq 0 ]] || fail "$label: exit status $status, expected 0; stderr: $(<"$tmp/err")"
  out="$(tr -d '\r' <"$tmp/out")"
  lines="$(wc -l <"$tmp/out" | tr -d ' ')"
  reported="$(sed -n '1s/^mobile-agent-android \([^ ]*\) protocol=[0-9]*$/\1/p' <<<"$out")"
  if [[ -n $reported ]]; then
    [[ $reported == "$version" ]] ||
      fail "$label: the jar reports version $reported but VERSION holds $version; rebuild it with build.sh"
    echo "ok: $label reports version $reported, the content of VERSION"
  fi
  [[ $lines -eq 1 && $out == "$expected" ]] ||
    fail "$label: stdout was '$out' ($lines lines), expected exactly '$expected'"
  echo "ok: $label prints '$out' and exits 0"
}

expect_banner "--version" --version
expect_banner "no arguments"

status=0
run_agent --no-such-option || status=$?
[[ $status -ne 124 ]] || fail "unknown argument: agent still running after ${run_timeout_s}s"
[[ $status -eq 2 ]] || fail "unknown argument: exit status $status, expected 2; stderr: $(<"$tmp/err")"
grep -q 'unknown argument' "$tmp/err" ||
  fail "unknown argument: stderr lacks 'unknown argument': $(<"$tmp/err")"
[[ ! -s $tmp/out ]] || fail "unknown argument: stdout should be empty, got: $(<"$tmp/out")"
echo "ok: an unknown argument prints 'unknown argument' to stderr and exits 2"

status=0
run_agent serve || status=$?
[[ $status -eq 2 ]] || fail "serve without --socket: exit status $status, expected 2; stderr: $(<"$tmp/err")"
grep -q 'usage: ' "$tmp/err" || fail "serve without --socket: stderr lacks a usage line: $(<"$tmp/err")"
[[ ! -s $tmp/out ]] || fail "serve without --socket: stdout should be empty, got: $(<"$tmp/out")"
echo "ok: serve without a socket name prints a usage line and exits 2"

procs="$(adb shell ps -A -o PID,ARGS)"
if leftover="$(grep -F "$main_class" <<<"$procs")"; then
  fail "agent process still running on $serial: $leftover"
fi
echo "ok: no agent process remains on $serial"

echo "PASS: $expected on $serial"
