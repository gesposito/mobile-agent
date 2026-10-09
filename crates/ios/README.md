# ios

Drives a physical iPhone over USB from the host (design D18 to D25, spec
`ios-device-driver`): discovery and readiness over usbmuxd and lockdown,
the CoreDevice tunnel and the developer services over it, the developer
disk image mounted when it is missing, and app lifecycle through DVT and
the installation proxy. Everything runs in the caller's process through the
`idevice` crate: no root, no TUN interface, no helper process in another
language, and no `devicectl` except as the disk image mount's fallback.
The runner session, tree reads, input and frames come with tasks 3.4 to
3.8.

`IosDriver` implements the driver traits, `Devices` and `Lifecycle`. They
carry no `idevice` types, so the daemon can hold the driver behind them
and adapt it to its own driver contract, as it does the Simulator driver
(Driver contract). On hosts other than macOS the crate still builds:
`IosDriver` exists there, and `open` fails with `platform_unsupported`.

## Layout

```
src/
  driver.rs        the driver traits: Devices, Lifecycle
  model.rs         the values they take and return
  error.rs         errors, each with a stable code
  readiness.rs     the readiness states, their codes and steps, and what each operation needs
  discovery.rs     usbmuxd's listing to USB entries, lockdown's values to a device and its probe
  identity.rs      the host's signing identity for the runner, through security
  bundle.rs        the app bundle an install takes, checked on the host
  services.rs      the RSD services the driver needs
  compat.rs        the supported iOS range and the call-site table
  version.rs       dotted version comparison
  unsupported.rs   the driver on hosts other than macOS
  macos/
    host.rs        the Host and Link seams: usbmuxd, the lockdown probe, the lock, the installation proxy
    tunnel.rs      the Opener seam and the Tunnel: CoreDeviceProxy, jktcp, RSD, reconnection
    image.rs       the Mounter seam: the disk image's check and mount, idevice then devicectl
    dvt.rs         DVT's channels on connections of their own, the calls written by hand
    driver.rs      IosDriver over Core, which keeps each target's tunnel, channels and lock session
    fake.rs        the tests' fake device: RSD, DVT over DTX, the link and the mounter
tests/
  common/          gating on MOBILE_AGENT_IOS_UDID
  discovery.rs     task 3.1 on the device
  tunnel.rs        task 3.2 on the device
  lifecycle.rs     task 3.3 on the device, and the install series (ignored)
  fixtures/        recorded values: usbmuxd's listing, lockdown's values, CopyDevices,
                   RSD's handshake, and the disk image's build manifest cut down
```

## Building

`idevice` is pinned to `=0.1.68`, as its README asks, since breaking changes
come with every point release before 0.2. Its features are `usbmuxd`,
`core_device_proxy`, `tunnel_tcp_stack`, `rsd`, `xpc`,
`mobile_image_mounter`, `tss` (the signing server's ticket for the mount),
`installation_proxy`, `afc` (the install's upload) and `dvt`, with its
default TLS, rustls on aws-lc-rs. Lockdown needs no feature, and pairing
(`pair`) is not used: the person at the phone trusts the Mac. `idevice` and
tokio are macOS-only dependencies; `plist` builds everywhere, for the
platform-neutral parts and their tests.

The driver owns a tokio runtime of two worker threads, as the Android
driver's emulator controller owns one; its traits are synchronous and each
call blocks on that runtime from a thread outside it.

## Driver contract

The daemon registers the driver in task 5.1. The traits map onto its
contract (`crates/daemon/src/driver.rs`) as the Simulator driver's do:

| Daemon | This crate |
|---|---|
| `Driver::discover` | `Devices::devices`: a `TargetInfo` per device, its state the readiness code and its advice the readiness message |
| `Driver::attach` | `Devices::connect`, then the runner's session (task 3.4) |
| `AppControl::launch` | `Lifecycle::launch`, `relaunch` as `terminate_running` |
| `AppControl::terminate` | `Lifecycle::terminate` |
| `Report::report` | `Devices::status` |
| `Driver::compatibility` | `ios::call_sites` and the supported range |

Every method names its device by identifier: names are not unique, so no
method takes one, and a name fails as `not_found`.

## Readiness

`devices` lists every iPhone attached over USB with its identifier, its
name, model (`ProductType`, such as `iPhone14,6`) and iOS version from
lockdown, and its readiness: the first state it has not passed, in the order
a person makes a device ready. A phone with Wi-Fi sync is listed twice by
usbmuxd, on USB and on the network; the USB entry is taken, and a device
listed only on the network is not listed.

| State | Code | When | The step its message names |
|---|---|---|---|
| unsupported iOS | `unsupported_ios` | lockdown's `ProductVersion` is outside iOS 17.4 up to but not including 27 | none; the version and this README's Compatibility section |
| not trusted | `not_trusted` | usbmuxd holds no pairing record (`ReadPairRecord`), the device refuses it (`InvalidHostID`), or lockdown starts no session | unlock it, connect it over USB, tap Trust and enter the passcode; Finder starts the pairing when nothing else asks |
| Developer Mode off | `developer_mode_off` | `DeveloperModeStatus` in `com.apple.security.mac.amfi` is not true | Settings > Privacy & Security > Developer Mode, restart, confirm with the passcode |
| disk image unmounted | `disk_image_unmounted` | the image mounter's `CopyDevices` lists no developer image | connecting mounts the selected Xcode's, which needs this Mac online for Apple's signing server; `xcrun devicectl device info ddiServices --device <identifier>` mounts it by hand |
| locked | `device_locked` | `PasswordProtected` is not false | unlock it, and set Auto-Lock to Never for long runs |
| no signing identity | `no_signing_identity` | no valid Apple Development identity of the team `MOBILE_AGENT_IOS_TEAM` names in the keychain | create the development certificate in Xcode with the team's account and export the team's identifier |
| ready | `ready` | none of the above | |

A value that could not be read does not pass its check. The device's state
is read on the device, the identity on the Mac: `security find-identity -v
-p codesigning` lists the valid identities, and `security find-certificate
-a -Z -p -c "Apple Development"` their certificates, whose subject's
organizational unit is the team (the code in the identity's name is the
developer's, not the team's). The check costs about 40 ms per command.

An operation needs a prefix of that order (`Need`), and fails with the
code of the first state it needs that the device has not passed;
`require(target, need)` reads the device afresh and does this for any
level. A command connects the device on its first use of it, which needs
what the developer services need: trust, Developer Mode and the disk image,
which it mounts when missing. A launch also needs the device unlocked,
read before every launch, and the runner's session (task 3.4) the signing
identity: a locked phone still connects, installs and lists its processes,
and a launch on it fails with `device_locked`.

## Transport

`connect` reads the device, fails on what stands before its developer
services, mounts the developer disk image when the image mounter lists
none, then opens the CoreDevice tunnel and DVT's process control unless
they are open, and keeps them for the target's lifetime: lockdown's
`CoreDeviceProxy` over usbmuxd carries the tunnel, jktcp ends it in this
process, and RSD's handshake over it lists the services by name and port.
`status` reports what is kept, the tunnel's opens, how long the last took,
the services listed and those needed but missing, the disk image and the
DVT channels open, from a snapshot it reads while a command runs.
`reset_target` closes them.

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
late reply cannot answer the next call.

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
  at others (S9). It then sends
  `launchSuspendedProcessWithDevicePath:bundleIdentifier:environment:arguments:options:`
  with the arguments and environment archived, the app started running and
  `KillExisting` as `terminate_running` asks, and returns the pid of the
  reply. A refusal is `app_not_installed` when the installation proxy does
  not list the app, `launch_failed` with DVT's `NSError` otherwise, read
  from the reply's archive since `idevice`'s decoder leaves it unresolved.
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
functional runs (the host's 1-minute load 8 to 13 from other sessions, so
these are not measurements): the first launch after an install took 2.2 s
three times out of three and a relaunch 26 to 41 ms with the lock read; one
read of the process list, about 580 processes through the tunnel, 474 to
728 ms; terminate 526 ms, nearly all of it that read. The install is timed
in Numbers.

## Numbers

The only series of this task, as the brief asks: ten installs of the
SwiftUI fixture's Debug device build in a row through `install`, the
connection open, after a wait for a quiet host (a 1-minute load under 4 and
no cargo, rustc, xcodebuild, gradle, kotlinc or java process above 5% of a
core). Results are added once the series has run.

## Compatibility

The driver supports iOS 17.4 up to but not including 27 and refuses other
versions with `unsupported_ios`, naming the version: lockdown's
`CoreDeviceProxy` carries the tunnel from iOS 17.4, and iOS 27 drops
`dtuhidd`'s reports unless a display stream runs, a path gated for later.
Only iOS 26.6.2 on an iPhone SE (3rd generation) under Xcode 27.0 (27A266a)
and macOS 26.6.2 was exercised, with `idevice` 0.1.68.
`ios::render_compatibility()` prints the range and the table below, which
`ios::call_sites()` returns for `doctor`:

| Call | Service | Validated on | Note |
|---|---|---|---|
| `ListDevices`, `ReadPairRecord`, `Connect` | usbmuxd | iOS 26.6.2, Xcode 27.0: S9, `tests/discovery.rs` | the USB entry of a twice-listed phone; the pairing record from usbmuxd, not `/var/db/lockdown` |
| `GetValue` `DeviceName`, `ProductType`, `ProductVersion`, `BuildVersion` without a session; `StartSession` | lockdownd | iOS 26.6.2, Xcode 27.0: S9, `tests/discovery.rs` | |
| `GetValue` `DeveloperModeStatus` in `com.apple.security.mac.amfi` | lockdownd | iOS 26.6.2, Xcode 27.0: S9, `tests/discovery.rs` | |
| `GetValue` `PasswordProtected` | lockdownd | iOS 26.6.2, Xcode 27.0: S9, `tests/discovery.rs` | read before every launch; CoreDevice's `getlockstate` is listed but answers that it is not implemented on iOS 26.6.2 (S9), so it is never asked |
| `CopyDevices` | image mounter (lockdown) | iOS 26.6.2, Xcode 27.0: S9, `tests/discovery.rs` | the personalized image at `/System/Developer`; the Cryptex1 form is not listed here |
| `StartService` `com.apple.internal.devicecompute.CoreDeviceProxy`, CDTunnel's handshake | lockdownd, jktcp in this process | iOS 26.6.2, Xcode 27.0: S9, `tests/tunnel.rs` | no root, no TUN interface |
| RSD's handshake (RemoteXPC over HTTP/2, protocol version 7) | remoted, in the tunnel | iOS 26.6.2, Xcode 27.0: S9, `tests/tunnel.rs` | 82 services with the image mounted, 61 without (S10) |
| `_notifyOfPublishedCapabilities:` | DVT | iOS 26.6.2, Xcode 27.0: S9, `tests/tunnel.rs` | read before a connection is used |
| `QueryPersonalizationManifest`, `QueryNonce`, `QueryPersonalizationIdentifiers`, `UploadImage`, `MountImage` `Personalized`; TSS | image mounter, Apple's signing server | iOS 26.6.2, Xcode 27.0: not exercised; the image stayed mounted, and S10's call never returned | first, within 60 s; needs this Mac online |
| `xcrun devicectl device info ddiServices --auto-mount-ddis` | CoreDevice 642.16 on the host | iOS 26.6.2, Xcode 27.0: not exercised | the fallback, within 120 s |
| `_requestChannelWithCode:identifier:` | DVT | iOS 26.6.2, Xcode 27.0: S9, `tests/lifecycle.rs` | a channel per connection, kept |
| `launchSuspendedProcessWithDevicePath:bundleIdentifier:environment:arguments:options:` with `KillExisting` | DVT's process control | iOS 26.6.2, Xcode 27.0: S9, `tests/lifecycle.rs` | written by hand: `launch_app` drops a refusal's reason |
| `sendSignal:toPid:` with SIGKILL and the pid archived | DVT's process control | iOS 26.6.2, Xcode 27.0: S9, `tests/lifecycle.rs` | written by hand: `kill_app`'s primitive pid stalled the phone's DVT service (S9) |
| `runningProcesses`, `isRunningPid:` | DVT's device information | iOS 26.6.2, Xcode 27.0: `tests/lifecycle.rs` | `bundleIdentifier` for application processes; the list takes 480 to 710 ms |
| AFC upload to `PublicStaging`; installation proxy `Upgrade`, `Lookup` | AFC, installation proxy (lockdown) | iOS 26.6.2, Xcode 27.0: S10, `tests/lifecycle.rs` | over usbmuxd, not the tunnel |

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
with an archived `NSError`, records every call with its arguments
unarchived, and drops the connection on a primitive pid as the phone did.
Its tunnel drops, closing the connections over it, and the device goes
away and comes back. On hosts other than macOS only the platform-neutral
tests run.

The tests in `tests/` drive the phone. Each checks `MOBILE_AGENT_IOS_UDID`
first; when it is unset, as in CI, the test prints why it skipped and
passes. To run them, attach an iPhone over USB, trusted, unlocked, with
Developer Mode on and Auto-Lock at Never, and name the team its runner is
signed for and the SwiftUI fixture's device build:

```sh
spikes/s10-ios-runner/fixtures.sh swiftui     # prints the bundle's path; task 6.1 moves this into fixtures/
MOBILE_AGENT_IOS_UDID=<udid> MOBILE_AGENT_IOS_TEAM=<team> \
  MOBILE_AGENT_IOS_SWIFTUI_APP=<path to FixtureSwiftUI.app> cargo test -p ios
cargo test -p ios --test lifecycle -- --ignored --nocapture install_series   # on a quiet host
```

Neither the identifier nor the team enters a tracked file, and no test
prints them. Without `MOBILE_AGENT_IOS_TEAM` the phone is listed as
`no_signing_identity`, which the discovery test accepts; without the
fixture's bundle the lifecycle tests skip. The lifecycle test installs the
fixture, which stays installed, launches and terminates it; nothing else is
installed and no setting is changed.

## Spec requirements and their tests

| Requirement | Scenario | Tests |
|---|---|---|
| Device discovery and readiness | Ready device listed | `discovery::the_phone_is_listed_once_with_its_identifier_name_model_and_ios_version`, `macos::driver::tests::a_ready_phone_is_listed_once_with_its_identifier_name_model_and_version`, `macos::host::tests::the_recorded_listing_goes_through_usbmuxds_parser_to_the_phones_usb_entry`, `discovery::tests::a_phone_listed_on_usb_and_the_network_is_kept_once_by_its_usb_entry`, `discovery::tests::the_recorded_values_describe_the_phone_and_make_it_ready`, `identity::tests::the_identity_of_the_named_team_is_found` |
| Device discovery and readiness | Device not ready | `macos::driver::tests::an_unready_phone_is_listed_in_its_state_with_the_step_to_take`, `macos::driver::tests::a_command_on_an_unready_phone_fails_with_a_code_naming_the_state`, `macos::driver::tests::a_command_needs_only_what_its_operation_needs`, `macos::driver::tests::a_command_on_a_phone_that_is_not_ready_fails_with_its_state`, `readiness::tests::every_unready_state_has_a_code_and_a_message_naming_its_step`, `readiness::tests::each_unready_state_is_the_first_check_the_device_has_not_passed`, `readiness::tests::an_operation_fails_only_on_the_states_it_needs`, `discovery::tests::each_recorded_value_that_changes_gives_its_state`, `macos::host::tests::a_failed_pairing_record_or_session_says_what_stands_in_the_way`, `identity::tests::a_missing_team_identity_or_team_says_which`, `discovery::a_command_reads_the_phone_again_and_an_unknown_identifier_is_not_found` |
| Transport without root or helpers | Services reached from the daemon (the launch; the screenshot comes with task 3.5) | `tunnel::the_tunnel_opens_in_this_unprivileged_process_and_lists_the_developer_services`, `lifecycle::the_fixture_is_installed_launched_with_an_environment_found_running_and_terminated`, `macos::tunnel::tests::the_handshake_lists_the_recorded_services_and_a_service_is_found_by_name`, `macos::tunnel::tests::the_services_a_device_without_the_disk_image_lacks_are_named_missing`, `macos::tunnel::tests::a_service_the_handshake_does_not_list_is_missing_and_the_tunnel_stays`, `macos::tunnel::tests::a_handshake_that_never_comes_times_out_and_leaves_nothing_open`, `macos::driver::tests::connecting_a_ready_phone_opens_the_tunnel_and_lists_its_services`, `macos::driver::tests::reset_closes_the_tunnel_and_the_next_connect_opens_a_new_one` |
| Transport without root or helpers | The disk image mounted when the device lacks one | `macos::driver::tests::a_phone_without_the_disk_image_has_it_mounted_before_the_tunnel_opens`, `macos::driver::tests::a_mount_that_fails_offline_fails_naming_the_signing_server_and_opens_nothing`, `macos::image::tests::a_missing_image_is_mounted_through_idevice_and_checked`, `macos::image::tests::idevice_hanging_or_failing_falls_back_to_devicectl`, `macos::image::tests::a_success_the_image_mounter_does_not_confirm_falls_back_too`, `macos::image::tests::both_failing_offline_say_the_signing_server_is_needed`, `macos::image::tests::the_manifest_names_the_image_and_trust_cache_of_the_devices_identity` (not on the device: the image stayed mounted) |
| App lifecycle | Fixture launched (the launch and the process; the tree read comes with task 3.4) | `lifecycle::the_fixture_is_installed_launched_with_an_environment_found_running_and_terminated`, `macos::driver::tests::install_checks_the_bundle_uploads_it_and_reports_its_identifier_and_time`, `macos::driver::tests::a_launch_reads_the_lock_then_launches_through_process_control_and_returns_the_pid`, `macos::driver::tests::a_launch_on_a_locked_phone_is_refused_without_being_tried`, `macos::driver::tests::launching_an_app_that_is_not_installed_fails_as_not_installed`, `macos::driver::tests::a_refused_launch_of_an_installed_app_says_why`, `macos::driver::tests::terminate_signals_the_apps_process_and_returns_once_it_is_gone`, `macos::driver::tests::terminating_an_app_that_is_not_running_signals_nothing`, `macos::dvt::tests::a_launch_sends_the_bundle_its_environment_arguments_and_kill_existing_and_reads_the_pid`, `macos::dvt::tests::a_kill_is_send_signal_to_pid_with_sigkill_and_the_pid_both_archived`, `macos::dvt::tests::the_process_list_gives_each_application_its_bundle_identifier`, `macos::dvt::tests::whether_a_pid_runs_is_asked_with_the_pid_archived`, `macos::dvt::tests::a_refused_launch_reads_the_errors_domain_code_and_description`, `bundle::tests::a_simulator_build_is_refused_naming_the_device_destination`, `macos::driver::tests::a_bundle_a_device_does_not_take_is_refused_before_anything_is_asked_of_it` |
| Failure containment | Cable pulled (the transport's part; the cable itself comes with task 3.6) | `macos::tunnel::tests::a_tunnel_that_dropped_is_opened_again_once`, `macos::tunnel::tests::a_device_that_is_gone_is_disconnected_with_the_cause`, `macos::driver::tests::a_call_after_the_tunnel_dropped_goes_through_a_new_tunnel`, `macos::dvt::tests::a_call_on_a_connection_that_closed_fails_and_the_channel_is_done`, `macos::dvt::tests::a_call_without_a_reply_times_out_and_ends_the_channel`, `macos::driver::tests::a_launch_whose_dvt_never_answers_times_out` |
| Compatibility statement | Table lists the device paths | `compat::tests::every_call_site_names_the_ios_and_xcode_it_was_validated_on`, `compat::tests::the_table_renders_the_range_and_every_call_site`, `compat::tests::ios_17_4_up_to_27_is_supported`, `readiness::tests::an_ios_outside_the_supported_range_is_refused_naming_the_version` |
