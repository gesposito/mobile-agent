#!/usr/bin/env bash
# Builds the Expo fixture: an iOS Simulator .app (PLATFORM=ios, the default) or an Android
# APK (PLATFORM=android).
#
# stdout carries exactly one line, the absolute path of the .app or .apk; all build output goes
# to stderr, so `app="$(./build.sh)"` works. Exits non-zero when any step fails.
#
# CONFIGURATION=Release (default) embeds the Hermes bundle, so the app runs without Metro.
# CONFIGURATION=Debug loads JavaScript from Metro, which must be running (`bunx expo start`).
#
# Android needs an SDK holding the packages the README lists, at ANDROID_HOME (default
# ANDROID_SDK_ROOT, then $HOME/Library/Android/sdk), and a JDK 17 or newer through JAVA_HOME or
# on PATH. Gradle may not install SDK packages itself (android.builder.sdkDownload=false), so a
# missing package fails the build.
set -euo pipefail

cd "$(dirname "$0")"

platform="${PLATFORM:-ios}"
case "$platform" in
  ios | android) ;;
  *)
    echo "build.sh: PLATFORM must be ios or android, got '$platform'" >&2
    exit 2
    ;;
esac

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

lock_hash="$(shasum -a 256 bun.lock | cut -d ' ' -f 1)"
if [ ! -d node_modules ] || [ "$(cat node_modules/.bun-lock-hash 2>/dev/null)" != "$lock_hash" ]; then
  require_sfw
  sfw bun install --frozen-lockfile >&2
  echo "$lock_hash" > node_modules/.bun-lock-hash
fi

# The generated native project (ios/ or android/) depends on app.json, the dependencies and the
# local config plugins. It is regenerated whenever they change, because a stale project would
# silently lack a plugin's edits.
prebuild_inputs="$(cat app.json package.json bun.lock plugins/*.js | shasum -a 256 | cut -d ' ' -f 1)"
if [ ! -d "$platform" ] || [ "$(cat "$platform/.prebuild-inputs" 2>/dev/null)" != "$prebuild_inputs" ]; then
  require_sfw
  rm -rf "$platform"
  sfw bunx expo prebuild --platform "$platform" --no-install >&2
  echo "$prebuild_inputs" > "$platform/.prebuild-inputs"
fi

if [ "$platform" = ios ]; then
  # pod install runs unwrapped: Socket Firewall cannot wrap CocoaPods (see README).
  # pod install must follow the JavaScript dependencies: a pod set installed before a new
  # native module was added leaves that module unimplemented at runtime.
  if [ ! -d ios/Pods ] || [ "$(cat ios/Pods/.bun-lock-hash 2>/dev/null)" != "$lock_hash" ]; then
    (cd ios && pod install) >&2
    echo "$lock_hash" > ios/Pods/.bun-lock-hash
  fi

  xcodebuild \
    -workspace ios/FixtureExpo.xcworkspace \
    -scheme FixtureExpo \
    -configuration "$configuration" \
    -destination 'generic/platform=iOS Simulator' \
    -derivedDataPath build \
    CODE_SIGNING_ALLOWED=NO \
    build >&2

  artifact="$PWD/build/Build/Products/$configuration-iphonesimulator/FixtureExpo.app"
else
  export ANDROID_HOME="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Library/Android/sdk}}"
  if [ ! -d "$ANDROID_HOME/platforms" ]; then
    echo "build.sh: no Android SDK at $ANDROID_HOME; set ANDROID_HOME" >&2
    exit 1
  fi
  if [ -z "${JAVA_HOME:-}" ] && ! command -v java >/dev/null 2>&1; then
    echo "build.sh: no JDK found; set JAVA_HOME or put java on PATH" >&2
    exit 1
  fi

  # The Gradle wrapper and the Maven dependencies download unwrapped: Socket Firewall cannot
  # wrap Gradle (see README).
  (cd android && ./gradlew --no-daemon -Pandroid.builder.sdkDownload=false ":app:assemble$configuration") >&2

  variant="$(printf '%s' "$configuration" | tr '[:upper:]' '[:lower:]')"
  artifact="$PWD/android/app/build/outputs/apk/$variant/app-$variant.apk"
fi

if [ ! -e "$artifact" ]; then
  echo "build.sh: the build reported success but $artifact does not exist" >&2
  exit 1
fi
echo "$artifact"
