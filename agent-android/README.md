# agent-android

The resident Android agent: a Kotlin program compiled into a single `classes.dex`,
pushed to `/data/local/tmp` and started with `app_process` as the shell user, so
nothing is installed on the device (design D2). The entry point prints the version
banner described below.

## Layout

```
agent-android/
  VERSION                            agent version, compiled into the banner
  build.sh                           kotlinc + d8 build, no Gradle
  test.sh                            on-device smoke test
  src/dev/mobileagent/agent/Main.kt  entry point that app_process calls
  out/   (git-ignored)               gen/BuildConfig.kt, classes/, dex/classes.dex, d8.log
  dist/  (git-ignored)               mobile-agent-android.jar
```

## Requirements

- A JDK 17 or newer with `java` and `jar` on PATH (tested with Temurin 21).
- The Kotlin compiler, `kotlinc` (tested with 2.4.20).
- An Android SDK at `ANDROID_HOME`, default `$HOME/Library/Android/sdk`, with
  `platforms/android-36` and a `build-tools` package (tested with 36.0.0), plus
  `platform-tools/adb` for the smoke test.

## Building the dex without Gradle

`./build.sh` runs four steps:

1. **Version constant.** `VERSION` must hold one semantic version. It becomes
   `const val VERSION` in the generated `out/gen/dev/mobileagent/agent/BuildConfig.kt`.
2. **Compile.**
   `kotlinc -jvm-target 17 -no-jdk -no-reflect -Werror -classpath $ANDROID_HOME/platforms/android-36/android.jar -d out/classes <sources>`.
   `-no-jdk` takes the host JDK off the compile classpath, which leaves
   `android.jar` as the only platform API. As a result, a call to a JDK-only API
   such as `java.lang.ProcessHandle` fails to compile instead of failing on the
   device. `-no-reflect` keeps kotlin-reflect out of scope because the dex does
   not include it. `-Werror` follows the workspace rule that warnings fail
   validation.
3. **Dex.**
   `d8 --release --min-api 26 --lib android.jar --output out/dex <each .class file> kotlin-stdlib.jar`.
   d8 accepts class files and archives but not directories, so each class file
   is passed on its own. The Kotlin stdlib is dexed in so that the jar needs
   nothing else on the device's class path. The build fails if d8 splits the
   output into more than one dex file.
4. **Package.** `jar --create --no-manifest` makes `classes.dex` the only member
   of `dist/mobile-agent-android.jar`. `app_process` accepts that jar on
   `CLASSPATH`.

Progress goes to stderr. The last two lines on stdout are the jar path and the
dex size:

```
/…/agent-android/dist/mobile-agent-android.jar
classes.dex 2281104 bytes
```

Tool resolution:

- **d8.** Taken from the newest `$ANDROID_HOME/build-tools/*`, never from PATH.
  The SDK's cmdline-tools and Homebrew ship their own d8 releases.
- **kotlinc and `kotlin-stdlib.jar`.** Both come from one Kotlin home, so the
  compiler and the dexed stdlib always match. The home is `KOTLIN_HOME` if set.
  Otherwise it is the install behind `kotlinc` on PATH, with symlinks resolved;
  a mise shim is resolved with `mise which kotlinc`.

### Kotlin metadata diagnostics

The D8 in build-tools 36.0.0 (8.10.9) reads Kotlin metadata up to version 2.2.0,
but Kotlin 2.4.20 writes version 2.4.0. That D8 therefore reports diagnostics for
almost every stdlib class: about three per class, over 2,800 per build. It still
exits 0 and produces a working dex. Only kotlin-reflect reads this metadata, and
the agent does not include it. `build.sh` saves d8's full output to
`out/d8.log`, prints a one-line count of these diagnostics, and shows every other
diagnostic in full. A D8 that knows the newer metadata stops reporting them:
D8 9.4.14 reports nothing for the same input.

## Launching with app_process

```sh
adb push dist/mobile-agent-android.jar /data/local/tmp/mobile-agent-android.jar
adb shell CLASSPATH=/data/local/tmp/mobile-agent-android.jar app_process /data/local/tmp dev.mobileagent.agent.Main --version
```

`app_process` is the platform's launcher for Java programs; the device's own
`monkey` and `uiautomator` commands are scripts that exec it. Given a class name,
it starts an ART runtime with `CLASSPATH` as the system class path, loads the
class from the jar's `classes.dex`, and calls its static `main(String[])`. The
`/data/local/tmp` argument fills app_process's parent-directory slot, which it
ignores.

Started through `adb shell`, the process runs as the `shell` user (uid 2000).
It installs no package, uses no instrumentation and shows no prompt. It ends
when `main` exits. `Main` calls `exitProcess` so that the process also ends if
platform code has started non-daemon threads.

## Banner contract

- `--version`, or no arguments: prints exactly one line to stdout and exits 0.
  The line is `mobile-agent-android <version> protocol=<n>`, for example
  `mobile-agent-android 0.1.0 protocol=0`. `<version>` is the content of
  `VERSION`. `<n>` is `Main.PROTOCOL_VERSION`, the revision of the host-agent
  wire protocol, which is currently 0.
- Any other argument: prints `unknown argument` to stderr, nothing to stdout,
  and exits 2.

adb's shell protocol v2, available since Android 7, returns stdout, stderr and
the exit status separately, so the host can check all three.

## Dex size baseline

| Measure | Value |
| --- | --- |
| `classes.dex` | 2,281,104 bytes |
| Agent classes alone (stdlib on the classpath, not dexed) | 2,432 bytes |
| `mobile-agent-android.jar` (deflated) | 663,748 bytes |
| Dex contents | 1,083 classes, 12,700 method references |
| Toolchain | Kotlin 2.4.20 stdlib, D8 8.10.9 (build-tools 36.0.0), `--release --min-api 26` |

The Kotlin stdlib, dexed whole and unshrunk, accounts for almost all of the size.
The build is reproducible: repeated builds produce byte-identical dex files.

This is the baseline for the dex-size gate in spike S5 (design D4): a wire-format
candidate may add under 100 KB. The size depends on the toolchain, so compare
builds made with the same Kotlin and D8 versions. For example, D8 9.4.14 gives
2,277,544 bytes for the same input.

## Smoke test

```sh
./build.sh && ./test.sh                  # the only connected device
ANDROID_SERIAL=emulator-5554 ./test.sh   # a chosen device
```

`test.sh` checks that the jar's only member is `classes.dex` and pushes the jar
to `/data/local/tmp`. It then runs the agent through `app_process` three times:
with `--version`, with no arguments, and with an unknown argument. Each run has
a 30-second timeout on the device. The test checks:

- the banner equals `mobile-agent-android <VERSION> protocol=0` exactly;
- the exit statuses are 0, 0 and 2;
- the unknown argument produces `unknown argument` on stderr;
- no process with `dev.mobileagent.agent.Main` in its command line is left
  running.

It removes the pushed jar on exit and stops at the first mismatch with a
non-zero status and a `FAIL:` message.
