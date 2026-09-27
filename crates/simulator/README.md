# simulator

Drives iOS Simulators entirely from the host (design D6, D7 and D12, spec
`simulator-driver`): the lifecycle through CoreSimulator, input through
dtuhidd or the legacy Indigo client, frames from the display surface, and the
accessibility tree through a reader spawned into the guest. Nothing is built
or installed in the Simulator.

`SimulatorDriver` implements the driver traits (`Lifecycle`, `Frames` and
`Input` so far). They carry no Apple types, so the mirror and the daemon can hold any
backend behind them; they live in this crate until `crates/mirror` defines
its own.
On hosts other than macOS the crate still builds: `SimulatorDriver` exists
there, and `open` fails with `platform_unsupported`.

## Layout

```
src/
  driver.rs        the driver traits
  model.rs         the values they take and return
  error.rs         errors, each with a stable code
  frames.rs        FrameStream and its bounded queue
  pipeline.rs      box downscale, hashes and the read throttle, no Simulator needed
  gesture.rs       gestures as timed touch steps in framebuffer ratios
  gate.rs          the activation gate that holds input until dtuhidd answers
  keymap.rs        characters and key names to USB HID usages
  lookup.rs        finding a device, runtime or device type from user input
  version.rs       dotted version comparison
  unsupported.rs   the driver on hosts other than macOS
  macos/
    objc.rs        framework loading, the exception guard, NSError conversion
    forward.rs     messages to ROCKit forwarding proxies
    coresim.rs     CoreSimulator: service context, device set, devices
    prefs.rs       the accessibility preferences written before a first boot
    display.rs     the main display, its SimScreen callbacks and IOSurface
    frames.rs      frame subscriptions and screenshots
    encode.rs      the JPEG encoders
    input.rs       a target's input: the Transport trait and the gesture runner
    xpc.rs         libxpc and the private _4sim functions
    dtuhid.rs      the dtuhidd XPC transport
    indigo.rs      the legacy Indigo transport
    driver.rs      SimulatorDriver
tests/
  common/          gating, fixtures, temporary devices, simctl and ps helpers
  lifecycle.rs     task 6.1
  frames.rs        task 6.2
  indigo.rs        task 6.3
  dtuhid.rs        task 6.4
```

## Building

The frame encoder is turbojpeg, the default feature, which spike S2 chose for
being 2.7 to 3.1 times faster than the alternative. It links libjpeg-turbo
statically from the host instead of building the copy vendored in
`turbojpeg-sys`, which needs cmake: the workspace's `.cargo/config.toml` sets
`TURBOJPEG_SOURCE=explicit`, `TURBOJPEG_STATIC=1` and the Homebrew paths on
Apple silicon, so `brew install jpeg-turbo` is all a build needs. Variables set
in the environment take precedence, for another prefix such as `/usr/local`.
The binary does not load libturbojpeg at run time.

On a host without libjpeg-turbo, build the pure-Rust fallback instead:

```sh
cargo build -p simulator --no-default-features --features jpeg-encoder
```

With both features, jpeg-encoder takes over if turbojpeg cannot start. The
workspace builds this crate and jpeg-encoder at `opt-level = 3` in the dev
profile too: unoptimized, the pixel pipeline took 230 ms per 460x1000 frame,
and even `cargo test` has to hold 30 fps.

## Private calls

Every Objective-C call into CoreSimulator is a dynamic message send inside
`objc::guarded`, which catches an Objective-C exception as an error naming
the call and drains the autorelease pool the call filled. Classes are looked
up by name (`AnyClass::get`), never linked. The exception guard needs
`panic = "unwind"`, which is the default for every profile of this workspace.

Before booting a device that is shut down, the driver writes
`AccessibilityEnabled`, `ApplicationAccessibilityEnabled` and
`AutomationEnabled` into `Library/Preferences/com.apple.Accessibility.plist`
under the device's `-[SimDevice dataPath]`, keeping every other key; spike S4
found both tree read paths failing for seconds after a boot without them.
`create_device` writes them too, so a new device has them before its first
boot. A booted device's preferences belong to the guest and are left alone.

## Frames

A subscription registers SimScreen callbacks on the main display
(`displayClass == 0`), whose descriptor is a ROCKit proxy: calls to it go
through `forward::send`, and its blocks carry their type signature. The
present callback only counts the tick and wakes a worker thread at
user-interactive QoS. The worker reads the IOSurface the surfaces callback
handed over, box-filters it down to `max_dimension` (1000 by default, 460x1000
on an iPhone 17) and hashes the result:

- `hash` covers every delivered pixel. A frame is delivered only when it
  differs from the last one, so presents without a visible change (S2 counted
  35 per appearance toggle) deliver nothing, and a static display delivers
  nothing at all.
- `content_hash` is S2's 32-pixel thumbnail hash of the frame below the status
  bar, for the settle predicate (D8): a clock tick or a blinking cursor does
  not change it.

The throttle (`max_fps`, 30 by default) skips presents rather than reading
late, because the render server does not wait for a reader's lock and late
reads tear (S2): after a read the worker sleeps through three quarters of the
interval and reads on the next present. A present that arrived during the
sleep is read if no fresh one follows, so the last frame of an animation is
never lost. Frames wait in a queue of 16 that drops the oldest when the
consumer falls behind. `FrameStats` counts presents, reads, deliveries, torn
and dropped frames, and the pipeline's CPU time.

On the spike Simulator the Anim tab streamed 30 fps at 460x1000 with the
pipeline at 10 to 14 percent of one core and the whole test process under 15
(turbojpeg, dev profile); jpeg-encoder held 30 fps at 15 to 16 percent. The
downscale costs about 2 ms per frame, with a 16-bit vertical pass and an
exact reciprocal in place of each division; its output is identical to S2's
filter.

## Input

`gesture.rs` turns a tap, a press, a swipe or a two-finger gesture into
touch steps at set times, with points converted to ratios of the native
portrait framebuffer (`point * scale / pixels`, as idb and S3 do). A swipe
sends its positions at even intervals and repeats the last before lifting,
which keeps an arm64 guest from adding inertia. `input.rs` sends the steps
through the target's transport, one request at a time per target, and
reports a `Dispatch`: the transport, the wait for its activation, and the
times from the request to the first event and to the last.

The dtuhidd transport (`dtuhid.rs`, from S3) is the default from
CoreSimulator 1155.4. It looks up `com.apple.coredevice.feature.remote.hid.digitizer`
with `-[SimDevice lookup:error:]`, wraps the port with the private
`xpc_endpoint_create_mach_port_4sim`, connects and marks the connection
simulator-to-host, and keeps that one connection for the target's life,
carrying touches with one or two contacts and the edge flag, keyboard usages
and Consumer-page buttons. Opening sends the activation probe and returns at
once. Until the reply, the gate (`gate.rs`) holds whatever is submitted; the
reply handler flushes it the moment the reply arrives, under the gate's lock
so nothing overtakes it, with no fixed wait after the reply. A request whose
first event was held waits for that flush, reports the wait as
`activation_wait`, and times the rest of its gesture from it. A daemon that
does not answer is retried every 4 s, five times, and a peer that goes away
ends the session; the next request opens a new one, and `status` counts the
connections.

On the spike Simulator, a tap requested as the session opened waited 14.7
ms for a warm daemon and 320 ms on a device's first session of its boot,
while launchd started dtuhidd, and went out within a microsecond of the
reply either way. Over 100 warm taps the touch-down reached XPC in 44 µs at
p50 and 0.83 ms at p95, and the whole tap was sent in 98 µs and 8.5 ms, under
a host load average near 20.

The Indigo transport (`indigo.rs`, from S3) is the fallback for hosts whose
CoreSimulator predates 1155.4. It builds each touch with SimulatorKit's
`IndigoHIDMessageForMouseNSEvent` (the Xcode 27 six-parameter prototype,
with a unit size so a ratio passes through), re-envelopes it as idb's
single-touch message and sends it through `SimDeviceLegacyHIDClient`, waiting
for each completion. The builder keeps unsynchronized global state and
returns nil within 16 ms of its previous message, so every build in the
process holds one lock and keeps 17 ms apart: a tap takes about 21 ms. Moves
are left-mouse-dragged events, which scroll like a finger. The message
pointer is passed with the struct encoding SimulatorKit declares, which
keeps objc2's debug-build signature check on. Indigo carries one finger
only; keys, buttons, two fingers and screen-edge touches are refused as
unsupported.

On this host Indigo touches did not land on the long-running shared
Simulator, whose dtuhidd had served many sessions, even with S3's 40 ms hold.
On a device booted for the test they landed every time, and they still did
after that device's first dtuhidd session, as in S3, so what stopped them on
the shared Simulator is not known. The Indigo tests therefore run on a device
they create.

## Tests

`cargo test -p simulator` runs the unit tests. They need macOS with Xcode,
since some load CoreSimulator, but no Simulator.

The integration tests in `tests/` drive real Simulators. Each checks
`MOBILE_AGENT_SIM_UDID` first; when it is unset, as in CI, the test prints why
it skipped and passes. To run them, name a booted iPhone 17 Simulator with
both fixtures installed and the Expo fixture on its Login tab:

```sh
MOBILE_AGENT_SIM_UDID=<udid> cargo test -p simulator
```

Tests that create devices name them `mobile-agent-simtest-*` and delete them
when they end, pass or fail; a binary that needs a freshly booted device
makes one and deletes it on exit, or uses the one `MOBILE_AGENT_SIM_FRESH_UDID`
names. They install the fixtures from `fixtures/*/build`,
which `just fixtures-build` produces, or from the bundles named by
`MOBILE_AGENT_SWIFTUI_APP` and `MOBILE_AGENT_EXPO_APP`. A first boot takes
about a minute.

## Spec requirements and their tests

| Requirement | Scenario | Tests |
|---|---|---|
| Simulator discovery and lifecycle | Launch with environment | `lifecycle::launch_with_environment`, `lifecycle::every_simulator_is_listed_with_identifier_name_runtime_and_state`, `lifecycle::a_booted_device_is_left_alone`, `lifecycle::an_unknown_device_is_not_found` |
| Frames and screenshots | Frame stream during animation | `frames::frame_stream_during_animation`, `frames::frames_stop_when_the_display_is_static`, `frames::frames_sustain_30_fps_under_a_fifth_of_a_core`, `frames::a_screenshot_is_timestamped_at_the_display_size` |
| Input transport selection | Input before activation | `dtuhid::input_before_activation`, `dtuhid::the_first_session_of_a_boot_waits_for_dtuhidd_to_start` |
| Input actions | Warm tap latency | `dtuhid::warm_tap_latency` |
| Input actions | Taps, keys and buttons over one dtuhidd connection (task 6.4) | `dtuhid::a_tap_lands_on_the_fixture_button`, `dtuhid::one_connection_carries_touch_keys_and_buttons` |
| Input actions | Taps and swipes over the Indigo fallback (task 6.3) | `indigo::a_tap_lands_on_the_fixture_button`, `indigo::a_swipe_scrolls_the_list` |
