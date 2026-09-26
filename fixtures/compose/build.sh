#!/usr/bin/env bash
# Builds the Compose fixture's debug APK with the Gradle wrapper:
# ./gradlew --no-daemon :app:assembleDebug.
#
# Gradle's output goes to stderr. The last line on stdout is the APK path.
#
# ANDROID_HOME  Android SDK root, default $HOME/Library/Android/sdk. Needs
#               platforms/android-36 and build-tools 36.0.0.
# JAVA_HOME     JDK 17 or later that runs Gradle, default the JDK behind java on PATH.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
apk="$here/app/build/outputs/apk/debug/app-debug.apk"

die() {
  printf 'build.sh: %s\n' "$*" >&2
  exit 1
}

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
[[ -d $ANDROID_HOME/platforms ]] || die "no Android SDK at $ANDROID_HOME; set ANDROID_HOME"

# Gradle needs JAVA_HOME to be a real JDK home. The java on PATH, a mise shim included,
# reports its own home in java.home.
if [[ -z ${JAVA_HOME:-} ]]; then
  command -v java >/dev/null || die "JAVA_HOME is unset and java is not on PATH; install JDK 17 or later"
  JAVA_HOME="$(java -XshowSettings:properties -version 2>&1 | sed -n 's/^ *java\.home = //p')"
  [[ -x $JAVA_HOME/bin/java ]] || die "could not derive JAVA_HOME from the java on PATH; set JAVA_HOME"
  export JAVA_HOME
fi

cd "$here"
./gradlew --no-daemon :app:assembleDebug >&2
[[ -f $apk ]] || die "Gradle succeeded but $apk is missing"
printf '%s\n' "$apk"
