# ios

Drives a physical iPhone over USB from the host (design D18 to D25, spec
`ios-device-driver`): discovery and readiness over usbmuxd and lockdown,
the CoreDevice tunnel and the developer services over it, the developer
disk image mounted when it is missing, app lifecycle through DVT and the
installation proxy, the runner's session through the test manager, input
through the disk image's `dtuhidd`, frames from screenshots polled on two
connections, and the containment of a phone that drops. Everything runs in
the caller's process through the `idevice` crate: no root, no TUN
interface, no helper process in another language, and no `devicectl`
except as the disk image mount's fallback. The tree is read by the runner
`agent-ios` builds (its README); this crate starts it and carries its calls.

`IosDriver` implements the driver traits, `Devices`, `Lifecycle`,
`Session`, `Input` and `Frames`. They carry no `idevice` types, so the
daemon can hold the driver behind them and adapt it to its own driver
contract and the mirror's backend traits, as it does the Simulator driver
(Driver contract). On hosts other than macOS the crate still builds:
`IosDriver` exists there, and `open` fails with `platform_unsupported`.

## Layout

```
src/
  driver.rs        the driver traits: Devices, Lifecycle, Session, Input, Frames
  model.rs         the values they take and return
  error.rs         errors, each with a stable code
  readiness.rs     the readiness states, their codes and steps, and what each operation needs
  discovery.rs     usbmuxd's listing to USB entries, lockdown's values to a device, its probe and its screen
  identity.rs      the host's signing identity for the runner, through security
  bundle.rs        the app bundle an install takes, checked on the host
  services.rs      the RSD services the driver needs
  compat.rs        the supported iOS range, the input path's gate and the call-site table
  runner.rs        what the runner's hello must name, and how the test manager's failures read
  touch.rs         touches as timed mainTouchscreen reports: taps, presses, swipes that rest
  frames.rs        the frame pipeline: PNG decode, the change below the status bar, FrameStream
  version.rs       dotted version comparison
  unsupported.rs   the driver on hosts other than macOS
  macos/
    host.rs        the Host and Link seams: usbmuxd, the lockdown probe, the lock, the screen,
                   the installation proxy, the runner's port
    tunnel.rs      the Opener seam and the Tunnel: CoreDeviceProxy, jktcp, RSD, reconnection
    image.rs       the Mounter seam: the disk image's check and mount, idevice then devicectl
    dvt.rs         DVT's channels on connections of their own, the calls written by hand
    runner.rs      the TestManager seam and the runner's session: start, supervisor, calls, stop
    connection.rs  the protocol's HostSession over the runner's socket, on a task of its own,
                   and what tells the runner stalled while its connection lives
    hid.rs         dtuhidd's two connections and the reports and events each input writes
    capture.rs     the two screenshot pollers and their reopen, the pipeline's thread,
                   screenshots and their fallback, frame paths
    driver.rs      IosDriver over Core, which keeps each target's tunnel, channels, runner,
                   HID connections and lock session, and contains a phone that drops
    fake.rs        the tests' fake device: RSD, DVT over DTX, the link, the mounter, the screen
    fake_runner.rs the tests' fake test manager and runner (the protocol's AgentSession)
    fake_xpc.rs    the tests' RemoteXPC services: dtuhidd, screen capture, the display service
    fake_fixture.rs the tests' SwiftUI fixture, launched, tapped and read on the fake device
    containment.rs failure containment's tests
    setups.rs      the device tests' SwiftUI helpers run on the fake device
tests/
  common/          gating on MOBILE_AGENT_IOS_UDID, the quiet-host guard, and the SwiftUI
                   fixture's helpers (swiftui.rs), which src/macos/setups.rs runs too
  discovery.rs     task 3.1 on the device
  tunnel.rs        task 3.2 on the device
  lifecycle.rs     task 3.3 on the device, and the install series (ignored)
  runner.rs        task 3.4 on the device: fifty commands, one runner process
  input.rs         task 3.8 on the device, the input series and the lock button (ignored),
                   and task 9.7's 120 emptying fills in one session (ignored)
  frames.rs        task 3.5 on the device, the measured minute (ignored), and task 9.8's
                   connection closed and opened again (ignored)
  containment.rs   tasks 3.4 and 3.6 on the device: the pulled cable (ignored)
  fixtures/        recorded values: usbmuxd's listing, lockdown's values, CopyDevices,
                   RSD's handshake, and the disk image's build manifest cut down
```

## Building

`idevice` is pinned to `=0.1.68`, as its README asks, since breaking changes
come with every point release before 0.2. Its features are `usbmuxd`,
`core_device_proxy`, `tunnel_tcp_stack`, `rsd`, `xpc`,
`mobile_image_mounter`, `tss` (the signing server's ticket for the mount),
`installation_proxy`, `afc` (the install's upload), `dvt`, `xctest` (the
runner's session), `core_device` (screen capture, the display service) and
`display_stream`, which carries the HID clients though no stream is
opened; its default TLS is rustls on aws-lc-rs. Lockdown needs no feature,
and pairing (`pair`) is not used: whoever holds the phone trusts the Mac.
`idevice` and tokio are macOS-only dependencies. Built everywhere: `plist`,
`protocol` (the runner speaks the device protocol), `png` (the device's
screenshots) and `simulator`, for the frame pipeline's `scale_bgra`,
`pixels_hash`, `fit_within` and JPEG encoder, without its default features
and with its dependency-free `jpeg-encoder`, so the crate builds without
libjpeg-turbo; in the workspace the daemon's turbojpeg comes in beside it.
`libc` is a dev-dependency, for the frames series' CPU time.

The driver owns a tokio runtime of two worker threads, as the Android
driver's emulator controller owns one; its traits are synchronous and each
call blocks on that runtime from a thread outside it. Each runner session's
supervisor runs on a thread of its own, driving its future with the
runtime's handle, since rustc cannot show the generic restart's future
`Send` for every lifetime it borrows; the frame pipeline runs on a thread of
its own beside the pollers' tasks.

## Driver contract

The daemon registers the driver as `ios` when `MOBILE_AGENT_DRIVERS` names
it (`crates/daemon/src/devices/ios.rs`, task 5.1). The traits map onto its
contract (`crates/daemon/src/driver.rs`) and the mirror's backend
(`crates/mirror/src/backend.rs`) as the Simulator driver's do:

| Daemon or mirror | This crate |
|---|---|
| `Driver::discover` | `Devices::devices`: a `TargetInfo` per device, its state the readiness code and its advice the readiness message |
| `Driver::forget`, which `devices` calls before it lists | `IosDriver::forget_identity`: the signing identity kept is checked again |
| `Driver::attach` | `Devices::connect`, then `Session::attach` with the runner the daemon built (`RunnerSpec`: the version, the bundle to install in its place) and the way input goes (`InputChoice`, by the iOS version by default) |
| detach, daemon stop | `Session::release` |
| `AppControl::launch` | `Lifecycle::launch`, `relaunch` as `terminate_running` |
| `AppControl::terminate` | `Lifecycle::terminate` |
| `Report::report` | `Devices::status`: the tunnel, the disk image, the connections open and `runner` (state, agent, restarts, handshake retries, leftovers ended, start time, last death); `FrameStream::stats`: each poller's captures, reopens and why it was left out |
| `Attachment::display`, `TreeRead::scale` | `Devices::screen`: lockdown's pixels and scale (750 by 1334 at 2 on the SE), read once per connection |
| `Driver::compatibility` | `ios::call_sites` and the supported range |
| `TreeSource::read` | `Session::call` with `Call::Snapshot`; the runner's reply is the protocol's snapshot (group 4) |
| `InputSink::perform`: `Input::Tap` | `Input::tap`, or `Input::press` with its hold |
| `Input::Swipe` | `Input::swipe`, `Release::Resting` as `Swipe::resting` |
| `Input::Key` | `Input::key` (an Indigo keyboard event), `Input::button` for the hardware buttons |
| `Input::Text` | `Session::call` with `Call::TextSet` through the runner (D22); `Input::keys` types on a virtual keyboard |
| `InputAck` | `Dispatch`: acknowledged when the last report is written, the path taken |
| `EventStream::next` with `StreamItem::Frame` | `FrameStream::next`; `FrameMeta { seq, at, hash, width, height }` from `Frame { sequence, arrived, hash, width, height }`, rotation R0 |
| `FrameSource::latest` | `FrameStream::latest` |
| `FrameSource::encode` | `FrameStream::encode(sequence, width, height, quality)`, a held frame as JPEG |
| the daemon's screenshot | `Frames::screenshot` |

Every method names its device by identifier: names are not unique, so no
method takes one, and a name fails as `not_found`.

## Readiness

`devices` lists every iPhone attached over USB with its identifier, its
name, model (`ProductType`, such as `iPhone14,6`) and iOS version from
lockdown, and its readiness: the first state it has not passed, in the order
a person makes a device ready. Inside the session it starts for readiness,
the probe also reads the battery from lockdown's `com.apple.mobile.battery`
domain (`DeviceInfo::battery`: the charge, whether it charges and whether the
phone is on external power), which the daemon's `status` names as the
attach found it and the harness records in a run's provenance. A phone with Wi-Fi sync is listed twice by
usbmuxd, on USB and on the network; the USB entry is taken, and a device
listed only on the network is not listed.

| State | Code | When | The step its message names |
|---|---|---|---|
| unsupported iOS | `unsupported_ios` | lockdown's `ProductVersion` is outside iOS 17.4 up to but not including 27 | none; the version and this README's Compatibility section |
| not trusted | `not_trusted` | usbmuxd holds no pairing record (`ReadPairRecord`), the device refuses it (`InvalidHostID`), or lockdown starts no session | unlock it, connect it over USB, tap Trust and enter the passcode; Finder starts the pairing when nothing else asks |
| Developer Mode off | `developer_mode_off` | `DeveloperModeStatus` in `com.apple.security.mac.amfi` is not true | Settings > Privacy & Security > Developer Mode, restart, confirm with the passcode |
| disk image unmounted | `disk_image_unmounted` | the image mounter's `CopyDevices` lists no developer image, or RSD lists no `com.apple.dt.testmanagerd.remote` when the runner starts | connecting mounts the selected Xcode's, which needs this Mac online for Apple's signing server; `xcrun devicectl device info ddiServices --device <identifier>` mounts it by hand |
| locked | `device_locked` | `PasswordProtected` is not false | unlock it, and set Auto-Lock to Never for long runs |
| no signing identity | `no_signing_identity` | no valid Apple Development identity of the team `MOBILE_AGENT_IOS_TEAM` names in the keychain | create the development certificate in Xcode with the team's account and export the team's identifier |
| UI automation not approved | `automation_not_enabled` | a runner's start: XCTest logged "enabling automation mode" and no hello followed within 3 s, or the session ended after it | on the phone, approve "Enable UI Automation" with Touch ID or the passcode; XCTest waits 60 s for it |
| ready | `ready` | none of the above | |

A value that could not be read does not pass its check. The device's state
is read on the device, the identity on the Mac: `security find-identity -v
-p codesigning` lists the valid identities, and `security find-certificate
-a -Z -p -c "Apple Development"` their certificates, whose subject's
organizational unit is the team (the code in the identity's name is the
developer's, not the team's). The check costs about 40 ms per command, and
each command is ended once it has run 5 s (`SECURITY_TIMEOUT`), the
identity then unreadable and the state `no_signing_identity` naming the
command and the time: the measured host's endpoint security held process
launches for up to 130.6 s (task 7.1). The driver checks the identity once
and keeps it while it is found: a check that failed is made again at the
next use, and `IosDriver::forget_identity` makes the next listing check it,
as the daemon's `devices` asks (design D26 of physical-ios).

An operation needs a prefix of that order (`Need`), and fails with the
code of the first state it needs that the device has not passed;
`require(target, need)` reads the device afresh and does this for any
level. A command connects the device on its first use of it, which needs
what the developer services need: trust, Developer Mode and the disk image,
which it mounts when missing. A launch also needs the device unlocked,
read before every launch, and the runner's session the signing identity: a
locked phone still connects, installs and lists its processes, and a launch
on it fails with `device_locked`. The approval of "Enable UI Automation" is
no probe's to read: only a start shows it, so `devices` never lists it, and
the target's status reports the runner `awaiting_approval` while a start
waits for it.

## Transport

`connect` reads the device, fails on what stands before its developer
services, mounts the developer disk image when the image mounter lists
none, then opens the CoreDevice tunnel and DVT's process control unless
they are open, and keeps them for the target's lifetime: lockdown's
`CoreDeviceProxy` over usbmuxd carries the tunnel, jktcp ends it in this
process, and RSD's handshake over it lists the services by name and port.
`status` reports what is kept, the tunnel's opens, how long the last took,
the services listed and those needed but missing, the disk image and the
connections open, from a snapshot it reads while a command runs.
`reset_target` closes them, the runner's session with them.

The services the driver needs are the test manager
(`com.apple.dt.testmanagerd.remote`), DVT
(`com.apple.instruments.dtservicehub`), CoreDevice's app service and screen
capture service, and the two HID services (`hid.indigo`,
`hid.universalhidservice`); `status` names those the handshake does not
list. Without the disk image the handshake lists fewer: 61 services instead
of 82 in spike S10, the test manager and the HID services among those gone.

A service connection that finds the tunnel dropped (jktcp answers "adapter
closed") closes it and opens a new one, once; a device that cannot be
reached again fails as `disconnected` with the cause, and a service the
handshake does not list as `service_missing`, without a reopen. Each DVT
channel lives on a connection of its own, served by a task that owns the
connection and sends one call at a time, since `idevice`'s channel borrows
its connection: a call whose connection was gone before it was written goes
again on a new one, and a reply that never comes ends the channel, so a
late reply cannot answer the next call. The messages the device starts on a
channel are passed over, DTX numbering their conversation even and a
reply's odd: process control sends what a launched app prints as
`outputReceived:fromProcess:atTime:`, and the Expo fixture's output came
before its launch's reply, which the driver had read as the launch's
answer, every later reply on the channel one call late.

The disk image is mounted through `idevice`'s personalized mount first,
within 60 s: the image and trust cache the build manifest of the selected
Xcode's image names for the device's `ProductType`
(`/Library/Developer/DeveloperDiskImages/iOS_DDI/Restore`), the chip
identifier from lockdown and the ticket from Apple's signing server. Then
through `xcrun devicectl device info ddiServices --auto-mount-ddis` within
120 s, which also mounts the Cryptex1 form a device outside the manifest
takes. Each attempt is timed and written to standard error, and counts only
once `CopyDevices` lists the image, since S10's call never returned though
the image was mounted by then. When neither mounts it, `connect` fails with
`disk_image_unmounted`, each attempt and its failure named, and says when a
failure shows this Mac offline that the mount needs the signing server. No
mount ran on the device in this change: the image was mounted throughout,
and unmounting it is not this driver's to do.

## Lifecycle

- **install** checks the bundle on the host (a `.app` directory with its
  `CFBundleIdentifier` and executable, built for `iPhoneOS`, signed and
  provisioned; a Simulator build is refused naming the device
  destination), then uploads it to `PublicStaging` through AFC and installs
  it through the installation proxy, over usbmuxd rather than the tunnel,
  whose stack sends one segment at a time.
- **launch** reads lockdown's `PasswordProtected` first, on a lockdown
  session kept for it, and fails with `device_locked` without trying a
  launch: on a locked phone DVT refuses without saying why, and CoreDevice's
  app service refused at some moments and launched behind the lock screen
  at others (S9). With `terminate_running` it then ends the running
  instance as `terminate` does, and launches without DVT's `KillExisting`:
  an app relaunched with it kept a black window on iOS 26.6.2, Settings in
  S11 and the SwiftUI fixture in task 3.8's device check, whose taps then
  reached nothing, where a launch after the instance ended drew. It sends
  `launchSuspendedProcessWithDevicePath:bundleIdentifier:environment:arguments:options:`
  with the arguments and environment archived, the app started running and
  `KillExisting` false, and returns the pid of the reply. Without
  `terminate_running` a running app keeps its process: DVT answered the
  running instance's pid, in 55 ms. A refusal is `app_not_installed` when
  the installation proxy does not list the app, `launch_failed` with DVT's
  `NSError` otherwise, read from the reply's archive since `idevice`'s
  decoder leaves it unresolved.
- **running** reads DVT's process list for the app's bundle identifier,
  which iOS 26.6.2 gives application processes; an app's extensions have
  identifiers of their own.
- **terminate** finds the app's processes in that list, sends each
  `sendSignal:toPid:` with SIGKILL and the pid archived, never twice, and
  returns once each has ended, asking `isRunningPid:` pid by pid. An app
  that is not running is left alone. `idevice` 0.1.68's `kill_app` is never
  called: it sends `killPid:` with the pid as a primitive integer, which on
  iOS 26.6.2 left the app running and dropped every DVT connection (S9).

On the iPhone SE (3rd generation) with iOS 26.6.2, in the device tests'
functional runs (the host's 1-minute load 8 to 13 from other work on the host, so
these are not measurements): the first launch after an install took 2.2 s
three times out of three and a relaunch 26 to 41 ms with the lock read; one
read of the process list, about 580 processes through the tunnel, 474 to
728 ms; terminate 526 ms, nearly all of it that read. With the running
instance ended first, a relaunch took 580 ms and a first launch 865 ms
(load about 6), the list's read included. The install is timed in Numbers.

## Runner session

`Session::attach` starts the runner (design D20) and returns once it said
hello with the version the daemon asks for:

1. The device is connected and must be ready for a session: unlocked, with
   a signing identity on the host. RSD must list
   `com.apple.dt.testmanagerd.remote`, which the disk image carries;
   without it the attach fails with `disk_image_unmounted`.
2. Runner processes left from a lost session are ended first, found in
   DVT's process list by the runner's bundle identifier and signalled as
   `terminate` signals: after S10's pulled cable the runner outlived the
   test manager's session and answered the next attach's hello from the
   stale process, then closed the connection. The pids are reported as
   `leftovers`.
3. The installation proxy looks the runner up; a missing runner is
   installed from the bundle the `RunnerSpec` names, or reported as
   `runner_not_installed`. `idevice`'s `XCUITestService::run` starts the
   xctest session on a task kept for the session: two testmanagerd
   connections, the IDE sessions, the runner launched through process
   control and authorized, the test plan started. A listener writes what
   XCTest reports to the session's log.
4. The driver connects to the runner's loopback port 27183 through
   usbmuxd's `Connect` every 25 ms until the protocol's `HostSession` gets
   the hello. A hello naming another version stops the session, ends the
   runner, installs the bundle given and starts again, once; without a
   bundle the attach fails with `runner_version_mismatch`, naming both.
5. When the session ends before the hello, its error says why: "service not
   found" is the missing test manager; a handshake that timed out is made
   again after 2 s while the attach's time lasts (150 s by default: the
   test manager stopped answering for a minute or two after runners were
   ended mid-start); XCTest's error 103, which `idevice` calls an untrusted
   certificate, is a test bundle that did not load, an arm64e executable in
   S10; anything else is `runner_start_failed` with the reason. A session
   that asked for automation mode and ended without the hello is
   `automation_not_enabled`; while it waits the status says
   `awaiting_approval`, 3 s after the request, where the hello came 430 ms
   after it in S10.

A session that serves is kept: `attach` again reports it. A supervisor
watches the runner's connection and the test manager's session: when
either ends, the runner died, and it is started again once, through the
same steps, the restart counted in the status; a runner that dies again,
or whose restart fails, fails the session (`runner_died`, or `disconnected`
when the device went away), and a new attach starts over. `Session::call`
sends a protocol call and waits within its timeout; a call made while the
runner starts again waits for it, and fails with `timeout`, which can be
repeated, when its time runs out first. A call whose request was written
before the connection closed fails with `disconnected`, since the runner
may have acted on it. The runner's own errors pass through with the
protocol's codes (`deadline_exceeded`, `busy`, `window_changed`, which can
be repeated). `Session::release` ends the supervisor, which closes the
connection and the test manager's session; XCTest ends the runner, and any
runner process still running is signalled.

A runner that stops answering while its connection lives is treated as
died too (design D26 of physical-ios). Its worker runs one call at a time
and its session answers pings meanwhile, so a worker that never comes back
from an XCTest call, as the phone's did from the 43rd emptying text set of
every session in task 9.6 (`reports/11.1-ios-device/probes/text-hang-swiftui-*.jsonl`),
leaves the connection alive, answering that call `deadline_exceeded` at its
deadline and every later one `busy`. The connection counts the runner
stalled at the second call in a row answered `deadline_exceeded` or `busy`
with no other answer between them (`STALL_CALLS`), or once calls have
waited 30 s with no answer to any of them, the answers to pings aside
(`STALL_SILENCE`; both beside `RESTARTS` in `src/macos/runner.rs`), and
closes. The supervisor then starts the runner again once, the start ending
the stuck process through DVT first, the restart counted and `last_death`
naming the stall (`it stalled: ...`). The call that found the runner
stalled fails `runner_stalled`, which can be repeated and waits for the
restart; one refusal alone stays the caller's to repeat; a second stall in
the session fails it as `runner_died`, which the daemon reports as
`disconnected` and detaches on. A healthy runner pays nothing for this: no
call is added, and the silence bound is one deadline on calls already
waiting, past the 10 s and 500 ms of grace the daemon gives a snapshot or a
text set, so only a call given a longer deadline can meet it; one the
runner works on longer than that is ended as stalled, which typing over
about 1,800 characters at 60 a second would be. `release` ends a stuck
runner's process through DVT's process control, asking the runner nothing.

On the phone, with spike S10's runner (`mobile-agent-ios-runner 0.1.0`),
in the device test's functional run (a debug build, load 4.1, so not a
measurement): the attach took 3.2 s, the hello 2.2 s after the start;
fifty mixed commands (17 snapshots at 150 and 232 ms p50 and p95, 17
screenshots at 71 and 98 ms, 16 taps at 276 and 282 ms) were served by one
runner process with no restart; release took 476 ms and left no runner
process. Later the same day iOS asked for the approval of "Enable UI
Automation" again without a restart: the start waited 60 s in "enabling
automation mode" with the passcode prompt on screen, and the driver
reported `automation_not_enabled`.

The runner `agent-ios` builds replaced S10's through this path (task 4.2):
its hello named `0.2.0`, so the attach installed the bundle
`MOBILE_AGENT_IOS_RUNNER_APP` names, in 4.9 s with S10's start and end, the
new hello 842 ms after its start. With it, fifty mixed commands took 148
and 208 ms per snapshot, 70 and 76 per screenshot and 278 and 280 per tap
through the runner, at p50 and p95 (debug build, load 4.1 to 3.3,
functional).

## Input

`Input` sends taps, presses, swipes, hardware buttons and keys from the
host through the disk image's `dtuhidd` over RSD (design D22), the
driver's default input, with `idevice`'s HID clients on two connections
kept for the target's lifetime:

- touches as 58-byte `mainTouchscreen` reports to
  `com.apple.coredevice.hid.universalhidservice`, at the touchscreen's
  coordinates, 0 to 65535 across the screen in points, which come from
  lockdown's `com.apple.mobile.iTunes` domain (`ScreenWidth`,
  `ScreenHeight`, `ScreenScaleFactor`: 750, 1334 and 2.0 on the SE). A tap
  is a contact held 50 ms, then the release; a press holds its hold. A
  swipe reports every 16 ms along its line; a resting swipe stays at its
  end, reporting, for 60 ms, the Android `rest_ms`, then creeps on along
  its line three moves of a point 16 ms apart and lifts at the last, as the
  Simulator driver's does, so the content stops where the finger left it.
  UIKit takes a drag's velocity from its last moves: a rest followed by one
  pixel on, the Android shape, still flung the fixture's list 917 px for a
  400 px stroke on the SE. A point off the screen is refused
  (`invalid_argument`) before anything is sent;
- hardware buttons as Indigo button events on the Consumer page to
  `com.apple.coredevice.hid.indigo`, held 80 ms: home 0x40, lock 0x30,
  volume up 0xE9, volume down 0xEA;
- keys as Indigo keyboard events, held 30 ms, their modifiers' events
  around them; `keys` types on a virtual keyboard registered on the
  universal service for them and removed afterwards, whatever happened.

Each input is acknowledged when its last report is written to the tunnel,
with no idle wait, and `Dispatch` says when the first and the last went
out. A write that fails before any report reached the device goes again on
new connections; one that fails after does not, since the device saw part
of it. Input reads the lock first, one value on lockdown's kept session,
and on a locked phone fails as `disconnected` (locked), since its reports
would reach the lock screen.

The path is gated on the device's iOS version: `dtuhidd` takes input from
the host from iOS 18 (Xcode 27's notes), and iOS 27 drops its reports
unless a display stream runs (pymobiledevice3, `idevice`), which this
driver does not open; iOS 26.6.2 took them with none (S11). Outside that
range the input goes through the attached runner's synthesis (`tap`,
`swipe` and `key` calls at device pixels, the buttons as `key` with the
Consumer page in the high half, `agent-ios`'s README), and without a
runner fails as `input_unavailable`, naming the version. That is
`AttachOptions::input`'s default, `InputChoice::Auto`; a session attached
with `InputChoice::Only(InputPath::Runner)` sends every touch, button and
key through the runner's synthesis whatever the version, as a fallback where
`dtuhidd` misbehaves, and one with `Only(InputPath::Hid)` through `dtuhidd`
alone, refused as `input_unavailable` where it takes no report. The
runner's synthesis misses D22's 150 ms: on the phone (task 4.5, functional,
load about 10) a tap was acknowledged in 313 ms, a 600 ms press in 831 ms
and a resting swipe of 400 px in 661 ms, which moved the list 347 px and
stopped; the measured series is in Numbers. The runner presses home and the
volume buttons but not the lock button, which `XCUIDevice` lacks.

On the phone, in the device test's functional run (load about 6): a tap on
the SwiftUI fixture's `row-1` was acknowledged in 84 ms, the first input
also opening the HID connection, and `detail-1` was in the runner's tree
259 ms later; the home button was acknowledged in 86 ms and the fixture
left the foreground. The measured series is in Numbers: taps acknowledged
in 57.1 ms at p95, every one reaching the screen, a resting swipe moving
the list 364 px for a 400 px stroke, and volume up and down showing the
volume HUD.

## Frames and screenshots

`Frames::frames` subscribes to the screen (design D23): DVT's
`takeScreenshot` and CoreDevice's `capturescreenshot` (PNG) polled side by
side, each on a connection of its own, at most `max_fps` a second over
both, the connections staggered. The pipeline, on a thread of its own:

- skips a PNG known to show the last frame before decoding it, by a hash of
  its bytes: the one delivered and those decoded since and found unchanged,
  whichever connection they came on;
- decodes the rest (8 or 16 bits a channel, RGB or RGBA) to BGRA and scales
  it with the Simulator pipeline's `scale_bgra` to `max_dimension` (1000:
  562x1000 on the SE);
- counts it as a frame when a pixel below the status bar moved more than
  two levels in a channel from the last frame, S11's tolerance, which
  counted none of 1,283 still-screen frames and caught every tap and drag
  on its first changed frame. A slow drift is measured from the last frame
  delivered, not the capture before;
- leaves out the device's status bar: 20 points on a home-button screen,
  under twice as tall as wide, as on the SE; 7.5% of the height on others,
  which covers their bars of 44 to 62 points. `FrameOptions::status_bar`
  takes another height;
- delivers the first frame and each change, with its sequence, when its
  capture was requested and arrived, its size, a hash of its pixels below
  the status bar, and its connection; keeps the last four for `encode`,
  which scales one down and encodes it as JPEG; drops the oldest when the
  consumer falls 16 behind.

A capture that fails is recorded in the stats, and its poller polls on
(design D26 of physical-ios): two captures in a row that fail (`MISSES`, in
`src/macos/capture.rs`), each waiting the reply limit, 10 s, or one that
finds the connection closed, make the poller open its connection and
channel again, once, through the target's tunnel, the target's lock held
only to connect, and counted in `FrameStats::reopens`, but never through a
tunnel the driver closed itself, at a reset or containing a phone that
dropped, which the next command opens; a poller that fails
the same way on the connection it opened again, before any capture served,
is left out of the frames, `FrameStats::left_out` saying why, and the other
poller goes on alone, as CoreDevice's did at 8 to 11 captures a second
while DVT's `takeScreenshot` answered none of 106 on a degraded phone in
task 9.6 (`reports/11.1-ios-device/probes/dvt-screenshot-fixed.txt`). DVT's
channel waits for a reply that came late rather than ending, so one slow
capture costs no connection; a closed connection is opened again at once
and captures at its poller's next turn, losing one capture interval. The
stream ends when both pollers are left out. `FrameStream::close_connection`
closes a poller's connection as a dropped one closes, the device test's
hook. A healthy poller pays nothing for this. `Frames::screenshot` takes
the screen through DVT's kept screenshot channel, or, once that failed
where CoreDevice's screen capture served, through CoreDevice's, kept for
the next screenshots and listed among the connections open, the other tried
when one fails: the device's PNG at the native size, or decoded, scaled to
`max_dimension` and encoded as JPEG (the Simulator pipeline's encoder) or
PNG; a failure of both is the first one's. `Frames::frame_paths` says which
path serves: screenshots; the display stream off, the display service's
`getmediasupportinfo` listing no media features on iOS 26.6.2, which
refuses the stream until iOS 27, and a later addition where it lists some;
AVFoundation's capture off, a later addition behind CoreMediaIO's
screen-capture allowance and the macOS camera permission.

On the phone, in the device test's functional run (load 3.1): a native
PNG of 127 KB in 199 ms and a JPEG at 1000 pixels in 224 ms, each opening
its channel first; about 20 captures a second over the two connections;
after a tap acknowledged in 61 ms, the first changed frame arrived 175 ms
later. The measured minute is in Numbers.

## Failure containment

A command that meets a disconnect (spec "Failure containment") tells the
cause from usbmuxd's listing and one probe of lockdown and the image
mounter, about 50 ms on failure paths only:

| Cause (`Disconnect`) | Told by | Then |
|---|---|---|
| `unplugged` | the phone left usbmuxd's listing: the cable was pulled, or it is restarting | its tunnel, channels, HID connections and runner session close |
| `rebooted` | it is back without the disk image, or refuses lockdown until its first unlock | the same; the next attach mounts the image again, once the phone is unlocked |
| `locked` | `PasswordProtected` reads true, as input reads it first | nothing closes; input goes on once it is unlocked |
| `lost` | the connection failed while the phone stays listed and ready | its connections close |

The error's code stays `disconnected`; its `kind` and message name the
cause and the step. No other target is touched: each has its own tunnel,
connections and session. While an unplugged phone stays out of usbmuxd's
listing, every command on it fails as `disconnected`, `unplugged`. A
replugged phone comes back under a new usbmuxd number, where the tunnel,
lockdown link and test manager held for the old one answer "bad device":
the driver records the number a target's state was made for and makes the
state anew when the listing shows another, so input works again at once.
After a disconnect the next attach starts over: a new tunnel, the runner's
leftover process ended, a new session with no restart counted. The process
list, whose read costs 474 to 728 ms on the phone, is read only by
`running`, `terminate`, a relaunch and a runner's start and stop, never by
input, frames, screenshots or the runner's calls.

A runner whose connection falls silent while the phone stays listed and
ready is lost one of two ways. With a call in flight, the call fails as
`disconnected`, `lost`, once three pings went unanswered (the protocol's
liveness, about 3 s), and is contained as the table says: the session
fails with that error, no restart counted, its connections close, and the
runner's process stays until a release or the next attach ends it
(`macos::containment::a_call_in_flight_when_the_runner_falls_silent_fails_the_session_until_the_next_attach`),
which is how the phone's status read after the measured run's stall
(`reports/11.1-ios-device/probes/status-after-stall.txt`: failed, 0
restarts, the runner's process still on the phone). With none, the
supervisor finds the connection gone, ends the silent process and starts
the runner again once, the restart counted
(`macos::runner::tests::a_runner_that_falls_silent_is_ended_and_started_again_once`).
The daemon detaches a target once a command fails `disconnected`, and
reports `runner_died` as `disconnected`, so its next command attaches
again, which starts over (task 9.3, design D26 of physical-ios).

A runner that stalls while its connection lives (Runner session) is no
disconnect: the call that finds it fails `runner_stalled` and is not
contained, its restart keeps the phone's tunnel and connections, input
through `dtuhidd` goes on meanwhile, and another phone is untouched
(`macos::containment::a_stalled_runner_is_started_again_with_the_phones_connections_kept_and_the_other_phone_untouched`,
task 9.7).

On the phone, with a person pulling the cable and plugging it back twice
(`containment::a_pulled_cable_is_reported_and_a_replug_attaches_again`, a
debug build; the 1-minute load read 4.18 during the second, so these are
not measurements): after each pull, the first of the test's calls, made
every 300 ms, failed with `disconnected`, `unplugged`, its cause the
tunnel's "CoreDeviceProxy: bad device". At the first replug, that run's
driver kept the old usbmuxd number and every attach failed for 60 s as
`lost` ("bad device"), which is how the renumbering above was found. After
the second, the phone was listed again and the new attach took 1862 ms,
the hello 806 ms after its start; it ended the runner process the pull had
left, the old session's, counted no restart, and a tap was then
acknowledged in 87.1 ms. That run's driver answered a second command made
while the phone was away with `not_found`; the `unplugged` answer above is
checked on the fakes only
(`macos::containment::every_command_on_a_pulled_phone_fails_as_unplugged_until_it_is_back`).

## Numbers

Measured series on 2026-10-09. Host: Apple M1 Max, macOS 26.6.2, Xcode 27.0
(27A266a); device: iPhone SE (3rd generation), iOS 26.6.2, on USB,
unlocked, Auto-Lock Never, battery on USB power. Each series waited for a
quiet host, a 1-minute load under 4 and no cargo, rustc, xcodebuild,
gradle, kotlinc, java or emulator process above 5% of a core, checked
every 30 s and logged, as the spikes' guard did (`tests/common`); device
tests are debug builds.

The install of the SwiftUI fixture's Debug device build (664 KB in 7
files), ten times in a row through `install` with the connection open
(`lifecycle::install_series`), at 17:15. The series waited 15 minutes; the
check that let it start read 3.99.

| Series | n | p50 | p95 | min | max | Load before / after (1, 5, 15 minutes) |
|---|---:|---:|---:|---:|---:|---|
| install, SwiftUI fixture | 10 | 1088 ms | 1379 ms | 1011 ms | 1379 ms | 4.15, 5.39, 11.69 / 4.05, 5.33, 11.59 |

Percentiles are nearest rank. Spike S10 installed its 3.7 MB runner the same
way in 1118 and 1290 ms at p50 and p95, and this fixture once in 1.6 to
1.7 s.

A minute of frames on the fixture's still List tab (`frames::frames_series`),
at 19:05, the guard's first check quiet:

| Series | Window | Captures a second (DVT, CoreDevice) | Identical, decoded, unchanged | Frames after the first | CPU, this process | CPU, usbmuxd | Load before / after |
|---|---:|---:|---|---:|---:|---:|---|
| frames, still screen, debug build | 60.0 s | 19.0 (570, 570) | 1138, 2, 2 | 0 | 15.3% | 13.0% | 2.58, 2.71, 3.69 / 2.51, 2.72, 3.63 |
| frames, still screen, release build | 60.0 s | 19.2 (575, 575) | 1148, 2, 2 | 0 | 4.0% | 7.5% | 2.92, 2.79, 2.79 / 4.43, 3.22, 2.95 |

The rate passes task 3.5's 15 a second and the CPU its quarter of a core:
in the release build, as the daemon runs, 4.0% in this process and 7.5% in
usbmuxd, which relays the PNGs, 11.5% together (28.3% in the debug build,
whose TCP stack is unoptimized). The release series started at 19:27 after
one busy check; the load rose to 4.43 from other work on the host by its end. S11
measured 16.8 a second from two pollers on a busy phone, and 6.5 to 15.5%
in its process with usbmuxd's 3.2 to 7.4% beside it.

The input series (`input::input_series`, debug build) at 19:34, the
guard's first check quiet: 20 taps after 2 warm-up taps, alternating the
fixture's Login and List tabs, each after the screen had been still for
400 ms, counted as reaching the screen when a frame captured after the
request showed a change; a resting swipe up the List tab, its travel the
best-matching shift of the rows between two screenshots; and the volume
buttons, each pressed once.

| Series | n | p50 | p95 | min | max | Load before / after |
|---|---:|---:|---:|---:|---:|---|
| tap, request to acknowledgement (the 50 ms hold included) | 20 | 54.5 ms | 57.1 ms | 52.6 ms | 57.8 ms | 2.13, 2.22, 2.50 / 1.97, 2.15, 2.44 |
| tap, request to the first changed frame | 20 | 190 ms | 240 ms | | | |

Every tap reached the screen, none missed. The resting swipe, 200 points
(400 px) up over 300 ms, was acknowledged in 397 ms and moved the list
364 px, 91% of the stroke, and stopped. Volume up and down were
acknowledged in 86 ms each, their 80 ms hold included, and each showed the
volume HUD in a frame captured after the press; each was pressed once, so
the volume came back to where it was. The lock button, pressed with a
someone at the phone to unlock it (`input::the_lock_button`), locked it:
lockdown's `PasswordProtected` read true 652 ms after the press, which was
acknowledged in 419 ms, the HID connections opened first in a new
process; the phone was unlocked by hand 33 s after the test asked.
S11 acknowledged its taps in 53.7 ms at p95 and S10 in 53.5 ms, the same
50 ms hold.

The tree series (`tree::tree_series`, debug build) at 22:39, after the
guard's fifth check: each Big tab read through `agent-ios`'s runner 20
times after 2 warm-up reads, from the request to the reply.

| Series | n | Nodes | p50 | p95 | min | max | Load before / after |
|---|---:|---:|---:|---:|---:|---:|---|
| tree, SwiftUI Big tab | 20 | 546 | 550.3 ms | 554.3 ms | 539.0 ms | 554.5 ms | 3.80, 7.18, 8.35 / 3.52, 7.01, 8.28 |
| tree, Expo Big tab | 20 | 529 | 445.6 ms | 451.9 ms | 443.2 ms | 453.8 ms | 3.64, 6.98, 8.26 / 4.17, 6.98, 8.24 |

Both pass D20's 1 s at p95; S10's `snapshotWithError:` took 476 and 527 ms
without platform classes or scroll areas.

The runner's input series (`input::runner_input_series`, debug build), the
session asking for the runner's synthesis, run three times at 23:04, 23:07
and 23:09: 20 taps after 2 warm-up taps alternating the fixture's Login and
List tabs, checked by changed frames as `input_series` checks `dtuhidd`'s,
then the harness's 36 characters typed 20 times and set 20 times into
`login-username`, each read back.

| Series | n | p50 | p95 | min | max | Load before / after |
|---|---:|---:|---:|---:|---:|---|
| tap through the runner, request to acknowledgement, run 1 | 20 | 292.2 ms | 300.0 ms | 287.6 ms | 302.0 ms | 3.12, 8.27, 9.35 / 2.91, 6.99, 8.74 |
| the same, run 2 | 20 | 295.2 ms | 299.2 ms | 283.8 ms | 303.2 ms | 3.48, 6.34, 8.37 / 2.13, 5.01, 7.61 |
| the same, run 3 | 20 | 292.2 ms | 298.4 ms | 286.5 ms | 301.1 ms | 1.66, 4.76, 7.48 / 2.73, 4.22, 6.96 |
| tap, request to the first changed frame, runs 1 to 3 | 20 each | 203, 198, 210 ms | 272, 246, 278 ms | | | |
| 36 characters typed, runs 1 to 3 | 20 each | 812, 810, 809 ms | 817, 813, 813 ms | | | |
| 36 characters set, runs 1 to 3 | 20 each | 216, 216, 215 ms | 237, 225, 228 ms | | | |

The runner's taps miss D22's 150 ms at p95 by about 150 ms, as S10's did at
294 ms, which is why `dtuhidd`'s 57 ms are the default. In the first run
3 of the 20 taps showed no changed frame within 2 s, after the guard had
waited 8 minutes through loads of 7 to 17; the next two runs, which name the
tab selected after such a tap, saw none in 40. Every text entry was read
back whole, 120 of 120.

## Compatibility

The driver supports iOS 17.4 up to but not including 27 and refuses other
versions with `unsupported_ios`, naming the version: lockdown's
`CoreDeviceProxy` carries the tunnel from iOS 17.4, and iOS 27 drops
`dtuhidd`'s reports unless a display stream runs, a path gated for later.
Host input is gated further to iOS 18 and later. Only iOS 26.6.2 on an
iPhone SE (3rd generation) under Xcode 27.0 (27A266a) and macOS 26.6.2 was
exercised, with `idevice` 0.1.68. `ios::render_compatibility()` prints the
range and the table below, which `ios::call_sites()` returns for `doctor`:

| Call | Service | Validated on | Note |
|---|---|---|---|
| `ListDevices`, `ReadPairRecord`, `Connect` | usbmuxd | iOS 26.6.2, Xcode 27.0: S9, `tests/discovery.rs` | the USB entry of a twice-listed phone; the pairing record from usbmuxd, not `/var/db/lockdown` |
| `GetValue` `DeviceName`, `ProductType`, `ProductVersion`, `BuildVersion` without a session; `StartSession` | lockdownd | iOS 26.6.2, Xcode 27.0: S9, `tests/discovery.rs` | |
| `GetValue` `DeveloperModeStatus` in `com.apple.security.mac.amfi` | lockdownd | iOS 26.6.2, Xcode 27.0: S9, `tests/discovery.rs` | |
| `GetValue` `PasswordProtected` | lockdownd | iOS 26.6.2, Xcode 27.0: S9, `tests/discovery.rs` | read before every launch and every input; CoreDevice's `getlockstate` is listed but answers that it is not implemented on iOS 26.6.2 (S9), so it is never asked |
| `GetValue` `BatteryCurrentCapacity`, `BatteryIsCharging`, `ExternalConnected` in `com.apple.mobile.battery` | lockdownd | iOS 26.6.2, Xcode 27.0: task 6.2's harness run, through the daemon's `status` | read in the probe's session, so a target's status names the battery as its attach found it; `devicectl device info details` reports no battery |
| `CopyDevices` | image mounter (lockdown) | iOS 26.6.2, Xcode 27.0: S9, `tests/discovery.rs` | the personalized image at `/System/Developer`; the Cryptex1 form is not listed here |
| `StartService` `com.apple.internal.devicecompute.CoreDeviceProxy`, CDTunnel's handshake | lockdownd, jktcp in this process | iOS 26.6.2, Xcode 27.0: S9, `tests/tunnel.rs` | no root, no TUN interface |
| RSD's handshake (RemoteXPC over HTTP/2, protocol version 7) | remoted, in the tunnel | iOS 26.6.2, Xcode 27.0: S9, `tests/tunnel.rs` | 82 services with the image mounted, 61 without (S10) |
| `_notifyOfPublishedCapabilities:` | DVT | iOS 26.6.2, Xcode 27.0: S9, `tests/tunnel.rs` | read before a connection is used |
| `QueryPersonalizationManifest`, `QueryNonce`, `QueryPersonalizationIdentifiers`, `UploadImage`, `MountImage` `Personalized`; TSS | image mounter, Apple's signing server | iOS 26.6.2, Xcode 27.0: not exercised; the image stayed mounted, and S10's call never returned | first, within 60 s; needs this Mac online |
| `xcrun devicectl device info ddiServices --auto-mount-ddis` | CoreDevice 642.16 on the host | iOS 26.6.2, Xcode 27.0: not exercised | the fallback, within 120 s |
| `_requestChannelWithCode:identifier:` | DVT | iOS 26.6.2, Xcode 27.0: S9, `tests/lifecycle.rs` | a channel per connection, kept |
| `launchSuspendedProcessWithDevicePath:bundleIdentifier:environment:arguments:options:` with `KillExisting` false | DVT's process control | iOS 26.6.2, Xcode 27.0: S9, `tests/lifecycle.rs`, `tests/input.rs` | written by hand: `launch_app` drops a refusal's reason; a running instance is signalled first, since a `KillExisting` relaunch kept a black window (S11, task 3.8); the app's output, `outputReceived:fromProcess:atTime:` on the channel (the Expo fixture's before the reply, task 4.4), is passed over |
| `sendSignal:toPid:` with SIGKILL and the pid archived | DVT's process control | iOS 26.6.2, Xcode 27.0: S9, `tests/lifecycle.rs` | written by hand: `kill_app`'s primitive pid stalled the phone's DVT service (S9) |
| `runningProcesses`, `isRunningPid:` | DVT's device information | iOS 26.6.2, Xcode 27.0: `tests/lifecycle.rs`, `tests/runner.rs` | `bundleIdentifier` for application processes, the runner's included; the list takes 480 to 710 ms |
| AFC upload to `PublicStaging`; installation proxy `Upgrade`, `Lookup` | AFC, installation proxy (lockdown) | iOS 26.6.2, Xcode 27.0: S10, `tests/lifecycle.rs` | over usbmuxd, not the tunnel |
| installation proxy `Lookup` of the runner (`Path`, `Container`, `CFBundleExecutable` ending in `-Runner`) | installation proxy (lockdown) | iOS 26.6.2, Xcode 27.0: S10, `tests/runner.rs` | a missing runner is installed from the bundle given |
| the xctest session: the DTX handshake on two testmanagerd connections, `_IDE_initiateControlSessionWithCapabilities:`, `_IDE_initiateSessionWithIdentifier:capabilities:`, the runner launched through process control (`XCTestBundlePath`, `XCTestSessionIdentifier`, `XCTestManagerVariant` DDI), `_IDE_authorizeTestSessionWithProcessID:`, `_XCT_testRunnerReadyWithCapabilities:` answered with the configuration, `_IDE_startExecutingTestPlanWithProtocolVersion:` | `com.apple.dt.testmanagerd.remote` and DVT, through `idevice`'s `XCUITestService` | iOS 26.6.2, Xcode 27.0 (XCTest 25228): S10, `tests/runner.rs` | kept for the session; the 60 s wait for "Enable UI Automation" is `automation_not_enabled`; an unanswered handshake is made again; error 103 is reported by `idevice` as an untrusted certificate |
| `Connect` to the runner's loopback port 27183; the device protocol's hello | usbmuxd, the runner's `AgentSession` | iOS 26.6.2, Xcode 27.0: S10, `tests/runner.rs` | the hello names `mobile-agent-ios-runner` and its version |
| `GetValue` `ScreenWidth`, `ScreenHeight`, `ScreenScaleFactor` in `com.apple.mobile.iTunes` | lockdownd | iOS 26.6.2, Xcode 27.0: `tests/input.rs`, `tests/frames.rs` | 750, 1334 and 2.0 on the SE |
| `send` with a 58-byte `mainTouchscreen` report (0x09, the contact count, 5, state 0xC2 touching or 0x02 lifted, x and y over 0 to 65535) to surface 257 | `com.apple.coredevice.hid.universalhidservice` (`dtuhidd`) | iOS 26.6.2, Xcode 27.0: S10, S11, `tests/input.rs` | no display stream on iOS 26.6.2; gated to iOS 18 up to 27 |
| `IndigoButtonEvent` (Consumer page; home 0x40, lock 0x30, volume up 0xE9, volume down 0xEA; state 1 down, 2 up) | `com.apple.coredevice.hid.indigo` (`dtuhidd`) | iOS 26.6.2, Xcode 27.0: S10, S11, `tests/input.rs` | held 80 ms; home reaches the home screen, where the phone's own home button does not work (S11); volume up and down show the volume HUD; lock locks the phone, `PasswordProtected` reading true 652 ms after the press |
| `IndigoKeyboardButtonEvent` (keyboard-page usage, state) | `com.apple.coredevice.hid.indigo` (`dtuhidd`) | iOS 26.6.2, Xcode 27.0: S11 | typed one key into Safari's address field |
| `createService` (a virtual `mainKeyboard`), `connectedServices`, `send` with a 39-byte keyboard report, `resetGestureState`, `removeService` | `com.apple.coredevice.hid.universalhidservice` (`dtuhidd`) | iOS 26.6.2, Xcode 27.0: S11 | removed after typing: a device may stay in hardware keyboard mode for two minutes (Xcode 27's notes) |
| `takeScreenshot` | DVT's screenshot service | iOS 26.6.2, Xcode 27.0: S9, S11, `tests/frames.rs` | 8-bit PNGs, 16-bit with an ICC profile for the home screen's wallpaper (S11); a frame poller's connection is opened again once two captures in a row time out (task 9.8) |
| `capturescreenshot`, `requestedFormat` `png` | `com.apple.coredevice.screencaptureservice` | iOS 26.6.2, Xcode 27.0: S9, S11, `tests/frames.rs` | the second frame connection, and the screenshot's where DVT's fails (task 9.8); JPEG is refused (S9) |
| `getmediasupportinfo` | `com.apple.coredevice.displayservice` | iOS 26.6.2, Xcode 27.0: S11, `tests/frames.rs` | `supportedFeatures` 0; the stream is refused with CoreDeviceError 9021 until iOS 27 (S11) |
| `-[XCAXClient_iOS activeApplications]`, `-systemApplication` | XCTest's accessibility client in the runner | iOS 26.6.2, Xcode 27.0 (XCTest 25228): S10, task 4.4, `tests/tree.rs` | the foreground, SpringBoard when no application is active; read again after the tree |
| `-[XCTRunnerDaemonSession requestApplicationSpecifierForPID:reply:]` | testmanagerd, through XCTestCore in the runner | iOS 26.6.2, Xcode 27.0: S10, `tests/tree.rs` | the foreground's bundle, once per pid; the runner's sandbox hides other processes' executables |
| `-[XCAXClient_iOS requestSnapshotForElement:attributes:parameters:error:]` on the application element, `snapshotKeyHonorModalViews` false | testmanagerd (`_XCT_requestSnapshotForElement:attributes:parameters:reply:`), through XCTest's accessibility client in the runner | iOS 26.6.2, Xcode 27.0: task 4.4, `tests/tree.rs` | the reader's attribute names with the visible frame and scroll areas and without `XC_kAXXCAttributeChildren`, whose array keeps the reply from decoding (65 s timeout, S10); `XC_kAXXCAttributeElementType` never comes back, so the class is `XC_kAXXCAttributeElementBaseType`'s; 546 nodes in 550 ms at p50 |
| `XCAXAccessibilityAttributesForStringAttributes` | XCTAutomationSupport in the runner | iOS 26.6.2, Xcode 27.0: task 4.4 | the numbers the snapshot's `additionalAttributes` are keyed by (ElementBaseType 5004, VisibleVerticalScrollArea 5027) |

## Tests

`cargo test -p ios` runs the unit tests, which need no device. Recorded
values stand in for one: usbmuxd's `ListDevices` reply recorded from the
phone (its serial numbers replaced) and fed through `idevice`'s own parser
over a fake usbmuxd socket, lockdown's values and the image mounter's
entries spike S9 and S10 recorded, RSD's handshake with S9's 82 services,
and a cut of the disk image's build manifest (`tests/fixtures/`). The fake
device (`src/macos/fake.rs`) answers `idevice`'s own RSD client with an
HTTP/2 frame carrying the XPC handshake, and its own DVT client in DTX: it
opens channels, launches, signals and lists processes, refuses a launch
with an archived `NSError`, takes screenshots of a screen the test sets,
stops answering a service's captures on the connections open or on every
connection, or drops a capture connection,
records every call with its arguments unarchived, and drops the connection
on a primitive pid as the phone did. Its tunnel drops, closing the
connections over it; the device goes away and comes back, locks, and
reboots, losing its disk image and its runner's process. A fake host holds
two phones.

`src/macos/fake_xpc.rs` serves `idevice`'s own RemoteXPC clients at the
wire, HTTP/2 frames carrying XPC messages: `dtuhidd`'s universal service
decodes each `mainTouchscreen` report and records it with when it
arrived, registers, lists and removes the virtual keyboard (whose identity
`idevice` checks), and the Indigo service decodes button and keyboard
events; CoreDevice's screen capture and display services answer in
CoreDevice's output envelope. `src/macos/fake_runner.rs` stands for the
test manager and the runner: its session refuses a runner not installed,
reports XCTest's events, starts the runner's process on the fake device and
lasts as long as it and the cable; scripted starts fail the handshake, miss
the test manager, wait for an approval or start slowly. The runner serves
the protocol's `AgentSession` on its port, saying hello with the installed
version (an installed bundle's `CFBundleShortVersionString`); one left from
a lost session says hello and closes the connection, as S10's did. A
runner can fall silent, pings included; get stuck, keeping the next
request, which its session answers `deadline_exceeded` at its deadline, and
answering every later one `busy`, as the phone's did in task 9.6; or answer
pings and no request.

`src/macos/fake_fixture.rs` is the SwiftUI fixture on the fake device, its
List and Login tabs on the SE's screen in the runner's pixels: DVT's launch
starts it on the tab `FIXTURE_INITIAL_TAB` names, a touch `dtuhidd` takes
taps its tab bar or its username field, and the fake runner reads it and
enters text into its focused field. Its first two reads after a launch list
the tab bar's buttons without their identifiers: SwiftUI sets them after the
first rows show (`tests/tree.rs` waits for a tab's own identifier), and on
the phone the runner's read after the one that listed `row-1` lacked
`tab-login` in task 9.7's device check. `src/macos/setups.rs` runs the
device tests' SwiftUI helpers (`tests/common/swiftui.rs`) on it through the
driver's own launch, runner session, taps and text entry, so a device test's
setup is checked there before it runs on the phone (task 9.9).

The tests in `tests/` drive the phone. Each checks `MOBILE_AGENT_IOS_UDID`
first; when it is unset, as in CI, the test prints why it skipped and
passes. To run them, attach an iPhone over USB, trusted, unlocked, with
Developer Mode on and Auto-Lock at Never, the SwiftUI fixture installed,
and name the team the runner is signed for, the runner bundle
`agent-ios/build.sh` built and the fixture's device build:

```sh
agent-ios/build.sh                            # prints the runner bundle's path, agent-ios/dist/
DESTINATION=device fixtures/swiftui/build.sh  # prints the fixture's device build, signed for the team
MOBILE_AGENT_IOS_UDID=<udid> MOBILE_AGENT_IOS_TEAM=<team> \
  MOBILE_AGENT_IOS_RUNNER_APP=agent-ios/dist/MobileAgentRunner-Runner.app \
  MOBILE_AGENT_IOS_SWIFTUI_APP=<path to FixtureSwiftUI.app> cargo test -p ios
# measurements, on a quiet host (each waits for the guard)
cargo test -p ios --test lifecycle -- --ignored --nocapture install_series
cargo test -p ios --test frames -- --ignored --nocapture frames_series
cargo test -p ios --test input -- --ignored --nocapture input_series
# with someone at the phone
cargo test -p ios --test input -- --ignored --nocapture the_lock_button
cargo test -p ios --test containment -- --ignored --nocapture
# task 9.7's device check, once the phone was restarted
cargo test -p ios --test input -- --ignored --nocapture one_hundred_twenty_emptying_fills_in_one_session
# task 9.8's device check
cargo test -p ios --test frames -- --ignored --nocapture a_dvt_connection_closed_mid_session_is_opened_again
```

The tests ask for the runner `agent-ios` builds, by its version
(`agent_ios::VERSION`), and `MOBILE_AGENT_IOS_RUNNER_APP` names its
bundle, which an attach installs when the phone's runner is missing or says
hello with another version; without the variable such an attach fails, and
the test of the replacement skips. Neither the identifier nor the team
enters a tracked file, and no test prints them. Without
`MOBILE_AGENT_IOS_TEAM` the phone is listed as `no_signing_identity`, which
the discovery test accepts; without the fixture's bundle the lifecycle
tests skip. The lifecycle test installs the fixture, which stays installed,
launches and terminates it; the runner and input tests launch it, press
home and leave it running; the runner's bundle is installed when the phone
has another version; nothing else is installed and no setting is changed.

## Spec requirements and their tests

| Requirement | Scenario | Tests |
|---|---|---|
| Device discovery and readiness | Ready device listed | `discovery::the_phone_is_listed_once_with_its_identifier_name_model_and_ios_version`, `macos::driver::tests::a_ready_phone_is_listed_once_with_its_identifier_name_model_and_version`, `macos::host::tests::the_recorded_listing_goes_through_usbmuxds_parser_to_the_phones_usb_entry`, `discovery::tests::a_phone_listed_on_usb_and_the_network_is_kept_once_by_its_usb_entry`, `discovery::tests::the_recorded_values_describe_the_phone_and_make_it_ready`, `discovery::tests::the_battery_domain_gives_the_charge_and_whether_it_charges_on_external_power`, `identity::tests::the_identity_of_the_named_team_is_found` |
| Device discovery and readiness | Device not ready | `macos::driver::tests::an_unready_phone_is_listed_in_its_state_with_the_step_to_take`, `macos::driver::tests::a_command_on_an_unready_phone_fails_with_a_code_naming_the_state`, `macos::driver::tests::a_command_needs_only_what_its_operation_needs`, `macos::driver::tests::a_command_on_a_phone_that_is_not_ready_fails_with_its_state`, `readiness::tests::every_unready_state_has_a_code_and_a_message_naming_its_step`, `readiness::tests::each_unready_state_is_the_first_check_the_device_has_not_passed`, `readiness::tests::an_operation_fails_only_on_the_states_it_needs`, `discovery::tests::each_recorded_value_that_changes_gives_its_state`, `macos::host::tests::a_failed_pairing_record_or_session_says_what_stands_in_the_way`, `identity::tests::a_missing_team_identity_or_team_says_which`, `identity::tests::the_check_reads_the_identities_and_certificates_security_prints`, `identity::tests::a_security_that_hangs_is_ended_at_its_timeout_and_the_readiness_names_it`, `macos::driver::tests::the_signing_identity_is_checked_once_and_again_after_a_failure_or_when_forgotten` (task 9.2), `discovery::a_command_reads_the_phone_again_and_an_unknown_identifier_is_not_found`, `macos::runner::tests::a_start_waiting_for_the_automation_approval_is_reported_as_that_state`, `macos::runner::tests::a_start_whose_approval_never_comes_fails_naming_the_step`, `macos::runner::tests::a_device_whose_rsd_lists_no_test_manager_fails_as_the_disk_image`, `macos::runner::tests::a_locked_phone_does_not_attach` |
| Transport without root or helpers | Services reached from the daemon | `tunnel::the_tunnel_opens_in_this_unprivileged_process_and_lists_the_developer_services`, `lifecycle::the_fixture_is_installed_launched_with_an_environment_found_running_and_terminated`, `frames::screenshots_frame_paths_and_a_frame_for_a_tap_come_from_the_phone`, `macos::tunnel::tests::the_handshake_lists_the_recorded_services_and_a_service_is_found_by_name`, `macos::tunnel::tests::the_services_a_device_without_the_disk_image_lacks_are_named_missing`, `macos::tunnel::tests::a_service_the_handshake_does_not_list_is_missing_and_the_tunnel_stays`, `macos::tunnel::tests::a_handshake_that_never_comes_times_out_and_leaves_nothing_open`, `macos::driver::tests::connecting_a_ready_phone_opens_the_tunnel_and_lists_its_services`, `macos::driver::tests::reset_closes_the_tunnel_and_the_next_connect_opens_a_new_one`, `macos::capture::tests::the_screenshot_is_the_devices_png_at_full_size_or_scaled_to_a_long_edge` |
| Transport without root or helpers | The disk image mounted when the device lacks one | `macos::driver::tests::a_phone_without_the_disk_image_has_it_mounted_before_the_tunnel_opens`, `macos::driver::tests::a_mount_that_fails_offline_fails_naming_the_signing_server_and_opens_nothing`, `macos::image::tests::a_missing_image_is_mounted_through_idevice_and_checked`, `macos::image::tests::idevice_hanging_or_failing_falls_back_to_devicectl`, `macos::image::tests::a_success_the_image_mounter_does_not_confirm_falls_back_too`, `macos::image::tests::both_failing_offline_say_the_signing_server_is_needed`, `macos::image::tests::the_manifest_names_the_image_and_trust_cache_of_the_devices_identity`, `macos::containment::a_rebooted_phone_is_told_apart_and_attaches_again_once_unlocked` (not on the device: the image stayed mounted) |
| One resident runner per session | Runner stays up | `runner::the_runner_serves_fifty_commands_with_one_process_and_no_restart`, `macos::runner::tests::fifty_commands_are_served_by_one_runner_with_no_restart`, `macos::runner::tests::the_runner_starts_through_the_test_manager_and_says_hello_with_its_version`, `macos::runner::tests::a_runner_that_dies_is_started_again_once_and_the_restart_is_reported`, `macos::runner::tests::release_stops_the_runner_and_its_session`, `macos::runner::tests::a_runner_left_from_a_lost_session_is_ended_before_the_start`, `macos::runner::tests::a_start_the_test_manager_does_not_answer_is_made_again`, `macos::runner::tests::a_session_the_test_manager_ends_for_want_of_its_service_is_the_disk_image`, `macos::runner::tests::a_call_before_attaching_fails_as_not_attached`, `runner::tests::the_hello_names_the_runner_and_its_version`, `runner::tests::the_test_managers_failures_say_what_stands_in_the_way` |
| One resident runner per session | Runner replaced on version change | `macos::runner::tests::a_runner_of_another_version_is_replaced_by_the_bundle_given`, `macos::runner::tests::a_runner_of_another_version_without_a_bundle_fails_naming_both_versions`, `macos::runner::tests::a_missing_runner_is_installed_from_the_bundle_or_reported`, `runner::the_built_runner_replaces_another_version_and_says_hello_with_its_own` (on the device it replaced S10's `0.1.0` once; the phone has `agent-ios`'s version since) |
| Foreground tree read | Reference screen read | `tree::the_fixtures_screens_list_through_the_mirror_as_on_the_simulator` (the SwiftUI and Expo Big tabs, the Login tab with the keyboard, the System tab's alert with its title and message, against the Simulator's listings in `tests/fixtures/listings/`, which `tree::record_the_simulators_listings` records, on an iPhone 17 Simulator with iOS 26.5), `tree::tree_series`, and `agent-ios`'s `xctree::tests` and `dispatch::tests` (its README) |
| App lifecycle | Fixture launched (the launch and the process; the tree read is the runner's, group 4) | `lifecycle::the_fixture_is_installed_launched_with_an_environment_found_running_and_terminated`, `macos::driver::tests::install_checks_the_bundle_uploads_it_and_reports_its_identifier_and_time`, `macos::driver::tests::a_launch_reads_the_lock_then_launches_through_process_control_and_returns_the_pid`, `macos::driver::tests::a_relaunch_ends_the_running_instance_itself_rather_than_with_kill_existing`, `macos::driver::tests::a_launch_on_a_locked_phone_is_refused_without_being_tried`, `macos::driver::tests::launching_an_app_that_is_not_installed_fails_as_not_installed`, `macos::driver::tests::a_refused_launch_of_an_installed_app_says_why`, `macos::driver::tests::terminate_signals_the_apps_process_and_returns_once_it_is_gone`, `macos::driver::tests::terminating_an_app_that_is_not_running_signals_nothing`, `macos::dvt::tests::a_launch_sends_the_bundle_its_environment_arguments_and_kill_existing_and_reads_the_pid`, `macos::dvt::tests::a_kill_is_send_signal_to_pid_with_sigkill_and_the_pid_both_archived`, `macos::dvt::tests::the_process_list_gives_each_application_its_bundle_identifier`, `macos::dvt::tests::whether_a_pid_runs_is_asked_with_the_pid_archived`, `macos::dvt::tests::a_refused_launch_reads_the_errors_domain_code_and_description`, `bundle::tests::a_simulator_build_is_refused_naming_the_device_destination`, `macos::driver::tests::a_bundle_a_device_does_not_take_is_refused_before_anything_is_asked_of_it` |
| Input actions without idle waits | Tap dispatched | `input::a_tap_reaches_the_screen_and_the_home_button_the_home_screen`, `input::input_series`, `frames::screenshots_frame_paths_and_a_frame_for_a_tap_come_from_the_phone`, `macos::hid::tests::a_tap_is_a_main_touchscreen_contact_held_then_released_and_acknowledged_at_the_release`, `macos::hid::tests::a_long_press_holds_its_contact_for_its_hold`, `macos::hid::tests::a_resting_swipe_rests_before_it_moves_one_pixel_on_and_lifts`, `macos::hid::tests::a_moving_swipe_lifts_at_the_end_of_its_path`, `macos::hid::tests::the_home_button_is_an_indigo_button_event_down_held_then_up`, `macos::hid::tests::lock_and_volume_take_their_consumer_usages`, `macos::hid::tests::a_key_is_an_indigo_keyboard_event_with_its_modifiers_held_around_it`, `macos::hid::tests::keys_are_typed_on_a_virtual_keyboard_registered_then_removed`, `macos::hid::tests::a_point_off_the_screen_is_refused_before_any_report`, `macos::hid::tests::the_connections_to_dtuhidd_are_kept_and_opened_again_after_the_tunnel_dropped`, `macos::hid::tests::an_ios_whose_dtuhidd_takes_no_report_goes_through_the_runner_or_is_refused`, `touch::tests::points_map_to_the_touchscreen_space_and_off_screen_points_are_refused`, `touch::tests::a_tap_is_a_contact_held_50_ms_then_a_release_at_the_same_point`, `touch::tests::a_moving_swipe_reports_every_16_ms_along_its_line_and_lifts_at_its_end`, `touch::tests::a_resting_swipe_stays_still_60_ms_then_moves_one_pixel_on_and_lifts_there`, `touch::tests::a_swipe_that_leaves_the_screen_is_refused`, `compat::tests::host_input_is_gated_on_an_ios_that_takes_reports_without_a_stream` |
| Frames and screenshots from the host | Frame change after a tap | `frames::screenshots_frame_paths_and_a_frame_for_a_tap_come_from_the_phone`, `frames::frames_series`, `macos::capture::tests::frames_are_polled_on_dvt_and_coredevice_side_by_side`, `macos::capture::tests::a_still_screen_counts_no_frame_after_the_first`, `macos::capture::tests::a_change_on_screen_is_the_next_frame`, `macos::capture::tests::frames_are_throttled_to_the_rate_asked_for`, `macos::capture::tests::a_held_frame_encodes_as_a_jpeg_of_the_size_asked_for`, `macos::capture::tests::the_screenshot_is_the_devices_png_at_full_size_or_scaled_to_a_long_edge`, `macos::capture::tests::the_display_stream_and_avfoundation_say_why_they_are_off`, `macos::capture::tests::the_pace_spreads_the_rate_over_the_connections`, `frames::tests::identical_pngs_are_skipped_before_decoding`, `frames::tests::a_frame_is_a_pixel_below_the_status_bar_more_than_two_levels_from_the_last_frame`, `frames::tests::a_change_inside_the_devices_status_bar_is_no_frame`, `frames::tests::the_status_bar_comes_from_the_devices_screen`, `frames::tests::no_still_frame_is_counted_through_one_level_noise_and_drift_is_measured_from_the_last_frame`, `frames::tests::frames_are_scaled_to_the_long_edge_asked_for_and_held_for_encoding`, `frames::tests::eight_and_sixteen_bit_pngs_become_bgra`, `frames::tests::a_png_that_does_not_decode_is_a_failure_not_a_frame` (settle on these frames is the mirror's, task 5.2); a poller whose connection stops answering, task 9.8: `macos::capture::tests::a_poller_whose_captures_time_out_twice_in_a_row_reopens_its_connection_and_polls_on`, `macos::capture::tests::a_poller_still_failing_after_its_reopen_is_left_out_and_the_other_polls_on`, `macos::capture::tests::a_poller_whose_connection_closes_opens_it_again_within_a_capture_interval`, `macos::capture::tests::the_test_hook_closes_a_pollers_connection_and_the_poller_opens_it_again`, `macos::capture::tests::the_pollers_open_no_tunnel_the_driver_closed`, `macos::capture::tests::the_screenshot_takes_the_other_connection_when_its_own_fails_and_keeps_to_it`, on the phone `frames::a_dvt_connection_closed_mid_session_is_opened_again` |
| Failure containment | Runner connection lost (the driver's side; the daemon's release and new attach are in `crates/daemon/README.md`) | `macos::containment::a_call_in_flight_when_the_runner_falls_silent_fails_the_session_until_the_next_attach`, `macos::runner::tests::a_runner_that_falls_silent_is_ended_and_started_again_once` |
| Failure containment; One resident runner per session | (a runner that stalls, its connection alive, treated as died: task 9.7, design D26 of physical-ios) | `macos::connection::tests::two_calls_in_a_row_answered_deadline_exceeded_or_busy_stall_the_runner`, `macos::connection::tests::an_answer_between_two_refusals_keeps_the_runner_serving`, `macos::connection::tests::a_call_left_unanswered_past_the_silence_bound_stalls_the_runner_though_it_answers_pings`, `macos::runner::tests::a_runner_stuck_on_a_call_is_ended_and_started_again_once_at_its_second_refusal`, `macos::runner::tests::a_runner_that_answers_no_call_for_the_silence_bound_is_ended_and_started_again_once`, `macos::runner::tests::a_second_stall_in_the_session_fails_it_as_a_runner_that_died`, `macos::runner::tests::a_release_ends_a_stuck_runners_process_through_dvt_within_five_seconds`, `macos::containment::a_stalled_runner_is_started_again_with_the_phones_connections_kept_and_the_other_phone_untouched`; on the phone, `input::one_hundred_twenty_emptying_fills_in_one_session`, its setup checked on the fake device by `macos::setups::the_emptying_fills_setup_waits_for_the_tab_bar_swiftui_identifies_after_its_rows` (task 9.9) |
| Failure containment | Cable pulled | `containment::a_pulled_cable_is_reported_and_a_replug_attaches_again`, `macos::containment::a_pulled_cable_fails_the_phones_commands_as_unplugged_and_spares_the_other_phone`, `macos::containment::a_replugged_phone_attaches_again_and_starts_over`, `macos::containment::a_replugged_phone_takes_commands_again_under_its_new_usbmuxd_number`, `macos::containment::every_command_on_a_pulled_phone_fails_as_unplugged_until_it_is_back`, `macos::containment::a_locked_phone_fails_input_as_locked_and_keeps_its_connections`, `macos::containment::a_rebooted_phone_is_told_apart_and_attaches_again_once_unlocked`, `macos::containment::a_command_made_while_the_runner_starts_again_waits_for_it_within_its_timeout`, `macos::containment::no_hot_path_reads_the_process_list`, `macos::containment::a_device_that_refuses_lockdown_until_its_first_unlock_reads_as_rebooted`, `macos::tunnel::tests::a_tunnel_that_dropped_is_opened_again_once`, `macos::tunnel::tests::a_device_that_is_gone_is_disconnected_with_the_cause`, `macos::driver::tests::a_call_after_the_tunnel_dropped_goes_through_a_new_tunnel`, `macos::dvt::tests::a_call_on_a_connection_that_closed_fails_and_the_channel_is_done`, `macos::dvt::tests::a_call_without_a_reply_times_out_and_ends_the_channel`, `macos::driver::tests::a_launch_whose_dvt_never_answers_times_out` |
| Compatibility statement | Table lists the device paths | `compat::tests::every_call_site_names_the_ios_and_xcode_it_was_validated_on`, `compat::tests::the_table_renders_the_range_and_every_call_site`, `compat::tests::ios_17_4_up_to_27_is_supported`, `compat::tests::host_input_is_gated_on_an_ios_that_takes_reports_without_a_stream`, `readiness::tests::an_ios_outside_the_supported_range_is_refused_naming_the_version` |
