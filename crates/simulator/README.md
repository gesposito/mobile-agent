# simulator

Drives iOS Simulators entirely from the host (design D6, D7 and D12, spec
`simulator-driver`): the lifecycle through CoreSimulator, input through
dtuhidd or the legacy Indigo client, frames from the display surface, and the
accessibility tree through a reader spawned into the guest. Nothing is built
or installed in the Simulator.

`SimulatorDriver` implements the driver traits: `Lifecycle`, `Frames`,
`Input` and `Tree`. They carry no Apple types, and the tree read returns the
protocol's snapshot payload (`crates/protocol`) in the form the mirror's
backend traits expect (`mirror::backend`), so the daemon can put the driver
behind those traits. On hosts other than macOS the crate still builds
(Building, Hosts other than macOS): `SimulatorDriver` exists there, and
`open` fails with `platform_unsupported`.

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
  cadence.rs       the pause between typed keys, React Native's or the native one
  select.rs        choosing the input transport from the host's versions
  compat.rs        the Xcode gate and the doctor compatibility table
  tree.rs          ForegroundTree: owner, alert, covered content, degradations
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
    host.rs        the host's CoreSimulator and Xcode versions
    pasteboard.rs  the paste grants, and a SET to dtpasteboardd or devicectl
    reader.rs      the embedded reader: caching, spawning, its protocol link
    translator.rs  the host translator, the tree fallback
    driver.rs      SimulatorDriver
build.rs           builds reader/ for the Simulator, for embedding
reader/            simulator-reader, the tree reader spawned into the guest
  src/
    elements.rs    the tree both read paths fill, and its snapshot payload
    guest/
      objc.rs      image loading and the exception guard, in the guest
      runtime.rs   XCTAutomationSupport, AXRuntime and AXSettings
      extract.rs   the runtime's snapshot dictionaries to elements
      process.rs   a pid's executable, bundle and whether it is system UI
      server.rs    the device protocol over one connection, reads on a worker
    main.rs        simulator-reader connect <socket> --token --scale
tests/
  common/          gating, fixtures, temporary devices, simctl and ps helpers
  lifecycle.rs     task 6.1
  frames.rs        task 6.2
  indigo.rs        task 6.3
  dtuhid.rs        task 6.4
  transport.rs     task 6.5
  tree.rs          task 6.6
  text.rs          task 6.7
  containment.rs   tasks 6.8 and 6.6
  metadata.rs      task 6.10
  cadence.rs       task 3.5 of rn-app-fixes
benches/
  frames.rs        the frame pipeline's microbenchmarks (design D14)
```

`cargo bench -p simulator -- --noplot [filter]` runs the benchmarks, which
need no Simulator. The library sets `bench = false`, so its test harness
never sees criterion's flags. `just micro` runs them with the other suites
against `reports/micro/baseline.json` (design D14 to D16 of the
`microbenchmarks` change).

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
cargo build -p simulator --no-default-features --features jpeg-encoder,reader
```

With both features, jpeg-encoder takes over if turbojpeg cannot start. The
workspace builds this crate and jpeg-encoder at `opt-level = 3` in the dev
profile too: unoptimized, the pixel pipeline took 230 ms per 460x1000 frame,
and even `cargo test` has to hold 30 fps.

### The reader

The tree reader, `reader/` (package `simulator-reader`, a workspace member),
is built for `aarch64-apple-ios-sim` and embedded in this crate, so nothing
is compiled on the machine that runs the driver. `build.rs` runs a nested
`cargo build --release --offline --locked` of it for that target, in
`target/<profile>/simulator-reader-target/`, which leaves an unchanged
reader at a fingerprint check; a first build takes about 20 s. It needs the
target's standard library and the iOS Simulator SDK that Xcode ships:

```sh
rustup target add aarch64-apple-ios-sim
```

On macOS without the target, the build prints a warning, embeds nothing,
and tree reads use the host translator; `doctor` says so. The `reader`
feature, on by default, asks for the reader: a build without it
(`--no-default-features --features turbojpeg`) embeds nothing on purpose,
which is how the fallback is tested. `MOBILE_AGENT_READER` names a prebuilt
reader to embed instead. A reader
that fails to compile fails the build. The binary is 0.89 MB, links only
Foundation, CoreFoundation, libobjc, libSystem and libiconv, and carries
only the ad hoc signature the linker gives every arm64 binary. The reader's
library, the conversion into the snapshot payload, also builds for the
host, where the workspace lints and tests it, on macOS with the rest of the
reader's code; its runtime exists only inside a Simulator. It builds as it
is for `aarch64-apple-ios` too, where the physical iPhone's runner
(`agent-ios`) links it and converts XCTest's snapshot with `elements`; the
snapshot call stays the Simulator's, since the runner's sandbox refuses it
on a device (design D21 of `physical-ios`).

### Hosts other than macOS

On other hosts, such as the Linux CI job, the crate builds without its
Apple parts. Its Apple dependencies (objc2 and its framework crates,
block2, dispatch2 and libc) and both JPEG encoders are macOS-only, so
neither libjpeg-turbo nor the `aarch64-apple-ios-sim` target is needed, and
the `TURBOJPEG_*` settings in `.cargo/config.toml` go unused. `build.rs`
skips the reader without a warning and embeds nothing, `src/unsupported.rs`
stands in for `src/macos/`, and the daemon registers only the Android
driver. The reader's `guest` module and its dependencies are Apple-only
too, so the reader's binary builds as a stub that exits with status 2, and
its library holds only `elements` and the error codes.

## Private calls

Every Objective-C call into the private frameworks is a dynamic message
send inside `objc::guarded`, which catches an Objective-C exception, and a
Rust panic such as objc2's debug-build signature check failing, as an error
naming the call, and drains the autorelease pool the call filled. Classes are
looked up by name (`AnyClass::get`), never linked. The guard needs
`panic = "unwind"`, which is the default for every profile of this workspace.
Blocks the frameworks call back, the frame callbacks and the XPC handlers,
catch their own panics, which would otherwise unwind into libdispatch and
abort.

Every request for one device runs through `contain`. An exception or panic
caught during it takes that device out of service: the request fails with
`target_unavailable` and diagnostics naming the call, the failure, the time
and the host's CoreSimulator and Xcode, later requests for the device fail at
once, `status` reports the diagnostics, and `reset_target` drops the device's
sessions and brings it back. Other devices are untouched. The
`fault-injection` feature, which only this crate's tests enable, makes a
device's next private call raise `MobileAgentInjectedFault` inside the
guard.

Before booting a device that is shut down, the driver writes
`AccessibilityEnabled`, `ApplicationAccessibilityEnabled` and
`AutomationEnabled` into `Library/Preferences/com.apple.Accessibility.plist`
under the device's `-[SimDevice dataPath]`, keeping every other key; spike S4
found both tree read paths failing for seconds after a boot without them.
`create_device` writes them too, so a new device has them before its first
boot. A booted device's preferences belong to the guest and are left alone.

## Tree

`snapshot` reads the tree of whatever owns the foreground and returns a
`ForegroundTree`: the payload, who owns the foreground, the alert in front,
and why the read is degraded.

The reader (spike S4's port of agent-device's snapshot bridge) is spawned
into each booted Simulator on its first read, through `-[SimDevice
spawnWithPath:options:terminationQueue:terminationHandler:pid:error:]`,
never `xcrun simctl spawn`, whose own start costs about 295 ms. The embedded
binary is written once per content to
`$HOME/Library/Caches/dev.mobileagent.simulator/`, since the first spawn of
a newly written file is slower, and runs from there: nothing is installed in
the Simulator. The host listens on a Unix socket in its temporary directory;
the reader loads XCTAutomationSupport and AXRuntime from the runtime root,
then connects and speaks the device protocol as an agent with the `tree`
capability, its hello carrying the token the host spawned it with. A
connection therefore means it can read, and readiness is the time from the
spawn call to its hello, which `status` reports with the reader's pid, path
and state. Reads run on a worker thread so the connection answers pings
during a slow one; a request that arrives meanwhile is answered `busy`, and
a read that hangs for 20 s ends the process (status 70). The reader exits
when the host shuts it down or its connection ends, so it never outlives the
driver. Its standard error goes to `simulator-reader-<udid>.log` in the
same cache directory.

A read asserts automation mode, as agent-device does, asks `+[AXElement
primaryApp]` who owns the foreground, snapshots that process with one
`-userTestingSnapshotForElement:options:error:` (64 levels and 10,000 nodes
by default, retried at half depth when the server rejects the depth), and
checks the owner again afterwards: a change fails the read with
`foreground_changed`; each scroll view on screen then takes one more call,
for its scroll area. A permission alert makes SpringBoard the owner, so the
read is SpringBoard's tree, whose alert lists its buttons. The snapshot
honours the views the app marks modal (`snapshotKeyHonorModalViews`, as
XCTest's queries do), since no attribute says which views those are (task
10.22): the app then replaces a modal view's parent's children with the
modal view and still counts them. With no modal view on screen it holds
every node and costs what a plain one does (79 and 225 ms against 79 and
218 ms at p50 on Bluesky's home and Explore screens, 244 and 484 nodes).
When some element delivered fewer children than it counts, a second
snapshot without the option is the read, which keeps what the modal views
cover for the mirror to cut, and the views the first one kept there are
flagged `modal` (matched by class, type, identifier and label, then by what
they hold, then by frame; nothing is flagged where the two disagree). On
Bluesky's search screen with the typeahead up the first took 41 ms for 164
nodes and the second 228 ms for 578, at p50. The payload follows the
mirror's conventions (`reader/src/elements.rs`):

- the process's own window, its id the owner's pid, its kind `system` for
  system UI (SpringBoard and the other processes of the runtime's
  CoreServices) and `application` otherwise, with the owner's bundle
  identifier and name; the keyboard's `UIRemoteKeyboardWindow` becomes an
  input-method window above it, its id the pid with bit 31 set. Its keys
  come in the application's own read (37 on both fixtures' login fields):
  agent-device lists that window as an `Application` only because iOS 26.5
  gives every window automation type 2, which XCTest's enum names so. A
  child moved to a window of its own still counts as delivered, so the move
  marks nothing truncated, as it did before task 6.12 whenever the keyboard
  was up. The window spans the screen and lets touches through outside
  what it draws, so its bounds are the union of its outermost descendants
  that do not span it, clipped to the screen: the host view of its input
  views, from the prediction bar down through the keys and the dock to the
  bottom of the screen (539 to 874 points on an iPhone 17 on Bluesky's
  search screen, where the `Keyboard` element starts at 583), and none
  while it draws nothing: with its frame the mirror would count every
  element under the window, above the keyboard too, as out of view (task
  10.21);
- an application's alert on screen moves, with the view that presents it,
  to a window of its own between the two, its id the pid with bit 30 set,
  titled by the alert, marked `modal`, and `active` and `focused` in the
  application window's stead. The view is the child of the alert's window
  that holds it, a transition view with two dimming views and the alert,
  whether the alert is drawn in the application's window (SwiftUI) or in a
  window of its own (React Native). The application's window keeps what the
  alert covers and is not modal, so the mirror covers all of it. System UI
  keeps its alerts in its own window, which is not modal: the read holds no
  application window for them to cover, and the mirror cuts the rest of
  SpringBoard's window by the alert's class;
- a sheet that blocks the screen moves likewise, with its transition view,
  to a window of its own, its id the pid with bit 29 set, titled by its
  navigation bar or first heading, marked `modal` and, unless an alert sits
  above it, `active` and `focused`. UIKit presents a sheet in a
  `UITransitionView`, a later sibling of the one holding the screen, beside
  a `UIDimmingView` spanning three screens (reading "dismiss popup",
  `PopoverDismissRegion`, where it dismisses the sheet, as on Bluesky); the
  sheet blocks what it covers while that dimming view takes touches, which
  the reader asks of each dimming view in a snapshot of its own element
  (`XC_kAXXCAttributeIsUserInteractionEnabled`, at most 8 a read), as
  agent-device's bridge does. The last child of an application's window
  that is such a view is taken, from the top-most window that has one. A
  dimming view that lets touches through, at an undimmed detent, or whose
  answer the runtime does not give, blocks nothing, and the sheet stays in
  the application's window; so does a system UI sheet. Contacts' New
  Contact sheet and Bluesky's hosting-provider sheet and birth-date dialog
  have this shape;
- every window says it sends no change events (`emits_events` false), since
  the Simulator sends none (spikes S3 and S4) and the mirror reads it again
  after each action and on a poll; system UI names itself as the `owner` of
  each of its windows by its process name, `SpringBoard`, in the string
  table, and an application's windows name no owner;
- windows are listed, and their nodes numbered, top-most first (the
  keyboard, an alert, a sheet, the process's own), so a node limit cuts the
  windows behind first;
- `class` is the `XCUIElementType` name, or the UIKit class for elements the
  runtime types as `Other` or `Group`, so the alert controller's
  `_UIAlertControllerPhoneTVMacView` names itself; a static text with the
  header trait is a `Heading`;
- `desc` is the label and `text` the value; an empty field's value, which
  repeats its placeholder, is dropped and the placeholder becomes its label;
  a switch's value becomes `checked`;
- bounds are pixels (points times the screen scale, which the host passes
  the reader); `visible` means the frame overlaps the screen, `enabled`,
  `selected` and `clickable` come from the traits, `editable`, `password`
  and `scrollable` from the type, and `focused` from the private editing
  trait, which only the field with keyboard focus carries; `modal` marks a
  view the app marks `accessibilityViewIsModal`, found as above, whose
  siblings the mirror cuts within its bounds;
- a scroll view on screen (a `ScrollView`, `Table` or `CollectionView`, at
  most 32 a read), told by the runtime's automation type, never by a class
  name, carries `scroll`, from a snapshot of its own element with
  three more attributes: where its frame's edges fall in its content along
  each axis, as fractions of the content
  (`XC_kAXXCAttributeVisibleVerticalScrollArea` and its horizontal twin),
  and the part of the frame that shows content
  (`XC_kAXXCAttributeVisibleFrame`), which leaves out the insets UIKit keeps
  under bars (62 and 83 points on the SwiftUI list, none on the Expo one).
  Along an axis with more than a point of content beyond the visible frame,
  `back` and `forward` say on which side, and the range is percent of the
  content, 0 to 100, with the visible frame's part as the position,
  rounded, but 0 only at the start and 100 only at the end: 0 to 7 for the
  SwiftUI list at its top, 4 to 11 after a 300-point drag. A scroll view
  that scrolls along both axes names no axis and no range, and one whose
  content fits has no `scroll`. The runtime reports no item counts or
  indices (its `ScrollStatus` is VoiceOver's localized "page 1 of 15"), so
  no range counts items. Asking every node for the three attributes cost
  the Big screen's 546 nodes 40 ms a read; asking each scroll view costs
  3.6 ms on the SwiftUI list and 0.6 ms on the Expo one. The translator
  reports no scroll semantics. React Native's `ScrollView`, a `FlatList`'s
  too, is an `XCUIElementTypeScrollView` inside its
  `RCTScrollViewComponentView`, and the SwiftUI fixture's List an
  `XCUIElementTypeCollectionView`; a view React Native lays out reads as an
  `RCTViewComponentView` holding nothing when it draws something, as a
  background, what it lays out following it as its siblings, top to
  bottom, and as no node at all when it draws nothing (a transparent view,
  one with `pointerEvents="box-none"` or with `onLayout` alone), as the
  throwaway creation tab of `rn-app-fixes` task 5.1 read on the spike
  Simulator, which the mirror takes for a view drawn over what it lies on
  (`crates/mirror/README.md`);

An alert is the last visible alert container in the first window that holds
one: the alert's own window for an application's alert, SpringBoard's for a
system alert. Its title is the container's label, its message the other
static texts, and its buttons the outermost ones, since UIKit nests each
action's button in another with the same label. The payload keeps what an
alert covers, as the reader returns it, because the mirror cuts that itself
and still reports a ref to a covered element as covered; `uncovered()`
returns the payload without the windows below the top-most modal window,
and with only the alert and the containers it sits in in the alert's
window. The foreground is the process's own window, never an alert's or a
sheet's. UIKit ignores
touches while an alert presents, although the alert is in the tree from the
start, so a client waits for the display to settle before tapping its
buttons.

A read is degraded when the translator read it, when a bound cut it, or when
an application window holds only its root, as a read right after a launch
does. `no_foreground`, `foreground_changed`, `snapshot_failed` and `busy` are
retryable: an application answers about 1.5 s (SwiftUI, Debug) to 2 s
(Expo, Release) after its launch. An Objective-C exception inside the reader
comes back as the call and the exception, and takes the device out of
service like any other.

The host translator (`AXPTranslator`, answered through `-[SimDevice
sendAccessibilityRequestAsync:completionQueue:completionHandler:]`) reads
instead when the reader cannot run: no reader in this build, a spawn that
fails, or a reader that exits or never says hello. Without a reader in the
build the first read already uses the translator; after two failed spawns
in a row the device stays on it until `reset_target`; a reader whose
connection ends is replaced once within the read. A spawn runs with the
device's tree state unlocked, so what it records there cannot wait on the
read that asked for it, while a second read of the same device waits for
that spawn rather than starting another. The translator
needs nothing in the guest, but makes about seven requests per element,
flattens containers, drops the tab bar's buttons and React Native's text
identifiers, and answers with a childless application for seconds after a
launch (S4), so its reads are marked degraded with the reason the reader was
not used. It maps AX roles to the same class names. The translator is one
object per process, so reads of different devices take turns.
`DriverOptions::tree` can force either backend and `DriverOptions::reader`
can name another reader binary.

On the spike Simulator the reader was ready 271 to 303 ms after its spawn
(first spawn of the cached file 302 ms), read the SwiftUI Login screen's 48
nodes in 13.9 to 14.3 ms and the Big screen's 546 in 126 to 152 ms, the
nodes XCTest returns.

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
  bar, for the settle predicate (D8): a tick of the status bar clock does not
  change it. A focused field's blinking cursor does, and so does the exact
  `hash`, so a screen with a focused field keeps delivering frames.

The throttle (`max_fps`, 30 by default) skips presents rather than reading
late, because the render server does not wait for a reader's lock and late
reads tear (S2): after a read the worker sleeps through three quarters of the
interval and reads on the next present. A present that arrived during the
sleep is read if no fresh one follows, so the last frame of an animation is
never lost. Frames wait in a queue of 16 that drops the oldest when the
consumer falls behind. `FrameStats` counts presents, reads, deliveries, torn
and dropped frames, and the pipeline's CPU time.

On the spike Simulator the Anim tab streamed 30 fps at 460x1000 with the
pipeline at 7 to 13 percent of one core and the whole test process at 8 to
14 (turbojpeg, dev profile); jpeg-encoder held 30 fps at 12 to 16 percent.
The downscale costs about 1.2 ms per frame: for a frame 2.6 times smaller
than the display it sums rows in 16 bits, sums each pixel's columns without
a branch, and divides with a 32-bit reciprocal; any other size, such as the
thumbnails, takes a general path, and both give S2's filter's output byte
for byte. Measured CPU time rises with the host's load: runs right after a
rebuild, while the host's endpoint agent scanned the new binaries, read up
to 24 percent, so the CPU test waits for a quiet host as the spikes did.

The pixel work is public from the crate root so that `benches/frames.rs`
can time it without a Simulator: `FrameView`, `fit_within`, `scale_bgra`,
`pixels_hash`, `thumbnail_hash`, `content_hash` and `status_bar_rows`
wherever the crate builds, and the JPEG `Encoder` on macOS.

## Input

`gesture.rs` turns a tap, a press, a swipe or a two-finger gesture into
touch steps at set times, with points converted to ratios of the native
portrait framebuffer (`point * scale / pixels`, as idb and S3 do). A swipe
sends its positions at even intervals and repeats the last before lifting,
which keeps an arm64 guest from adding inertia; the finger still lifts
moving, and UIKit glides the content on at the drag's speed (a 600-point
swipe over 300 ms moved the Expo fixture's list 1,522 to 1,608 points on
the iOS 26.5 Simulator). A swipe with a `rest` (task 10.16) comes to rest
instead: the finger is at the end at the swipe's duration, sends nothing for
the rest, creeps on three moves of a point 16 ms apart and lifts at the
last. UIKit takes a drag's velocity from its last moves, so the content
stops where the finger left it. A rest without moves after it, or with one
move of a pixel to 3 points, still ended in the drag's full momentum, and three
moves of a pixel did at one start point in sixteen. `input.rs` sends the steps
through the target's transport, one request at a time per target, and
reports a `Dispatch`: the transport, the wait for its activation, and the
times from the request to the first event and to the last.

The transport comes from the host, never from a probe (`select.rs`): the
CoreSimulator that vends `SimDevice` injects the same input service into
every Simulator it boots, whatever the runtime, and opening one transport to
see whether it works can disable the other for the rest of the boot. From
CoreSimulator 1155.4 the plan is dtuhidd, with Indigo behind it only when
dtuhidd cannot be reached; before 1155.4, Indigo alone; an unreadable version
counts as current. The first transport of the plan that opens is used and the
rest are never touched. A dtuhidd session whose activation fails outright
counts as failed on that target, and the request moves to the next transport;
when every one has failed, input fails with `no_input_transport`, naming the
CoreSimulator, Xcode and runtime versions and each transport tried with its
reason. `status` names the transport and the rule that chose it.
`DriverOptions` can force one transport, which the Indigo and dtuhidd tests
do.

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
connections. `open_input` opens the session without any input, as the
daemon's `open` does while an app launches: the probe goes out, and the
call waits up to the time it is given for the reply, reporting the session
as `status` does; with no wait it returns once the probe is out (2.7 ms on
the spike Simulator) and the reply lands on its own. On the spike Simulator
it activated in 15.8 ms, and the first tap after it waited for no
activation and dispatched in 0.24 ms.

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

## Text

`type_text` types through key events or pastes, and reports which it did.
`Auto` types a string of at most 32 characters that a US keyboard can type,
with Shift where needed; anything longer, or with a character no key types,
is pasted. `TextMethod::resolve` says which a text gets before it is
entered. Keys go at a cadence, the pause from one key's release to the
next key's press: `type_text_at` takes one, and `type_text` types at the
native cadence, 15 ms (React Native's fields, below). A paste puts the text
on the device's general pasteboard and sends Command-V. The text goes there
in a `SET` to the guest's `dtpasteboardd`, the daemon behind `devicectl
device pasteboard copy`, reached as dtuhidd is: `-[SimDevice lookup:error:]`
for `com.apple.coredevice.feature.pasteboard`, then the `_4sim` libxpc
functions, over a connection of its own per paste. The message is `{command: "SET",
pasteboardName: "general", items: [{types, data}]}`, the text's bytes under
`public.utf8-plain-text`, `public.plain-text` and `public.text` as `devicectl`
writes them, and `dtpasteboardd` answers `SET_REPLY` with the pasteboard as
the `SET` left it, or an error naming what it could not decode. When the
guest does not vend the service, or the `SET` fails, `devicectl device
pasteboard copy` puts the text there instead, a process of its own per paste,
and an error names both failures. `simctl pbcopy` is not used: it goes to
`CoreSimulatorBridge` in the guest, which ignores it once `dtpasteboardd` has
started. On Xcode 27 with iOS 26.5 the `SET` took 8 to 35 ms with its lookup
and connection, against 118 to 250 ms for `devicectl`, on a host at a load of
4 to 24, and `pbcopy` took 299 to 468 ms on a fresh boot (task 10.31,
`reports/10.26-remeasure.md`).

iOS then asks whether the application may paste from `dtpasteboardd`, and the
text never arrives unless someone taps Allow Paste, so before a paste the
driver grants TCC's `kTCCServicePasteboard`, the "Paste from Other Apps"
setting, to every user application on the device through `-[SimDevice
setPrivacyAccessForService:bundleID:granted:error:]`, the call behind `simctl
privacy`. That changes the device's privacy settings; it does not restart
running applications. System applications are left alone and still ask. The
listing of the device's applications, 11 to 22 ms, and the grants, 3 to 21 ms
each, are made again only once the device's applications change: the driver
keeps the modification time of the directory that holds their bundles,
`Containers/Bundle/Application` under the device's data path, which every
install, upgrade and uninstall changes, whatever tool made it. An uninstall
drops its application's grant, so a change grants every user application
again.

A React Native field takes its keys through the app's JavaScript thread,
and a field whose value the app owns, on a screen that renders for longer
than the pause between keys, loses some: the acceptance run of
`rn-app-fixes` lost an address's "@" in 2 of 15 typed entries and a code's
digit in 1 of 5 (task 2.2). `Cadence::for_read` gives the fields of a read
React Native's cadence, 60 ms, when one of the read's classes starts with
`RCT`, and the native one otherwise; the daemon types at the cadence of its
latest read. The reader names an element the runtime leaves untyped by its
platform class, and React Native draws its screens under its own views: the
Expo fixture's username field lies under `RCTSurfaceHostingProxyRootView`,
`RCTSurfaceView`, `RCTRootComponentView` and its own
`RCTTextInputComponentView`, while the SwiftUI fixture's read names no
`RCT` class. The read's string table holds each class once, so the check
visits no node. Nothing in the mirror tells a React Native tree, since its
rules for React Native's shapes (task 7.11) apply to every tree, and a check
of the field's own ancestors would walk to the field for a precision that
buys only time: a native field on a React Native screen, an alert's, is
typed at 60 ms and loses nothing. The translator names no platform class,
so its reads keep the native cadence.

Task 3.5 of `rn-app-fixes` chose the cadence on the spike Simulator: 20
entries of the benchmark's 36 characters at each of three cadences, typed
into the focused, empty field, the cadences in turn within each trial, and
read back from the tree until the field held the whole entry for 200 ms
(`cadence::the_36_character_entry_at_three_cadences`, run by name), each
series started on a quiet host:

| Field | 15 ms | 30 ms | 60 ms |
|---|---|---|---|
| The Expo fixture's `login-username` | 19 of 20 whole | 20 of 20 | 20 of 20 |
| The same field, its value the app's and its label changing once it holds text, on a screen that takes 30 ms to render | 13 of 20 | 10 of 20 | 20 of 20 |
| Typing the 36 characters into the first, p50 / p95 | 637 / 651 ms | 1,180 / 1,218 ms | 2,236 / 2,264 ms |

The second field, the shape of the acceptance's fields, which no fixture
has, was a throwaway variant of the Expo fixture, not committed. Each of its
losses was one character, more at 30 ms than at 15, as keys then come about
once per render; the fixture's own loss at 15 ms kept only the first key.
The slowest cadence that lost nothing, 60 ms, is React Native's. A screen
that takes longer than that to render can still lose keys, which `fill`'s
read-back finds and its paste repairs; `type` reads nothing back.

A field that already holds text is filled by selecting it with Command-A
and typing over the selection, and there autocorrection rewrites what the
keys typed: the acceptance run of `rn-app-fixes` typed "Smoke Org" over an
organization's name and the field showed "smoke Org", four times out of
four (task 3.7). Task 4.5 traced it on the spike Simulator with throwaway
screens, not committed, of four fields, one per auto-capitalization setting,
autocorrection on as a React Native `TextInput` leaves it, each entry
following the daemon's readying of a field after another: a tap, reads
until a field has the focus, Command-A, then the keys. Over the field's
"Smoke Org", the field showed "Smoke" until the space, which autocorrection
replaced with "smoke "; with autocorrection off, nothing was rewritten (12
of 12). The shift the keyboard raises for a capitalizing field played no
part: a key sent with Shift landed as a capital in every setting, and one
sent without it in lower case, whatever the keyboard showed. In SwiftUI
fields, 20 entries each, what landed:

| Sent | None | Words | Sentences | All characters |
|---|---|---|---|---|
| Over "Smoke Org", every capital with Shift, as the driver types | "smoke Org" 20 | "smoke Org" 18, whole 2 | "smoke Org" 20 | "smoke Org" 16, whole 4 |
| Over "Smoke Org", the first letter without Shift | "smoke Org" 20 | "smoke Org" 20 | "smoke Org" 20 | "smoke Org" 20 |
| Into the emptied field, every capital with Shift | whole 20 | whole 20 | whole 20 | whole 20 |
| Into the emptied field, the first letter without Shift | "smoke Org" 20 | "smoke Org" 20 | "smoke Org" 20 | "smoke Org" 20 |
| Over "Smoke Org", the selection deleted and a read showing the field empty first | whole 20 | whole 19, "S" 1 | whole 20 | whole 20 |

React Native's fields, typed at 60 ms, lost the capital to the rewrite in 62
of 68 entries over "Smoke Org", and took all 80 whole with the selection
deleted and the field read empty first. The keyboard's window shows the
shift: its `shift` key carries `selected` and its letter keys are labelled
in capitals while the shift is up, in lower case otherwise, up in the
words, sentences and all-characters fields when they are empty. It does not
show the rewrite: with that read between Command-A and the first key, the
field without auto-capitalization was still rewritten ten times out of ten
while its shift read down. Reading the first letter back after its key does
not see it either, since the rewrite comes at the space. So the daemon,
which selects the field's text before `fill`'s keys, deletes the selection
too when the entry goes in as keys and starts with a capital, and the keys
wait for a read that shows the field empty, at most 1 s
(`empties_first` in `crates/daemon/src/devices/simulator.rs`): the delete
and one to three reads, 0.28 s more a fill at p50 through the daemon at
loads of 7.7 to 9.7. `fill` itself empties a field that holds text before
keys of any first letter, an empty entry the Simulator selects and deletes
the field's text for, then waits for the replica to show the field empty
before the keys go into the field the emptying gave the focus, so that the
read-back compares with the emptied field (task 5.2 of `rn-app-fixes`): a
refill of the code a code field held, its name spelling the same digits
before and after, could not be read back. Through the daemon on the spike
Simulator a refill of the Expo fixture's username field took 0.93 s at p50
against 0.665 s before, a first fill 0.655 s against 0.66. Text into the
field in focus, which `fill` enters so after the emptying and for an
identifier on a view holding no field, is readied as text into a field the
input taps, Command-A first, but for the tap (`readying`). A paste and
`type`, which selects nothing, keep the readying they had.

The tests read a field back by putting a sentinel on the pasteboard and
sending Command-A and Command-C: 500 characters pasted in 48 ms, 204 ms through
`devicectl`, and 25 typed in 0.49 s came back exactly.

## Compatibility

The driver supports Xcode 26.0 up to but not including 28 and refuses to
work outside that range: every command fails with `unsupported_xcode`, which
names the detected Xcode and the range and points here. The driver still
opens, so `compatibility()`, the table the CLI's `doctor` prints, answers
anyway. The Xcode version comes from the selected developer directory's
`version.plist` (`DEVELOPER_DIR`, else `xcode-select`); the CoreSimulator
version from the bundle that vends `SimDevice`, which decides the input
transport.

- Validated: Xcode 27.0 (27A266a), CoreSimulator 1171.7, SimulatorKit 1005.2
  and iOS 26.5 (23F77), where every spike and every integration test ran.
- Xcode 26.x is in the range because the design keeps Indigo as the input for
  its CoreSimulator, which predates 1155.4, and CI builds on it, but nothing
  has run there: input would be touch only, since keys, buttons and text
  need dtuhidd, and `doctor` marks that row limited.
- Xcode 28 and later, and 16 and earlier, are refused until validated.

`compatibility()` reports Xcode, CoreSimulator, SimulatorKit, the installed
runtimes, the input transport with the rule that picked it, the `_4sim` XPC
symbols, the tree reader and translator, and the frame encoder, each
validated, supported, limited or unsupported, followed by every private call
the driver makes with the image it lives in and where it was validated:

| Private calls | Image | Validated on |
|---|---|---|
| `SimServiceContext`, `SimDeviceSet` and `SimDevice` lifecycle: create, delete, boot, `bootStatus`, shutdown, install, launch, terminate | CoreSimulator | Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: spike S2, `tests/lifecycle.rs` |
| `-[SimDevice dataPath]` | CoreSimulator | `tests/lifecycle.rs` |
| `-[SimDevice installedAppsWithError:]`, `-setPrivacyAccessForService:bundleID:granted:error:` | CoreSimulator | `tests/text.rs` |
| `-[SimDevice lookup:error:]` for `com.apple.coredevice.feature.pasteboard`, the three `_4sim` libxpc functions, its `SET` command and `SET_REPLY`; `devicectl device pasteboard copy` when they fail | CoreSimulator, libxpc, `dtpasteboardd` of CoreDevice 642.16 in the guest | task 10.31's benchmark, `tests/text.rs` |
| `-[SimDevice io]`, `-ioPorts`, the display descriptor, `-framebufferSurface`, the SimScreen callbacks | CoreSimDeviceIO, ROCKit proxies | spike S2, `tests/frames.rs` |
| `-[SimDevice lookup:error:]`, the three `_4sim` libxpc functions | CoreSimulator, libxpc | spike S3, `tests/dtuhid.rs` |
| `SimDeviceLegacyHIDClient`, `IndigoHIDMessageForMouseNSEvent` (six parameters) | SimulatorKit 1005.2 | spike S3, `tests/indigo.rs` on a fresh boot |
| `-[SimDevice spawnWithPath:options:terminationQueue:terminationHandler:pid:error:]` | CoreSimulator | spike S4, `tests/tree.rs` |
| In the guest: `XCTAccessibilityFramework -initForRemoteAccess`, `-userTestingSnapshotForElement:options:error:`, `XCAccessibilityElement`, `XCTDefaultSnapshotParameters`, `XCAXAccessibilityAttributesForStringAttributes`; `+[AXElement primaryApp]`, `AXValueGetValue`; `-[AXSettings setAutomationEnabled:]`, `_AXSAutomationEnabled` | XCTAutomationSupport, AXRuntime, AccessibilityUtilities, libAccessibility of the iOS 26.5 runtime | spike S4, `tests/tree.rs` |
| `AXPTranslator` and `AXPMacPlatformElement` attributes, `-[SimDevice sendAccessibilityRequestAsync:completionQueue:completionHandler:]` | AccessibilityPlatformTranslation 545.18, CoreSimulator | spike S4, `tests/tree.rs` (fallback) |

## Tests

`cargo test -p simulator` runs the unit tests. They need macOS with Xcode,
since some load CoreSimulator, but no Simulator. On other hosts only the
platform-neutral unit tests run: the tests in `src/macos/` and the
integration tests in `tests/` (each file `#![cfg(target_os = "macos")]`)
are compiled out.

The integration tests in `tests/` drive real Simulators. Each checks
`MOBILE_AGENT_SIM_UDID` first; when it is unset, as in CI, the test prints why
it skipped and passes. To run them, name a booted iPhone 17 Simulator with
both fixtures installed and the Expo fixture on its Login tab:

```sh
MOBILE_AGENT_SIM_UDID=<udid> cargo test -p simulator
```

The tree tests use the SwiftUI fixture on the same Simulator and put the
Expo fixture back on Login when they end; so do the cadence tests, which
type into both fixtures' username fields and read them back from the tree,
and whose measurement, `cadence::the_36_character_entry_at_three_cadences`,
is ignored, to be run by name on a quiet host; the metadata tests use both
fixtures, and Contacts, which every Simulator has, for a sheet: they open
its New Contact sheet, close it without saving and terminate the app. The permission alert tests reset the fixture's location
permission before and after, with `simctl privacy`. SpringBoard keeps a
permission alert up after its application is gone, over whatever runs
next, so the metadata test dismisses it whether its checks passed or not.

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
| No test runner and no per-app build | Fresh Simulator | `tree::fresh_simulator` |
| No test runner and no per-app build | Spawned helper is reported | `tree::target_status_reports_the_reader_and_its_readiness`, `macos::reader::tests::the_build_embedded_a_reader_for_the_simulator` |
| Foreground tree read | System alert in front | `tree::a_permission_alert_is_reported_as_system_owned`, `tree::tests::a_system_alert_belongs_to_system_ui` |
| Foreground tree read | Sparse tree flagged | `tree::sparse_tree_flagged`, `tree::tests::an_application_root_alone_is_sparse`, `tree::tests::the_translator_and_truncation_degrade_the_read_with_reasons` |
| Foreground tree read | Identifiers, labels, values, roles and frames (task 6.6) | `tree::swiftui_read_returns_identifiers_and_frames`, and in `reader/`: `elements::tests::classes_are_type_names_or_the_platform_class_of_untyped_elements`, `elements::tests::labels_values_and_placeholders_become_descriptions_and_text`, `elements::tests::flags_come_from_the_type_and_traits`, `elements::tests::bounds_are_pixels_rounded_from_points` |
| Foreground tree read | Window owner, modal alert windows and windows without change events (task 6.10) | `metadata::a_permission_alert_window_is_owned_by_springboard`, `metadata::an_in_app_alert_window_is_modal_and_the_application_window_is_not`, `tree::tests::a_modal_window_above_the_application_is_not_taken_for_the_foreground`, and in `reader/`: `elements::tests::system_ui_owns_its_windows_by_process_name_and_keeps_its_alert`, `elements::tests::an_applications_windows_have_no_owner`, `elements::tests::an_in_app_alert_moves_to_a_modal_window_above_the_application`, `elements::tests::an_alert_in_a_window_of_its_own_moves_with_its_presentation`, `elements::tests::an_alert_that_is_not_on_screen_stays_where_it_is`, `elements::tests::the_keyboard_stays_above_an_alert_window`, `elements::tests::every_window_says_it_sends_no_change_events` |
| Foreground tree read | The keyboard read whole, sheets that block the screen as modal windows (task 6.12) | `metadata::a_read_with_the_keyboard_up_is_whole_and_holds_the_keys` (both fixtures), `metadata::a_sheet_over_the_screen_is_a_modal_window` (Contacts' New Contact sheet), `tree::tests::a_modal_window_cuts_the_windows_below_it`, and in `reader/`: `elements::tests::a_read_with_the_keyboard_up_is_not_truncated`, `elements::tests::an_alert_moved_to_its_own_window_leaves_the_read_whole`, `elements::tests::a_sheet_that_takes_touches_moves_to_a_modal_window_above_the_screen`, `elements::tests::blueskys_bottom_sheet_is_a_modal_window_and_its_presenter_is_not`, `elements::tests::a_sheet_that_lets_touches_through_stays_in_the_application_window`, `elements::tests::a_dimming_view_that_does_not_span_the_screen_dims_nothing`, `elements::tests::an_alert_over_a_sheet_is_the_top_most_modal`, `elements::tests::system_ui_keeps_a_sheet_in_its_window` |
| Foreground tree read | Scroll semantics of scroll views (task 6.10) | `metadata::a_list_scroll_view_reports_its_axis_and_whether_it_is_at_its_top`, and in `reader/`: `elements::tests::a_list_at_its_top_scrolls_forward_only`, `elements::tests::a_list_scrolled_down_scrolls_both_ways`, `elements::tests::a_list_at_its_end_scrolls_back_only`, `elements::tests::content_that_fits_its_visible_frame_does_not_scroll`, `elements::tests::content_taller_than_the_visible_frame_scrolls_though_the_frame_holds_it`, `elements::tests::a_carousel_scrolls_horizontally`, `elements::tests::content_that_scrolls_both_ways_names_no_axis`, `elements::tests::elements_without_a_scroll_area_have_no_scroll_semantics` |
| Foreground tree read | Covered content cut under alerts, the translator fallback (tasks 6.6 and 6.11) | `tree::in_app_alert_content_is_cut`, `tree::the_translator_stands_in_when_the_reader_cannot_run`, `tree::a_build_without_the_reader_reads_through_the_translator` (without the `reader` feature), `macos::reader::tests::a_spawn_can_record_on_the_slot_while_it_runs`, `macos::reader::tests::a_build_without_the_reader_feature_embeds_none` (without the feature), `tree::tests::what_the_alert_covers_is_cut_and_the_payload_keeps_it`, `tree::tests::an_alert_is_found_with_its_title_message_and_buttons_once_each` |
| Foreground tree read | The reader's protocol link (task 6.6) | in `reader/`: `guest::server::tests::a_snapshot_request_is_answered_after_the_handshake`, `guest::server::tests::a_request_during_a_read_is_answered_busy`, `guest::server::tests::the_connection_stays_alive_through_a_read_longer_than_the_peer_timeout`, `guest::server::tests::the_reader_stops_when_the_host_goes_away` |
| Input transport selection | Input before activation | `dtuhid::input_before_activation`, `dtuhid::the_first_session_of_a_boot_waits_for_dtuhidd_to_start` |
| Input transport selection | Two runtimes on one host | `transport::two_simulators_get_the_hosts_transport` (both on iOS 26.5, the host's only runtime), `select::tests::the_host_core_simulator_selects_dtuhidd_from_1155_4`, `select::tests::an_unreadable_version_is_taken_as_current`, `select::tests::a_forced_transport_is_the_only_one_tried`, `select::tests::the_fallback_is_opened_only_when_the_first_fails`, `select::tests::a_transport_that_failed_on_the_target_is_skipped` |
| Input transport selection | Unsupported combination | `select::tests::the_unsupported_combination_names_every_version_and_transport_tried` |
| Input transport selection | The input session opened ahead of input (task 6.13) | `dtuhid::the_input_session_opens_and_activates_without_input`, `dtuhid::an_input_session_opened_without_waiting_activates_on_its_own` |
| Input actions | Warm tap latency | `dtuhid::warm_tap_latency` |
| Input actions | Taps, keys and buttons over one dtuhidd connection (task 6.4) | `dtuhid::a_tap_lands_on_the_fixture_button`, `dtuhid::one_connection_carries_touch_keys_and_buttons` |
| Input actions | Taps and swipes over the Indigo fallback (task 6.3) | `indigo::a_tap_lands_on_the_fixture_button`, `indigo::a_swipe_scrolls_the_list` |
| Input actions | A swipe that comes to rest before the finger lifts (task 10.16) | `gesture::tests::a_swipe_that_rests_reaches_its_end_then_creeps_a_point_at_a_time_and_lifts`; on the Simulator through the daemon, `devices::scroll_until_finds_a_mid_list_row_on_a_simulator` |
| Frames and screenshots | Frame stream during animation | `frames::frame_stream_during_animation`, `frames::frames_stop_when_the_display_is_static`, `frames::frames_sustain_30_fps_under_a_fifth_of_a_core`, `frames::a_screenshot_is_timestamped_at_the_display_size` |
| Text entry | Long string | `text::long_string`, `text::a_short_string_is_typed_through_keyboard_events`, `text::text_no_key_types_is_pasted`; the paste's way to the pasteboard and its grants (task 10.31): `macos::pasteboard::tests::a_paste_goes_to_dtpasteboardd_and_leaves_devicectl_alone`, `macos::pasteboard::tests::a_guest_that_does_not_take_the_set_gets_devicectl`, `macos::pasteboard::tests::when_devicectl_fails_too_the_error_names_both`, `macos::pasteboard::tests::the_set_carries_the_text_under_each_plain_text_type`, `macos::pasteboard::tests::a_set_reply_succeeds_and_any_other_answer_says_why_not`, `macos::pasteboard::tests::the_second_paste_lists_nothing`, `macos::pasteboard::tests::a_changed_application_directory_lists_and_grants_again` |
| Text entry | Capital first letter over what the field held (task 4.5 of `rn-app-fixes`) | in the daemon `devices::simulator::tests::a_capital_first_letter_lands_when_fill_types_over_what_the_field_held` (on a fake of the keyboard as measured), `devices::simulator::tests::only_keys_whose_first_letter_is_a_capital_empty_the_field_first`, `devices::simulator::tests::the_focused_field_shows_its_text_or_nothing`, and `model::tests::auto_types_a_short_typeable_string_and_pastes_any_other`; no fixture's field has autocorrection on, so the measurement and the ten fills behind the rule ran on throwaway screens (task 4.5) |
| Text entry | A field emptied before keys, and text into the field in focus readied as into one the input taps (task 5.2 of `rn-app-fixes`) | in the daemon `handlers::act::tests::a_keyed_fill_into_a_field_holding_text_empties_it_and_types_once_it_reads_empty`, `handlers::act::tests::a_refill_of_the_code_a_field_holds_empties_it_first_and_is_verified_through_the_name` and `devices::simulator::tests::text_into_the_field_in_focus_is_readied_as_text_into_a_field_it_taps`; on throwaway screens through the daemon, the record of task 5.2 |
| Text entry | React Native field typed whole (task 3.5 of `rn-app-fixes`) | `cadence::a_react_native_field_takes_the_36_character_entry_whole_twenty_times`, `cadence::a_react_native_field_is_typed_at_its_cadence_and_a_native_one_at_the_native_cadence`, `cadence::tests::a_read_of_react_natives_views_types_at_the_react_native_cadence`, `cadence::tests::a_native_read_types_at_the_native_cadence`, and in the daemon `devices::simulator::tests::text_is_typed_at_the_cadence_the_latest_read_calls_for`; the measurement behind the cadence, `cadence::the_36_character_entry_at_three_cadences` (ignored) |
| Parallel Simulators | Two sessions | `transport::two_sessions`, `tree::each_simulator_has_its_own_reader` |
| Failure containment | Framework call throws | `containment::a_tree_read_that_throws_takes_only_its_target_out_of_service`, `containment::framework_call_throws`, `objc::tests::an_objective_c_exception_becomes_an_error_naming_the_call`, `objc::tests::a_rust_panic_in_a_call_becomes_an_error_naming_the_call`, `macos::driver::tests::reader_failures_become_driver_errors`, and in `reader/`: `guest::objc::tests::an_exception_becomes_a_failure_naming_the_call` |
| Xcode compatibility statement | Unsupported Xcode | `compat::tests::unsupported_xcode`, `macos::driver::tests::commands_refuse_an_xcode_outside_the_range`, `compat::tests::a_host_without_xcode_is_refused`, `compat::tests::xcode_26_and_27_are_supported`, `compat::tests::the_table_renders_every_row_and_call_site`, `macos::driver::tests::this_host_is_within_the_range_and_the_table_says_why` |
