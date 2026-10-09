#!/usr/bin/env bash
# Builds and signs the runner for a physical iPhone, without an xcodebuild
# project (design D20):
#
#   dist/MobileAgentRunner-Runner.app       a copy of Xcode's XCTRunner.app
#     MobileAgentRunner-Runner              its executable, renamed and thinned to arm64
#     Info.plist                            dev.mobileagent.runner, a launch screen, VERSION
#     embedded.mobileprovision              the development profile, found by name
#     PlugIns/MobileAgentRunner.xctest/
#       MobileAgentRunner                   shim/Runner.m and the agent's static library,
#                                           one MH_BUNDLE linked by clang -bundle
#       Info.plist                          dev.mobileagent.runner.xctest, VERSION
#
#   1. cargo builds the agent (this crate) for aarch64-apple-ios as a static library.
#   2. The iPhoneOS SDK's clang compiles shim/Runner.m and links it with the
#      library into the test bundle's image.
#   3. The template's executable is thinned to arm64: an A12 or later phone runs
#      the fat template's arm64e slice, which refuses the arm64 test bundle. The
#      template gets a UILaunchScreen entry, without which UIKit gives the runner
#      the 320x480-point compatibility size. No XCTest framework is embedded: the
#      template's rpaths and the test manager's DYLD_FRAMEWORK_PATH load them from
#      the developer disk image at /System/Developer.
#   4. The installed development profile goes in, and codesign signs the test
#      bundle, then the app with the team's entitlements.
#
# The team comes from MOBILE_AGENT_IOS_TEAM and never enters a tracked file: the
# entitlements are written under out/, and the signed bundle, which embeds the
# profile naming the team and its devices, under dist/; git ignores both.
#
# MOBILE_AGENT_IOS_TEAM      the development team (required)
# MOBILE_AGENT_IOS_IDENTITY  the signing identity's name, default "Apple Development"
# MOBILE_AGENT_IOS_PROFILE   the provisioning profile's name, default "mobile-agent development"
#
# Progress goes to stderr; the last line on stdout is the bundle's path.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(dirname "$here")"
name=MobileAgentRunner
runner_id=dev.mobileagent.runner
xctest_id=dev.mobileagent.runner.xctest
deployment=17.0
out="$here/out"
app="$here/dist/$name-Runner.app"
xctest="$app/PlugIns/$name.xctest"

die() {
  printf 'build.sh: %s\n' "$*" >&2
  exit 1
}

log() {
  printf '==> %s\n' "$*" >&2
}

version="$(<"$here/VERSION")"
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.-]+)?$ ]] ||
  die "VERSION must hold one semantic version such as 0.2.0, found '$version'"

team="${MOBILE_AGENT_IOS_TEAM:-}"
[[ -n $team ]] ||
  die "MOBILE_AGENT_IOS_TEAM is not set; export the development team the runner is signed for"
identity="${MOBILE_AGENT_IOS_IDENTITY:-Apple Development}"
profile_name="${MOBILE_AGENT_IOS_PROFILE:-mobile-agent development}"

xcrun --sdk iphoneos --show-sdk-path >/dev/null 2>&1 ||
  die "no iPhoneOS SDK; install Xcode and select it with xcode-select"
developer="$(xcode-select -p)"
platform="$developer/Platforms/iPhoneOS.platform/Developer/Library"
template="$platform/Xcode/Agents/XCTRunner.app"
[[ -d $template ]] || die "no XCTRunner.app template at $template"

security find-identity -v -p codesigning | grep -qF "\"$identity" ||
  die "no valid signing identity named '$identity' in the keychain; create the development certificate in Xcode with the team's account, or set MOBILE_AGENT_IOS_IDENTITY"

# The profile, found by name among the installed ones: Xcode 16 and later keep
# them under UserData, older ones under MobileDevice.
profile=""
for dir in "$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles" \
  "$HOME/Library/MobileDevice/Provisioning Profiles"; do
  for candidate in "$dir"/*.mobileprovision; do
    [[ -f $candidate ]] || continue
    if [[ "$(security cms -D -i "$candidate" 2>/dev/null | plutil -extract Name raw -o - - 2>/dev/null)" == "$profile_name" ]]; then
      profile="$candidate"
      break 2
    fi
  done
done
[[ -n $profile ]] ||
  die "no installed provisioning profile named '$profile_name'; download it in Xcode (Settings > Accounts) or set MOBILE_AGENT_IOS_PROFILE"

mkdir -p "$out" "$(dirname "$app")"

log "cargo: agent-ios $version for aarch64-apple-ios, iOS $deployment"
IPHONEOS_DEPLOYMENT_TARGET="$deployment" cargo build --release --locked \
  --manifest-path "$here/Cargo.toml" -p agent-ios --target aarch64-apple-ios >&2
library="${CARGO_TARGET_DIR:-$root/target}/aarch64-apple-ios/release/libagent_ios.a"
[[ -f $library ]] || die "cargo produced no $library"

log "clang: shim/Runner.m and the agent into one MH_BUNDLE"
cc=(xcrun -sdk iphoneos clang -target "arm64-apple-ios$deployment")
"${cc[@]}" -fobjc-arc -O2 -Wall -F"$platform/Frameworks" -c "$here/shim/Runner.m" -o "$out/Runner.o"
"${cc[@]}" -bundle -o "$out/$name" "$out/Runner.o" "$library" \
  -F"$platform/Frameworks" -framework XCTest -framework UIKit -framework Foundation \
  -framework CoreFoundation -framework CoreGraphics -lobjc -lc++ -lresolv \
  -Wl,-dead_strip \
  -Wl,-rpath,/System/Developer/Library/Frameworks \
  -Wl,-rpath,/System/Developer/Library/PrivateFrameworks \
  -Wl,-rpath,/System/Developer/usr/lib
strip -x "$out/$name"

log "bundle: $(basename "$app") from $(basename "$template"), thinned to arm64"
rm -rf "$app"
cp -R "$template" "$app"
chmod -R u+w "$app"
lipo "$app/XCTRunner" -thin arm64 -output "$app/$name-Runner"
rm "$app/XCTRunner"
rm -f "$app/RunnerEntitlements.plist"
plist="$app/Info.plist"
plutil -replace CFBundleExecutable -string "$name-Runner" "$plist"
plutil -replace CFBundleIdentifier -string "$runner_id" "$plist"
plutil -replace CFBundleName -string "$name-Runner" "$plist"
plutil -replace CFBundleShortVersionString -string "$version" "$plist"
plutil -replace UILaunchScreen -dictionary "$plist"
mkdir -p "$xctest"
cp "$out/$name" "$xctest/$name"
cat >"$xctest/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDevelopmentRegion</key><string>en</string>
  <key>CFBundleExecutable</key><string>$name</string>
  <key>CFBundleIdentifier</key><string>$xctest_id</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleName</key><string>$name</string>
  <key>CFBundlePackageType</key><string>BNDL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleSupportedPlatforms</key><array><string>iPhoneOS</string></array>
  <key>CFBundleVersion</key><string>1</string>
  <key>DTPlatformName</key><string>iphoneos</string>
  <key>MinimumOSVersion</key><string>$deployment</string>
  <key>UIDeviceFamily</key><array><integer>1</integer><integer>2</integer></array>
  <key>XCTContainsUITests</key><true/>
</dict>
</plist>
PLIST

cp "$profile" "$app/embedded.mobileprovision"
cat >"$out/entitlements.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>application-identifier</key><string>$team.$runner_id</string>
  <key>com.apple.developer.team-identifier</key><string>$team</string>
  <key>get-task-allow</key><true/>
</dict>
</plist>
PLIST

log "codesign: the test bundle, then the app, with '$identity' and '$profile_name'"
codesign --force --sign "$identity" --timestamp=none "$xctest" 2>&1 | sed 's/^/    /' >&2
codesign --force --sign "$identity" --timestamp=none --entitlements "$out/entitlements.plist" "$app" 2>&1 | sed 's/^/    /' >&2
codesign --verify --deep --strict "$app" >&2

printf '%s\n' "$app"
