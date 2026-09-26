#!/usr/bin/env bash
# Builds the SwiftUI fixture for the iOS Simulator without code signing.
# xcodebuild's log goes to stderr; stdout carries only the .app path, printed
# as the last line. Exits non-zero when the build fails.
set -euo pipefail

cd "$(dirname "$0")"
root="$(pwd -P)"

xcodebuild \
  -project FixtureSwiftUI.xcodeproj \
  -scheme FixtureSwiftUI \
  -destination 'generic/platform=iOS Simulator' \
  -derivedDataPath build \
  -configuration Debug \
  CODE_SIGNING_ALLOWED=NO \
  build >&2

app="$root/build/Build/Products/Debug-iphonesimulator/FixtureSwiftUI.app"
if [[ ! -x "$app/FixtureSwiftUI" ]]; then
  echo "build.sh: no app executable at $app" >&2
  exit 1
fi
echo "$app"
