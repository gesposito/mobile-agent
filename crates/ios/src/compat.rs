//! Which iOS versions the driver supports, and the compatibility table
//! `doctor` prints (spec `ios-device-driver`, "Compatibility statement").
//!
//! Every private or undocumented interface the driver uses on the device is
//! a [`CallSite`] with where it was validated: iOS 26.6.2 on an iPhone SE
//! (3rd generation) under Xcode 27.0, the only device and version this change
//! measures. A device outside the supported range is refused with its version
//! named ([`crate::Readiness::UnsupportedIos`]).

use crate::version::Version;

/// The first supported iOS: lockdown's `CoreDeviceProxy` carries the
/// CoreDevice tunnel over usbmuxd from iOS 17.4; before it the tunnel needs
/// RemotePairing over the USB Ethernet link, which this driver does not
/// implement (research note, "The CoreDevice tunnel, RemoteXPC and RSD").
pub const FIRST_SUPPORTED_IOS: &str = "17.4";
/// The first iOS past the supported range: iOS 27 drops `dtuhidd`'s reports
/// unless a display stream runs, a path gated for later (design, Open
/// Questions).
pub const FIRST_UNSUPPORTED_IOS: &str = "27";
/// Where the driver and its tests ran.
pub const VALIDATED_IOS: &str = "26.6.2";
pub const VALIDATED_XCODE: &str = "27.0";
/// The `idevice` crate the driver is built on, pinned with `=` as its README
/// asks: breaking changes come with every point release before 0.2.
pub const IDEVICE: &str = "0.1.68";
/// Where the compatibility notes are.
pub const NOTES: &str = "the Compatibility section of crates/ios/README.md";

/// Whether the driver supports a device running iOS `version`.
pub fn supported_ios(version: &str) -> bool {
    let first = Version::parse(FIRST_SUPPORTED_IOS).expect("a dotted version");
    let past = Version::parse(FIRST_UNSUPPORTED_IOS).expect("a dotted version");
    Version::parse(version).is_some_and(|version| version >= first && version < past)
}

/// The first iOS whose `dtuhidd` takes input from the host, by Xcode 27's
/// release notes ("input works from iOS 18.0 only").
pub const FIRST_HID_IOS: &str = "18";
/// The first iOS whose `dtuhidd` drops reports unless a display stream runs,
/// as pymobiledevice3 and `idevice` report; iOS 26.6.2 took them without one
/// (S11), and this driver opens no stream yet.
pub const FIRST_STREAM_HID_IOS: &str = "27";

/// Whether the device's `dtuhidd` takes reports from the host without a
/// display stream on iOS `version`; why not otherwise.
pub fn hid_input(version: Option<&str>) -> Result<(), String> {
    let Some(text) = version else {
        return Err("the device's iOS version is unknown, so its dtuhidd is not used".into());
    };
    let parsed = Version::parse(text)
        .ok_or_else(|| format!("iOS {text:?} is not a version this driver reads"))?;
    let first = Version::parse(FIRST_HID_IOS).expect("a dotted version");
    let streamed = Version::parse(FIRST_STREAM_HID_IOS).expect("a dotted version");
    if parsed < first {
        return Err(format!(
            "the device runs iOS {text}, and dtuhidd takes input from the host from iOS \
             {FIRST_HID_IOS}"
        ));
    }
    if parsed >= streamed {
        return Err(format!(
            "the device runs iOS {text}, whose dtuhidd drops reports unless a display stream \
             runs, which this driver does not open yet"
        ));
    }
    Ok(())
}

/// The supported range in words.
pub fn supported_range() -> String {
    format!(
        "iOS {FIRST_SUPPORTED_IOS} up to but not including {FIRST_UNSUPPORTED_IOS}, validated on {VALIDATED_IOS}"
    )
}

/// A private or undocumented interface the driver uses, and where it was
/// validated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallSite {
    /// The call, message or service, as the device names it.
    pub call: &'static str,
    /// The service or daemon that answers it.
    pub service: &'static str,
    /// The iOS and Xcode it was exercised on, and by what.
    pub validated_on: &'static str,
    /// What the driver avoids or works around at this call site.
    pub note: &'static str,
}

/// Every interface the driver uses on the device, with where it was
/// validated.
pub fn call_sites() -> Vec<CallSite> {
    const DISCOVERY: &str = "iOS 26.6.2 (iPhone SE 3rd generation), Xcode 27.0, macOS 26.6.2: spike S9 and tests/discovery.rs";
    const TUNNEL: &str = "iOS 26.6.2 (iPhone SE 3rd generation), Xcode 27.0, macOS 26.6.2: spike S9 and tests/tunnel.rs";
    const MOUNT: &str = "iOS 26.6.2, Xcode 27.0: not exercised, the image was mounted in S9, S10 and tests/tunnel.rs; S10's personalized mount through idevice never returned, though the image was mounted by then";
    const LIFECYCLE: &str = "iOS 26.6.2 (iPhone SE 3rd generation), Xcode 27.0, macOS 26.6.2: spike S9 (Settings) and tests/lifecycle.rs (the SwiftUI fixture)";
    const SESSION: &str = "iOS 26.6.2 (iPhone SE 3rd generation), Xcode 27.0 (XCTest 25228), macOS 26.6.2: spike S10 and tests/runner.rs";
    const INPUT: &str = "iOS 26.6.2 (iPhone SE 3rd generation), Xcode 27.0, macOS 26.6.2: spikes S10 and S11, and tests/input.rs";
    const KEYS: &str = "iOS 26.6.2 (iPhone SE 3rd generation), Xcode 27.0, macOS 26.6.2: spike S11 (typed into Safari's address field)";
    const FRAMES: &str = "iOS 26.6.2 (iPhone SE 3rd generation), Xcode 27.0, macOS 26.6.2: spikes S9 and S11, and tests/frames.rs";
    let site = |call, service, validated_on, note| CallSite {
        call,
        service,
        validated_on,
        note,
    };
    vec![
        site(
            "ListDevices, ReadPairRecord, Connect",
            "usbmuxd (/var/run/usbmuxd)",
            DISCOVERY,
            "a phone with Wi-Fi sync is listed twice; the USB entry is taken. The pairing record comes from usbmuxd, not /var/db/lockdown",
        ),
        site(
            "GetValue DeviceName, ProductType, ProductVersion, BuildVersion (no session); StartSession",
            "lockdownd (port 62078)",
            DISCOVERY,
            "",
        ),
        site(
            "GetValue DeveloperModeStatus in com.apple.security.mac.amfi",
            "lockdownd",
            DISCOVERY,
            "",
        ),
        site(
            "GetValue PasswordProtected",
            "lockdownd",
            DISCOVERY,
            "follows the lock and is read before every launch, on a session kept for it; CoreDevice's com.apple.coredevice.feature.getlockstate is listed but answers that it is not implemented on iOS 26.6.2 (S9), so it is never asked",
        ),
        site(
            "CopyDevices",
            "com.apple.mobile.mobile_image_mounter (lockdown)",
            DISCOVERY,
            "the personalized image of Xcode 27.0 at /System/Developer; a Cryptex1 form (iOS 17.4 and later, devices outside the image's build manifest) is not listed here",
        ),
        site(
            "StartService com.apple.internal.devicecompute.CoreDeviceProxy, CDTunnel's clientHandshakeRequest",
            "lockdownd, then the CoreDevice tunnel ended in this process by jktcp",
            TUNNEL,
            "no root and no TUN interface; jktcp sends one segment at a time and retransmits after 200 ms, so bulk transfers (installs) go through usbmuxd instead",
        ),
        site(
            "RSD handshake (RemoteXPC over HTTP/2, MessagingProtocolVersion 7)",
            "remoted's RSD port in the tunnel",
            TUNNEL,
            "82 services listed with Xcode 27.0's image mounted; 61 without it in S10, the test manager and the HID services gone",
        ),
        site(
            "_notifyOfPublishedCapabilities: on a new connection",
            "com.apple.instruments.dtservicehub (DVT)",
            TUNNEL,
            "read before the connection is kept, as idevice's own process-control tool does",
        ),
        site(
            "QueryPersonalizationManifest, QueryNonce, QueryPersonalizationIdentifiers, UploadImage, MountImage Personalized; TSS at gs.apple.com",
            "com.apple.mobile.mobile_image_mounter (lockdown), Apple's signing server",
            MOUNT,
            "tried first when CopyDevices lists no developer image, within 60 s; needs this Mac online",
        ),
        site(
            "xcrun devicectl device info ddiServices --auto-mount-ddis",
            "CoreDevice 642.16 on the host",
            MOUNT,
            "the fallback within 120 s, a process of its own; it mounts the Cryptex1 form for a device the build manifest does not name",
        ),
        site(
            "_requestChannelWithCode:identifier:",
            "com.apple.instruments.dtservicehub (DVT)",
            LIFECYCLE,
            "each channel on a DVT connection of its own, kept for the target's lifetime",
        ),
        site(
            "launchSuspendedProcessWithDevicePath:bundleIdentifier:environment:arguments:options: with KillExisting",
            "DVT's com.apple.instruments.server.services.processcontrol",
            LIFECYCLE,
            "written by hand, since idevice 0.1.68's launch_app drops the device's reason for a refusal; the NSError is read from the reply's archive; the launched app's output comes on the channel as outputReceived:fromProcess:atTime: (the Expo fixture's before the reply) and is passed over",
        ),
        site(
            "sendSignal:toPid: with SIGKILL and the pid archived",
            "DVT's com.apple.instruments.server.services.processcontrol",
            LIFECYCLE,
            "idevice 0.1.68's kill_app sends killPid: with the pid as a primitive integer, which left the app running and dropped every DVT connection on iOS 26.6.2 (S9); it is never called",
        ),
        site(
            "runningProcesses, isRunningPid:",
            "DVT's com.apple.instruments.server.services.deviceinfo",
            LIFECYCLE,
            "application processes carry bundleIdentifier on iOS 26.6.2; the list of about 580 processes takes 480 to 710 ms, so a terminated pid is waited for with isRunningPid:",
        ),
        site(
            "AFC upload to PublicStaging; installation_proxy Upgrade and Lookup",
            "com.apple.afc and com.apple.mobile.installation_proxy (lockdown)",
            LIFECYCLE,
            "through usbmuxd rather than the tunnel, whose stack sends one segment at a time",
        ),
        site(
            "installation_proxy Lookup of the runner (Path, Container, CFBundleExecutable ending in -Runner)",
            "com.apple.mobile.installation_proxy (lockdown)",
            SESSION,
            "a missing runner is installed from the bundle the caller gives, or reported as runner_not_installed",
        ),
        site(
            "the xctest session: the DTX handshake on two testmanagerd connections, _IDE_initiateControlSessionWithCapabilities:, _IDE_initiateSessionWithIdentifier:capabilities:, the runner launched through DVT's process control (XCTestBundlePath, XCTestSessionIdentifier, XCTestManagerVariant DDI), _IDE_authorizeTestSessionWithProcessID:, _XCT_testRunnerReadyWithCapabilities: answered with the configuration, _IDE_startExecutingTestPlanWithProtocolVersion:",
            "com.apple.dt.testmanagerd.remote and DVT, through idevice 0.1.68's XCUITestService on a tunnel of its own",
            SESSION,
            "kept on a task for the session; XCTest asks the owner to approve Enable UI Automation and waits 60 s in \"enabling automation mode\" (automation_not_enabled), which iOS 26.6.2 asked again once in a day without a restart; the handshake went unanswered for a minute or two after runners were ended mid-start, so a start is made again within the attach's time; idevice reports XCTestErrorDomain 103 as an untrusted certificate though S10's cause was an arm64e executable",
        ),
        site(
            "Connect to the runner's loopback port 27183; the device protocol's hello",
            "usbmuxd, then the runner's AgentSession",
            SESSION,
            "usbmuxd rather than the tunnel, as for installs; the hello names mobile-agent-ios-runner and its version, another version is replaced by the bundle given",
        ),
        site(
            "GetValue ScreenWidth, ScreenHeight, ScreenScaleFactor in com.apple.mobile.iTunes",
            "lockdownd",
            INPUT,
            "750, 1334 and 2.0 on the SE: the touchscreen's coordinates and the status bar's band come from them",
        ),
        site(
            "send {_0: a 58-byte mainTouchscreen report (0x09, contact count, 5, state 0xC2 touching or 0x02 lifted, x and y over 0 to 65535 across the screen in points), _1: 257}",
            "com.apple.coredevice.hid.universalhidservice (dtuhidd of the disk image)",
            INPUT,
            "no display stream runs, and iOS 26.6.2 takes the reports anyway (S11); iOS 27 drops them without one (pymobiledevice3, idevice), so the path is gated to iOS 18 up to 27 and the runner synthesizes elsewhere",
        ),
        site(
            "IndigoButtonEvent {usagePage 0x0C, usageCode: home 0x40, lock 0x30, volume up 0xE9, volume down 0xEA, state 1 down, 2 up}",
            "com.apple.coredevice.hid.indigo (dtuhidd)",
            INPUT,
            "held 80 ms; home reached the home screen, where the phone's own home button does not work (S11), and volume up and down showed the volume HUD, and lock locked the phone, PasswordProtected reading true 652 ms after the press (task 3.8)",
        ),
        site(
            "IndigoKeyboardButtonEvent {usageCode on the keyboard page, state}",
            "com.apple.coredevice.hid.indigo (dtuhidd)",
            KEYS,
            "a key held 30 ms with its modifiers' events around it; pymobiledevice3 found dtuhidd ignoring it on iOS 27",
        ),
        site(
            "createService (a virtual mainKeyboard), connectedServices, send {_0: a 39-byte keyboard report, _1: its service}, resetGestureState, removeService",
            "com.apple.coredevice.hid.universalhidservice (dtuhidd)",
            KEYS,
            "registered for typing and removed after it, since Xcode 27's notes say a device may otherwise stay in hardware keyboard mode for two minutes",
        ),
        site(
            "takeScreenshot",
            "DVT's com.apple.instruments.server.services.screenshot",
            FRAMES,
            "a PNG, 8-bit for most screens and 16-bit with an ICC profile for the home screen's wallpaper (S11); polled on a connection of its own for frames, and on the kept channel for screenshots",
        ),
        site(
            "capturescreenshot with requestedFormat png",
            "com.apple.coredevice.screencaptureservice",
            FRAMES,
            "the second frame connection; JPEG requests are refused (S9)",
        ),
        site(
            "getmediasupportinfo",
            "com.apple.coredevice.displayservice",
            FRAMES,
            "supportedFeatures 0 on iOS 26.6.2, which refuses the display stream with CoreDeviceError 9021 until iOS 27 (S11); the stream stays off, a later addition",
        ),
    ]
}

/// The table as aligned text: the supported range, then every call site.
pub fn render() -> String {
    let mut out = format!(
        "Supported: {}, under Xcode {VALIDATED_XCODE}, through idevice {IDEVICE}\n\n",
        supported_range()
    );
    let sites = call_sites();
    let rows: Vec<[&str; 4]> = sites
        .iter()
        .map(|site| [site.call, site.service, site.validated_on, site.note])
        .collect();
    out.push_str(&table(["Call", "Service", "Validated on", "Note"], &rows));
    out
}

/// Left-aligned columns, two spaces apart, the last one unpadded.
fn table<const N: usize>(header: [&str; N], rows: &[[&str; N]]) -> String {
    let mut widths = header.map(|cell| cell.chars().count());
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for row in std::iter::once(&header).chain(rows) {
        let mut line = String::new();
        for (index, (cell, width)) in row.iter().zip(widths).enumerate() {
            if index + 1 == N {
                line.push_str(cell);
            } else {
                line.push_str(&format!("{cell:<width$}  "));
            }
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ios_17_4_up_to_27_is_supported() {
        for version in ["17.4", "17.4.1", "18.0", "26.6.2", "26.9"] {
            assert!(supported_ios(version), "{version}");
        }
        for version in ["16.7.10", "17.3.1", "17", "27.0", "27", "28.1", "", "beta"] {
            assert!(!supported_ios(version), "{version}");
        }
    }

    #[test]
    fn host_input_is_gated_on_an_ios_that_takes_reports_without_a_stream() {
        for version in ["18.0", "18.4", "26.6.2", "26.9"] {
            assert_eq!(hid_input(Some(version)), Ok(()), "{version}");
        }
        let early = hid_input(Some("17.6")).expect_err("before iOS 18");
        assert!(early.contains("17.6") && early.contains("18"), "{early}");
        for version in ["27.0", "27.1", "28"] {
            let refused = hid_input(Some(version)).expect_err(version);
            assert!(
                refused.contains(version) && refused.contains("display stream"),
                "{refused}"
            );
        }
        assert!(hid_input(None).is_err());
    }

    #[test]
    fn every_call_site_names_the_ios_and_xcode_it_was_validated_on() {
        let sites = call_sites();
        assert!(!sites.is_empty());
        for site in &sites {
            assert!(
                site.validated_on.contains("iOS 26.6.2")
                    && site.validated_on.contains("Xcode 27.0"),
                "{} names no validated versions: {}",
                site.call,
                site.validated_on
            );
        }
        let calls: Vec<&str> = sites.iter().map(|site| site.call).collect();
        for expected in [
            "ListDevices",
            "ReadPairRecord",
            "DeveloperModeStatus",
            "PasswordProtected",
            "CopyDevices",
        ] {
            assert!(
                calls.iter().any(|call| call.contains(expected)),
                "no call site for {expected}: {calls:?}"
            );
        }
        for expected in [
            "testmanagerd",
            "27183",
            "mainTouchscreen",
            "IndigoButtonEvent",
            "IndigoKeyboardButtonEvent",
            "mainKeyboard",
            "takeScreenshot",
            "capturescreenshot",
            "getmediasupportinfo",
            "ScreenScaleFactor",
        ] {
            assert!(
                sites
                    .iter()
                    .any(|site| site.call.contains(expected) || site.service.contains(expected)),
                "no call site for {expected}"
            );
        }
        // The lock comes from lockdown because CoreDevice's lock state is not
        // implemented on iOS 26.6.2 (S9); the table says so at the call.
        let lock = sites
            .iter()
            .find(|site| site.call.contains("PasswordProtected"))
            .expect("the lock's call site");
        assert!(lock.note.contains("getlockstate"), "{}", lock.note);
    }

    #[test]
    fn the_table_renders_the_range_and_every_call_site() {
        let text = render();
        for part in [
            "17.4",
            "27",
            "26.6.2",
            "27.0",
            "0.1.68",
            "ListDevices",
            "PasswordProtected",
        ] {
            assert!(text.contains(part), "{part:?} is missing from:\n{text}");
        }
    }
}
