#!/usr/bin/env bash
# Builds the Expo fixture: an iOS Simulator .app (PLATFORM=ios, the default), an iOS .app for a
# physical iPhone (PLATFORM=ios DESTINATION=device), or an Android APK (PLATFORM=android).
#
# stdout carries exactly one line, the absolute path of the .app or .apk; all build output goes
# to stderr, so `app="$(./build.sh)"` works. Exits non-zero when any step fails, with status 2
# for a PLATFORM, CONFIGURATION or DESTINATION it does not build.
#
# CONFIGURATION=Release (default) embeds the Hermes bundle, so the app runs without Metro.
# CONFIGURATION=Debug loads JavaScript from Metro, which must be running (`bunx expo start`).
#
# DESTINATION=simulator (default) builds for the iOS Simulator without code signing.
# DESTINATION=device builds for a physical iPhone in a copy of the project under build/device/,
# which gets its own node_modules/, ios/, pods and derived data, so the project itself is not
# edited. It signs manually, so xcodebuild registers nothing with the account: the identity, the
# profile, which must include the device and cover dev.mobileagent.fixture.expo, and the team come
# from MOBILE_AGENT_IOS_TEAM (required), MOBILE_AGENT_IOS_IDENTITY (default "Apple Development")
# and MOBILE_AGENT_IOS_PROFILE (default "mobile-agent development"), and the team never enters a
# tracked file. codesign verifies the bundle once it is built. Without MOBILE_AGENT_IOS_TEAM the
# script builds nothing and exits with status 3.
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

destination="${DESTINATION:-simulator}"
case "$platform/$destination" in
  ios/simulator | ios/device | android/simulator) ;;
  android/device)
    echo "build.sh: DESTINATION=device builds for a physical iPhone; PLATFORM=android builds one APK for emulators and devices alike" >&2
    exit 2
    ;;
  *)
    echo "build.sh: DESTINATION must be simulator or device, got '$destination'" >&2
    exit 2
    ;;
esac

if [ "$destination" = device ]; then
  team="${MOBILE_AGENT_IOS_TEAM:-}"
  if [ -z "$team" ]; then
    echo "build.sh: MOBILE_AGENT_IOS_TEAM is not set; a device build is signed for that development team, so none is built" >&2
    exit 3
  fi
  # The project's sources, without what a build generates, copied into build/device/; the
  # generated directories already there are left for the next build to reuse.
  mkdir -p build/device
  rsync -a --delete --exclude /node_modules --exclude /ios --exclude /android --exclude /build \
    --exclude /.expo ./ build/device/ >&2
  cd build/device
fi

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
  # Native projects generated against an older dependency set are stale: their autolinking and
  # pods miss modules added since. Drop the markers so both are regenerated on their next build.
  rm -f ios/.prebuild-inputs android/.prebuild-inputs
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

  if [ "$destination" = device ]; then
    sdk=iphoneos
    options=(
      -destination 'generic/platform=iOS'
      -sdk iphoneos
      CODE_SIGN_STYLE=Manual
      CODE_SIGN_IDENTITY="${MOBILE_AGENT_IOS_IDENTITY:-Apple Development}"
      DEVELOPMENT_TEAM="$team"
      PROVISIONING_PROFILE_SPECIFIER="${MOBILE_AGENT_IOS_PROFILE:-mobile-agent development}"
    )
  else
    sdk=iphonesimulator
    options=(-destination 'generic/platform=iOS Simulator' CODE_SIGNING_ALLOWED=NO)
  fi
  xcodebuild \
    -workspace ios/FixtureExpo.xcworkspace \
    -scheme FixtureExpo \
    -configuration "$configuration" \
    -derivedDataPath build \
    "${options[@]}" \
    build >&2

  artifact="$PWD/build/Build/Products/$configuration-$sdk/FixtureExpo.app"
  if [ "$destination" = device ] && [ -e "$artifact" ]; then
    codesign --verify --strict "$artifact" >&2
  fi
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
