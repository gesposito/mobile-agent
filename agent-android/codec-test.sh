#!/usr/bin/env bash
# Golden cross-check of the agent's CBOR codec (src/dev/mobileagent/agent/codec)
# against crates/protocol, without Gradle:
#
#   ./codec-test.sh [--write-golden] [--host-only]
#
#   1. kotlinc compiles the codec and codec-test/*.kt with the flags build.sh
#      uses (-no-jdk against android.jar, so what compiles also links on a
#      device) into out/codec-test/codec-test.jar, Kotlin runtime included.
#   2. d8 dexes that jar and classes.dex is added to it, so the same jar runs
#      on the host JVM and, through app_process, on ART.
#   3. The host JVM runs CodecGoldenTestKt against crates/protocol/golden:
#      every Kotlin sample must encode to the frame the Rust crate wrote, byte
#      for byte, and every Rust frame must decode and encode back to itself.
#      With --write-golden it also writes the Kotlin frames to
#      crates/protocol/golden/kotlin, which `cargo test -p protocol` decodes.
#   4. Unless --host-only, the jar and the golden frames are pushed to a fresh
#      directory under /data/local/tmp on the only connected device, or the one
#      named by ANDROID_SERIAL, and the test runs once more through app_process;
#      the directory is removed afterwards.
#
# Exits non-zero if the build or any check fails.
#
# ANDROID_HOME  Android SDK root, default $HOME/Library/Android/sdk; needs
#               platforms/android-36, build-tools (d8 from the newest) and
#               platform-tools/adb.
# KOTLIN_HOME   Kotlin compiler home, default the install behind kotlinc on PATH.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
golden="$(cd "$here/../crates/protocol/golden" && pwd)"
android_home="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
platform_jar="$android_home/platforms/android-36/android.jar"
adb_bin="$android_home/platform-tools/adb"
min_api=26
jvm_target=17
out="$here/out/codec-test"
jar="$out/codec-test.jar"
main_class=dev.mobileagent.agent.codec.CodecGoldenTestKt
# Enforced on the device, so a hung run fails instead of stalling the script.
device_timeout_s=120

write_golden=false
host_only=false
for arg in "$@"; do
  case $arg in
    --write-golden) write_golden=true ;;
    --host-only) host_only=true ;;
    *)
      printf 'usage: %s [--write-golden] [--host-only]\n' "$0" >&2
      exit 2
      ;;
  esac
done

die() {
  printf 'codec-test.sh: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '==> %s\n' "$*" >&2
}

[[ -f $platform_jar ]] || die "no platform jar at $platform_jar; install platforms;android-36 or set ANDROID_HOME"
[[ -d $android_home/build-tools ]] || die "no build-tools in $android_home"
build_tools="$(ls -1 "$android_home/build-tools" | sort -V | tail -n 1)"
d8="$android_home/build-tools/$build_tools/d8"
[[ -x $d8 ]] || die "no d8 at $d8"

# The Kotlin home is found as build.sh finds it.
if [[ -n ${KOTLIN_HOME:-} ]]; then
  kotlin_home="$KOTLIN_HOME"
else
  kotlinc_path="$(command -v kotlinc)" || die "kotlinc is not on PATH; install Kotlin or set KOTLIN_HOME"
  kotlinc_path="$(readlink -f "$kotlinc_path")"
  if [[ $(basename "$kotlinc_path") != kotlinc ]] && command -v mise >/dev/null; then
    kotlinc_path="$(mise which kotlinc)" || die "mise does not provide kotlinc; set KOTLIN_HOME"
    kotlinc_path="$(readlink -f "$kotlinc_path")"
  fi
  kotlin_home="$(dirname "$(dirname "$kotlinc_path")")"
fi
kotlinc="$kotlin_home/bin/kotlinc"
[[ -x $kotlinc ]] || die "no bin/kotlinc under $kotlin_home; set KOTLIN_HOME"
java_bin="$(command -v java)" || die "java is not on PATH; install a JDK"
jar_tool="$(command -v jar)" || die "jar is not on PATH; install a JDK"

rm -rf "$out"
mkdir -p "$out/dex"

sources=()
while IFS= read -r -d '' file; do
  sources+=("$file")
done < <(find "$here/src/dev/mobileagent/agent/codec" "$here/codec-test" -name '*.kt' -print0 2>/dev/null)
((${#sources[@]} > 0)) || die "no Kotlin sources found"

log "kotlinc: ${#sources[@]} sources, jvm-target $jvm_target, against android-36 (from $kotlin_home)"
"$kotlinc" -jvm-target "$jvm_target" -no-jdk -no-reflect -Werror -include-runtime \
  -classpath "$platform_jar" -d "$jar" "${sources[@]}" ||
  die "kotlinc failed"

log "d8 (build-tools $build_tools): the jar, Kotlin runtime included, min-api $min_api"
if ! "$d8" --release --min-api "$min_api" --lib "$platform_jar" --output "$out/dex" "$jar" \
  >"$out/d8.log" 2>&1; then
  cat "$out/d8.log" >&2
  die "d8 failed"
fi
[[ -f $out/dex/classes.dex && ! -e $out/dex/classes2.dex ]] || die "d8 did not produce a single classes.dex"
"$jar_tool" --update --file "$jar" -C "$out/dex" classes.dex

args=(--golden "$golden")
if $write_golden; then
  rm -rf "$golden/kotlin"
  args+=(--write "$golden/kotlin")
fi

log "host JVM: $("$java_bin" -version 2>&1 | head -n 1)"
status=0
"$java_bin" -cp "$jar" "$main_class" "${args[@]}" | tee "$out/host.log" || status=$?
[[ $status -eq 0 ]] || die "the codec test failed on the host JVM (log: $out/host.log)"
if $write_golden; then
  log "wrote $(find "$golden/kotlin" -name '*.bin' | wc -l | tr -d ' ') Kotlin frames to $golden/kotlin"
fi

if $host_only; then
  echo "PASS: codec golden test on the host JVM"
  exit 0
fi

adb() {
  "$adb_bin" ${ANDROID_SERIAL:+-s "$ANDROID_SERIAL"} "$@"
}

[[ -x $adb_bin ]] || die "adb not found at $adb_bin; set ANDROID_HOME, or pass --host-only"
state="$(adb get-state 2>&1)" || die "no usable device: $state (connect one, choose one with ANDROID_SERIAL, or pass --host-only)"
[[ $state == device ]] || die "device state is '$state', expected 'device'"
serial="$(adb get-serialno)"
grep -qx shell_v2 <<<"$(adb features)" ||
  die "$serial lacks the adb shell v2 protocol, so exit statuses cannot be checked"

remote="/data/local/tmp/mobile-agent-codec-test-$$-$RANDOM"
cleanup() {
  adb shell rm -rf "$remote" >/dev/null 2>&1 || true
}
trap cleanup EXIT

adb shell mkdir -p "$remote/golden" >/dev/null || die "cannot create $remote on $serial"
adb push -q "$jar" "$remote/codec-test.jar" >/dev/null || die "adb push of the jar failed"
adb push -q "$golden"/*.bin "$remote/golden/" >/dev/null || die "adb push of the golden frames failed"

log "device $serial: app_process in $remote"
status=0
adb shell "CLASSPATH=$remote/codec-test.jar timeout $device_timeout_s app_process $remote $main_class --golden $remote/golden" |
  tr -d '\r' | tee "$out/device.log" || status=$?
[[ $status -ne 124 ]] || die "the codec test did not finish on $serial within ${device_timeout_s}s"
[[ $status -eq 0 ]] || die "the codec test failed on $serial (log: $out/device.log)"
echo "PASS: codec golden test on the host JVM and on $serial"
