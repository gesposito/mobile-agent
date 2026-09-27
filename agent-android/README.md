# agent-android

The resident Android agent: a Kotlin program compiled into a single `classes.dex`,
pushed to `/data/local/tmp` and started with `app_process` as the shell user, so
nothing is installed on the device (design D2). It serves the device protocol of
`crates/protocol` on an abstract Unix socket; the entry point also prints the
version banner described below.

## Layout

```
agent-android/
  VERSION                  agent version, compiled into the banner
  build.sh                 kotlinc + d8 build, no Gradle
  test.sh                  on-device smoke test
  codec-test.sh            golden cross-check of the codec against crates/protocol
  codec-test/              the Kotlin side of that cross-check
  host-tests/              Rust tests that drive the agent on a device (own Cargo workspace)
  src/dev/mobileagent/agent/
    Main.kt                entry point that app_process calls, options
    Server.kt              abstract-socket server, peer check, request dispatch
    Session.kt             one host connection: handshake, requests, deadlines, events, liveness
    Errors.kt              the structured errors the agent sends
    codec/                 the protocol codec, byte for byte the same frames as crates/protocol
      Protocol.kt          versions, limits, kinds, methods, codes, flags
      Messages.kt          message classes
      Codec.kt             encoder and decoder of every message
      CborWriter.kt        CBOR writer (preferred serialization, definite lengths)
      CborReader.kt        strict CBOR reader
      Frames.kt            32-bit length-prefixed framing
  out/   (git-ignored)     gen/BuildConfig.kt, classes/, dex/classes.dex, d8.log
  dist/  (git-ignored)     mobile-agent-android.jar
```

The codec under `codec/` is shared with `codec-test.sh`, which checks that it
writes and reads the golden frames of `crates/protocol/golden` byte for byte;
the agent uses it unchanged for every frame it sends and receives.

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
classes.dex 2281104 (build-tools 36.0.0; 2277628 with build-tools 37.0.0) bytes
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
  `mobile-agent-android 0.1.0 protocol=1`. `<version>` is the content of
  `VERSION`. `<n>` is the codec's `Protocol.VERSION`, the revision of the
  host-agent wire protocol, currently 1.
- `serve` without `--socket NAME`, or with an unknown option: prints a usage
  line to stderr and exits 2.
- Any other first argument: prints `unknown argument` to stderr, nothing to
  stdout, and exits 2.

adb's shell protocol v2, available since Android 7, returns stdout, stderr and
the exit status separately, so the host can check all three.

## Serving the protocol

```sh
adb shell MOBILE_AGENT_TOKEN=<token> CLASSPATH=/data/local/tmp/mobile-agent-android.jar \
  app_process / dev.mobileagent.agent.Main serve --socket <name>
```

- **Socket.** The agent listens on the abstract Unix socket `<name>`, which
  the host chooses per launch, and never on TCP. The host reaches it as the
  adb server service `localabstract:<name>` after `host:transport:<serial>`
  (design D3), so no forward rule exists. Once listening the agent prints
  `ready socket=<name> pid=<pid> uid=<uid> protocol=1 capabilities=0x<bits>`
  on stdout, and `ps` lists it as `mobile-agent <name>`.
- **Peer check.** Every connection's `SO_PEERCRED` uid must be 2000, the uid
  adbd and `adb shell` run as, or the agent's own uid; any other peer is
  closed before a byte is sent and logged as `refused peer uid=<uid> pid=<pid>`.
- **Token.** `MOBILE_AGENT_TOKEN` in the environment is the per-launch token.
  It stays out of the process listing and travels in the hello, where the
  host's session refuses a token other than the one it started the agent with
  (`token_mismatch`).
- **Handshake.** The agent speaks first: its hello offers versions `[1]`, its
  capabilities, a 16 MiB frame limit and `mobile-agent-android <VERSION>`.
  After the welcome it sends a `ready` lifecycle event. A welcome refusing
  the connection with `version_mismatch` makes the agent exit, so a host that
  needs another version replaces it; any other refusal only closes that
  connection.
- **Requests.** A request id already in flight closes the connection as a
  protocol error; an unknown method is answered with `unknown_method`, a
  method whose capability the hello did not advertise with
  `capability_missing`. A request whose deadline passes is answered with
  `deadline_exceeded`, its work is cancelled and its late answer dropped. A
  response over the negotiated frame size is replaced by a `frame_too_large`
  error naming the limit and the request, as is an oversized request, whose
  payload is dropped as it arrives.
- **Liveness.** The agent pings a host it has not heard from for a second and
  closes the connection after three seconds of silence, as the protocol
  crate's sessions do.
- **Reuse.** The agent outlives a connection: a later connection gets a fresh
  hello with the same token, and a new connection replaces the current one,
  so a restarted daemon reuses the running agent.
- **Stop.** `shutdown` is answered with `{}` after a `shutting_down`
  lifecycle event, and the process exits with status 0.

## Dex size baseline

| Measure | Value |
| --- | --- |
| `classes.dex` | 2,281,104 with build-tools 36.0.0, 2,277,628 with build-tools 37.0.0, which also reads Kotlin 2.4 metadata and prints no diagnostics bytes |
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

- the banner equals `mobile-agent-android <VERSION> protocol=<n>` exactly,
  where `<n>` is read from the codec's `Protocol.VERSION`;
- the exit statuses are 0, 0 and 2;
- the unknown argument produces `unknown argument` on stderr;
- `serve` without a socket name prints a usage line and exits 2;
- no process with `dev.mobileagent.agent.Main` in its command line is left
  running.

It removes the pushed jar on exit and stops at the first mismatch with a
non-zero status and a `FAIL:` message.

## Host tests

`host-tests/` is a Cargo package outside the repository workspace (its
manifest declares its own `[workspace]`) that depends on `crates/protocol` by
path. Its tests start the agent the way the daemon will, open its socket as an
adb server service stream and drive `protocol::session::HostSession`:

```sh
./build.sh
cd host-tests
MOBILE_AGENT_ANDROID_SERIAL=emulator-5554 cargo test -- --nocapture
cargo test                       # without a device: every test prints why it skips
cargo clippy --all-targets -- -D warnings && cargo fmt --check
```

The tests run one at a time, since they share the device and its single UI
automation role. Each one starts its own agent from a jar pushed under a
unique name, and kills it by pid (after checking that `/proc/<pid>/cmdline`
still names its socket) and removes the jar when it ends. They need the
Compose and Expo fixtures installed (`fixtures/`) and put back every device
setting they change.

## Spec scenarios and their checks

Every agent-side scenario of `specs/android-driver/spec.md`, and the
agent-side behaviour of `specs/device-protocol/spec.md`, maps to a named check.
Rust tests are `host-tests/tests/<file>.rs::<name>`.

| Requirement | Scenario | Checks |
|---|---|---|
| device-protocol: Version negotiation on connect | Common version selected | `handshake::the_agent_speaks_first_with_a_hello_naming_protocol_1_and_its_token` |
| | No common version | `handshake::a_host_speaking_only_a_newer_version_is_refused_and_the_agent_exits_for_replacement` |
| device-protocol: Structured errors | Transient versus permanent failure (unknown method) | `handshake::an_unknown_method_gets_a_permanent_error_and_the_connection_stays_usable` |
| device-protocol: Liveness detection | Agent side of the pings | `handshake::the_agent_pings_a_silent_host_every_second`, `handshake::every_ping_answer_keeps_a_quiet_session_open_past_the_peer_timeout`, `handshake::the_agent_closes_a_connection_whose_host_stops_answering` |
| design D3: token and peer check | | `handshake::a_host_expecting_another_token_refuses_the_hello_and_the_agent_keeps_serving`, `handshake::a_peer_that_is_not_the_shell_user_gets_no_hello` |
| android-driver: Agent deployment without installation | Compatible agent reused | `handshake::a_later_connection_reuses_the_running_agent` |
| | Incompatible agent replaced | `handshake::a_host_speaking_only_a_newer_version_is_refused_and_the_agent_exits_for_replacement` |
| android-driver: Agent lifecycle | Explicit stop | `handshake::shutdown_is_answered_before_the_agent_exits` |
| Banner and arguments | | `test.sh` |
