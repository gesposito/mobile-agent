#!/usr/bin/env bash
# Checks the fixtures' build scripts without Xcode, a device or the network.
# Each check runs copies of the scripts in a scratch directory, with fake
# xcodebuild, codesign, sfw and pod first on PATH: each logs its working
# directory and arguments and creates what the scripts look for afterwards.
#
#   fixtures/test.sh
#
#   1. DESTINATION=device without MOBILE_AGENT_IOS_TEAM: the SwiftUI and Expo
#      scripts exit with status 3, name the variable on stderr, print nothing
#      on stdout and run no tool.
#   2. DESTINATION=device with the team: each runs xcodebuild for
#      generic/platform=iOS with the iphoneos SDK and manual signing for the
#      team, the identity and the profile, verifies the bundle with codesign
#      --verify --strict and prints its path as the only line on stdout. The
#      Expo script installs, prebuilds and installs the pods in its copy of
#      the project under build/device, leaving the project as it was.
#   3. fixtures/build.sh builds each fixture, then the device build of the iOS
#      fixtures: without the team it reports each device build as skipped and
#      exits 0, with the team it builds them.
#   4. An unknown DESTINATION, and DESTINATION=device for Android, exit with
#      status 2.
#   5. The Expo fixture's config plugins' edits of the template's
#      AppDelegate.swift and Info.plist (fixtures/expo/plugins/test/), with
#      Node's test runner; skipped when node is not on PATH.
#
# Prints ok or FAIL for each check and exits 1 when any check failed.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# Physical, as the scripts' own paths are: the SwiftUI script prints its
# bundle's path from `pwd -P`.
scratch="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/fixtures-test.XXXXXX")" && pwd -P)"
trap 'rm -rf "$scratch"' EXIT
bin="$scratch/bin"
team=placeholder-team
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

# The fakes log one line per call: the tool, its working directory and each
# argument, every field followed by `|`, so `|<argument>|` matches one whole
# argument.
mkdir -p "$bin"
log_call='{ printf "%s|%s|" "${0##*/}" "$PWD"; printf "%s|" "$@"; printf "\n"; } >> "$FAKE_LOG"'
cat > "$bin/xcodebuild" <<EOF
#!/usr/bin/env bash
$log_call
# The product the arguments name, as xcodebuild lays it out.
derived= configuration=Debug scheme= sdk=iphonesimulator
while ((\$#)); do
  case \$1 in
    -derivedDataPath) derived=\$2; shift ;;
    -configuration) configuration=\$2; shift ;;
    -scheme) scheme=\$2; shift ;;
    -sdk) sdk=\$2; shift ;;
  esac
  shift
done
app="\$derived/Build/Products/\$configuration-\$sdk/\$scheme.app"
mkdir -p "\$app"
printf '#!/bin/sh\n' > "\$app/\$scheme"
chmod +x "\$app/\$scheme"
EOF
cat > "$bin/codesign" <<EOF
#!/usr/bin/env bash
$log_call
EOF
cat > "$bin/sfw" <<EOF
#!/usr/bin/env bash
$log_call
case "\$1 \$2" in
  "bun install") mkdir -p node_modules ;;
  "bunx expo") mkdir -p "\$5" ;;
esac
EOF
cat > "$bin/pod" <<EOF
#!/usr/bin/env bash
$log_call
mkdir -p Pods
EOF
chmod +x "$bin"/*

# One copy of the build scripts and of the files the Expo script reads, which
# each check links into a directory of its own: a host that inspects every
# new executable before its first run inspects these once.
copies="$scratch/copies"
mkdir -p "$copies/swiftui" "$copies/expo/plugins"
cp "$here/build.sh" "$copies/"
cp "$here/swiftui/build.sh" "$copies/swiftui/"
cp "$here/expo/build.sh" "$here/expo/app.json" "$here/expo/package.json" \
  "$here/expo/bun.lock" "$copies/expo/"
cp "$here"/expo/plugins/*.js "$copies/expo/plugins/"

# fresh <name>: the copies, linked as <scratch>/<name>/fixtures; prints its
# path.
fresh() {
  local root="$scratch/$1/fixtures"
  mkdir -p "$root/swiftui" "$root/expo/plugins"
  ln "$copies/build.sh" "$root/"
  ln "$copies/swiftui/build.sh" "$root/swiftui/"
  ln "$copies"/expo/*.* "$root/expo/"
  ln "$copies"/expo/plugins/*.js "$root/expo/plugins/"
  printf '%s\n' "$root"
}

# run <root> <script> [VARIABLE=value...]: runs <root>/<script> with the fakes
# first on PATH, the variables given and none of the scripts' own variables
# otherwise; leaves the exit status in $status, stdout in <root>/out, stderr
# in <root>/err and the fakes' calls in <root>/calls.
run() {
  local root=$1 script=$2
  shift 2
  status=0
  env -u MOBILE_AGENT_IOS_TEAM -u MOBILE_AGENT_IOS_IDENTITY -u MOBILE_AGENT_IOS_PROFILE \
    -u DESTINATION -u PLATFORM -u CONFIGURATION -u FIXTURES \
    FAKE_LOG="$root/calls" PATH="$bin:$PATH" "$@" "$root/$script" \
    >"$root/out" 2>"$root/err" || status=$?
  touch "$root/calls"
}

status_is() { [[ $status == "$1" ]]; }
calls_none() { [[ ! -s $1/calls ]]; }
# called <root> <tool> <directory> <argument>...: a call of <tool> in
# <directory> had every argument given.
called() {
  local root=$1 tool=$2 directory=$3
  shift 3
  local line
  while IFS= read -r line; do
    [[ $line == "$tool|$directory|"* ]] || continue
    local argument found=1
    for argument in "$@"; do
      [[ $line == *"|$argument|"* ]] || found=0
    done
    ((found)) && return 0
  done <"$root/calls"
  return 1
}
# count <root> <pattern>: how many calls hold <pattern>.
count() { grep -cF -- "$2" "$1/calls" || true; }
stdout_is() { [[ $(cat "$1/out") == "$2" ]]; }
stderr_has() { grep -qF -- "$2" "$1/err"; }
stdout_has() { grep -qF -- "$2" "$1/out"; }
absent() { [[ ! -e $1 ]]; }

signing=(
  "CODE_SIGN_STYLE=Manual"
  "CODE_SIGN_IDENTITY=Apple Development"
  "DEVELOPMENT_TEAM=$team"
  "PROVISIONING_PROFILE_SPECIFIER=mobile-agent development"
)

# 1. Without the team, a device build is skipped before anything runs.
for fixture in swiftui expo; do
  root="$(fresh "no-team-$fixture")"
  run "$root" "$fixture/build.sh" DESTINATION=device
  expect "$fixture: a device build without MOBILE_AGENT_IOS_TEAM exits with status 3" status_is 3
  expect "$fixture: the skip names MOBILE_AGENT_IOS_TEAM on stderr" \
    stderr_has "$root" MOBILE_AGENT_IOS_TEAM
  expect "$fixture: the skip prints nothing on stdout" stdout_is "$root" ""
  expect "$fixture: the skip runs no tool" calls_none "$root"
done
expect "expo: the skip copies nothing into build/" absent "$root/expo/build"

# 2. With the team, each device build is signed for it.
root="$(fresh team-swiftui)"
run "$root" swiftui/build.sh DESTINATION=device MOBILE_AGENT_IOS_TEAM="$team"
app="$root/swiftui/build/Build/Products/Debug-iphoneos/FixtureSwiftUI.app"
expect "swiftui: the device build exits with status 0" status_is 0
expect "swiftui: xcodebuild builds for generic/platform=iOS with the iphoneos SDK, signed for the team" \
  called "$root" xcodebuild "$root/swiftui" -destination generic/platform=iOS -sdk iphoneos \
  -project FixtureSwiftUI.xcodeproj -derivedDataPath build "${signing[@]}"
expect "swiftui: codesign verifies the bundle strictly" \
  called "$root" codesign "$root/swiftui" --verify --strict "$app"
expect "swiftui: stdout is the bundle's path alone" stdout_is "$root" "$app"

root="$(fresh team-expo)"
run "$root" expo/build.sh DESTINATION=device MOBILE_AGENT_IOS_TEAM="$team"
copy="$root/expo/build/device"
app="$copy/build/Build/Products/Release-iphoneos/FixtureExpo.app"
expect "expo: the device build exits with status 0" status_is 0
expect "expo: sfw bun install runs in the copy under build/device" \
  called "$root" sfw "$copy" bun install --frozen-lockfile
expect "expo: sfw bunx expo prebuild runs in the copy" \
  called "$root" sfw "$copy" bunx expo prebuild --platform ios --no-install
expect "expo: pod install runs in the copy's ios/" called "$root" pod "$copy/ios" install
expect "expo: xcodebuild builds the copy's workspace in Release for generic/platform=iOS, signed for the team" \
  called "$root" xcodebuild "$copy" -workspace ios/FixtureExpo.xcworkspace -configuration Release \
  -destination generic/platform=iOS -sdk iphoneos -derivedDataPath build "${signing[@]}"
expect "expo: codesign verifies the bundle strictly" \
  called "$root" codesign "$copy" --verify --strict "$app"
expect "expo: stdout is the bundle's path alone" stdout_is "$root" "$app"
expect "expo: the project itself gets no node_modules/" absent "$root/expo/node_modules"
expect "expo: the project itself gets no ios/" absent "$root/expo/ios"

root="$(fresh identity)"
run "$root" swiftui/build.sh DESTINATION=device MOBILE_AGENT_IOS_TEAM="$team" \
  MOBILE_AGENT_IOS_IDENTITY="Apple Development: placeholder" MOBILE_AGENT_IOS_PROFILE=placeholder-profile
expect "swiftui: MOBILE_AGENT_IOS_IDENTITY and MOBILE_AGENT_IOS_PROFILE name the identity and the profile" \
  called "$root" xcodebuild "$root/swiftui" "CODE_SIGN_IDENTITY=Apple Development: placeholder" \
  PROVISIONING_PROFILE_SPECIFIER=placeholder-profile

# 3. fixtures/build.sh adds the device builds of the iOS fixtures.
root="$(fresh all-no-team)"
run "$root" build.sh FIXTURES=expo,swiftui
expect "build.sh: without the team the run exits with status 0" status_is 0
expect "build.sh: both Simulator builds run" \
  test "$(count "$root" "|generic/platform=iOS Simulator|")" = 2
expect "build.sh: no device build runs" test "$(count "$root" "|-sdk|iphoneos|")" = 0
for fixture in expo swiftui; do
  expect "build.sh: the $fixture device build is reported skipped" \
    stdout_has "$root" "fixtures/$fixture/build.sh: DESTINATION=device needs MOBILE_AGENT_IOS_TEAM, skipping the device build"
done

root="$(fresh all-team)"
run "$root" build.sh FIXTURES=expo,swiftui MOBILE_AGENT_IOS_TEAM="$team"
expect "build.sh: with the team the run exits with status 0" status_is 0
expect "build.sh: both device builds run" test "$(count "$root" "|-sdk|iphoneos|")" = 2
expect "build.sh: stdout lists the two Simulator bundles and the two device bundles" \
  test "$(grep -c '\.app$' "$root/out")" = 4

# 4. Destinations the scripts do not build are refused.
for fixture in swiftui expo; do
  root="$(fresh "refused-$fixture")"
  run "$root" "$fixture/build.sh" DESTINATION=watch MOBILE_AGENT_IOS_TEAM="$team"
  expect "$fixture: DESTINATION=watch exits with status 2" status_is 2
done
root="$(fresh refused-android)"
run "$root" expo/build.sh DESTINATION=device PLATFORM=android MOBILE_AGENT_IOS_TEAM="$team"
expect "expo: DESTINATION=device with PLATFORM=android exits with status 2" status_is 2

# 5. The Expo fixture's config plugins.
# The runner's report is printed when a test failed.
plugin_tests_pass() {
  node --test "$here/expo/plugins/test/plugins.test.js" > "$scratch/plugin-tests" 2>&1 ||
    { cat "$scratch/plugin-tests" >&2; return 1; }
}
if command -v node >/dev/null 2>&1; then
  expect "expo: the config plugins' edits of the iOS project" plugin_tests_pass
else
  printf 'skipped: the Expo config plugins need node on PATH\n'
fi

if ((failures > 0)); then
  printf '%d of %d checks failed\n' "$failures" "$checks" >&2
  exit 1
fi
printf 'all %d checks passed\n' "$checks"
