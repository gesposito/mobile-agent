#!/usr/bin/env bash
# Builds the Expo fixture for the iOS Simulator.
#
# stdout carries exactly one line, the absolute .app path; all build output goes to stderr,
# so `app="$(./build.sh)"` works. Exits non-zero when any step fails.
#
# CONFIGURATION=Release (default) embeds the Hermes bundle and launches without Metro.
# CONFIGURATION=Debug loads JavaScript from Metro, which must be running (`bunx expo start`).
set -euo pipefail

cd "$(dirname "$0")"

configuration="${CONFIGURATION:-Release}"
case "$configuration" in
  Release | Debug) ;;
  *)
    echo "build.sh: CONFIGURATION must be Release or Debug, got '$configuration'" >&2
    exit 2
    ;;
esac

require_sfw() {
  if ! command -v sfw >/dev/null 2>&1; then
    echo "build.sh: sfw (Socket Firewall) is required; every package install goes through it" >&2
    exit 1
  fi
}

if [ ! -d node_modules ]; then
  require_sfw
  sfw bun install --frozen-lockfile >&2
fi

if [ ! -d ios ]; then
  require_sfw
  sfw bunx expo prebuild --platform ios --no-install >&2
fi

# pod install runs unwrapped: Socket Firewall cannot wrap CocoaPods (see README).
if [ ! -d ios/Pods ]; then
  (cd ios && pod install) >&2
fi

xcodebuild \
  -workspace ios/FixtureExpo.xcworkspace \
  -scheme FixtureExpo \
  -configuration "$configuration" \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath build \
  CODE_SIGNING_ALLOWED=NO \
  build >&2

app="$PWD/build/Build/Products/$configuration-iphonesimulator/FixtureExpo.app"
if [ ! -d "$app" ]; then
  echo "build.sh: xcodebuild reported success but $app does not exist" >&2
  exit 1
fi
echo "$app"
