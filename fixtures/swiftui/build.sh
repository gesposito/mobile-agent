#!/usr/bin/env bash
# Builds the SwiftUI fixture's Debug configuration into build/: for the iOS
# Simulator without code signing (DESTINATION=simulator, the default), or for
# a physical iPhone (DESTINATION=device), signed for a development team.
#
# A device build signs manually, so xcodebuild registers nothing with the
# account: the identity, the profile, which must include the device and cover
# dev.mobileagent.fixture.swiftui, and the team come from
#   MOBILE_AGENT_IOS_TEAM      the development team (required)
#   MOBILE_AGENT_IOS_IDENTITY  the signing identity's name, default "Apple Development"
#   MOBILE_AGENT_IOS_PROFILE   the provisioning profile's name, default "mobile-agent development"
# and the team never enters a tracked file. codesign verifies the bundle once
# it is built. Without MOBILE_AGENT_IOS_TEAM the script builds nothing and
# exits with status 3.
#
# xcodebuild's log goes to stderr; stdout carries only the .app path, printed
# as the last line. Exits non-zero when the build fails, with status 2 for a
# DESTINATION other than simulator or device.
set -euo pipefail

cd "$(dirname "$0")"
root="$(pwd -P)"

destination="${DESTINATION:-simulator}"
case "$destination" in
  simulator)
    platform='generic/platform=iOS Simulator'
    sdk=iphonesimulator
    options=(CODE_SIGNING_ALLOWED=NO)
    ;;
  device)
    team="${MOBILE_AGENT_IOS_TEAM:-}"
    if [[ -z $team ]]; then
      echo "build.sh: MOBILE_AGENT_IOS_TEAM is not set; a device build is signed for that development team, so none is built" >&2
      exit 3
    fi
    platform='generic/platform=iOS'
    sdk=iphoneos
    options=(
      -sdk iphoneos
      CODE_SIGN_STYLE=Manual
      CODE_SIGN_IDENTITY="${MOBILE_AGENT_IOS_IDENTITY:-Apple Development}"
      DEVELOPMENT_TEAM="$team"
      PROVISIONING_PROFILE_SPECIFIER="${MOBILE_AGENT_IOS_PROFILE:-mobile-agent development}"
    )
    ;;
  *)
    echo "build.sh: DESTINATION must be simulator or device, got '$destination'" >&2
    exit 2
    ;;
esac

xcodebuild \
  -project FixtureSwiftUI.xcodeproj \
  -scheme FixtureSwiftUI \
  -destination "$platform" \
  -derivedDataPath build \
  -configuration Debug \
  "${options[@]}" \
  build >&2

app="$root/build/Build/Products/Debug-$sdk/FixtureSwiftUI.app"
if [[ ! -x "$app/FixtureSwiftUI" ]]; then
  echo "build.sh: no app executable at $app" >&2
  exit 1
fi
if [[ $destination == device ]]; then
  codesign --verify --strict "$app" >&2
fi
echo "$app"
