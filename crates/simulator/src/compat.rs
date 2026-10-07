//! Which Xcode the driver supports, and the compatibility table `doctor`
//! prints (design D12, spec "Xcode compatibility statement").

use crate::error::{Error, Result};
use crate::version::Version;

/// The first supported Xcode.
pub const FIRST_SUPPORTED: &str = "26.0";
/// The first Xcode past the supported range.
pub const FIRST_UNSUPPORTED: &str = "28";
/// Where the compatibility notes are.
pub const NOTES: &str = "the Compatibility section of crates/simulator/README.md";

/// How one component of the host fares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Validated on this version.
    Validated,
    /// Within the supported range but not validated.
    Supported,
    /// Present, with a known limitation stated in the row's note.
    Limited,
    /// Outside what the driver supports, or missing.
    Unsupported,
}

/// One line of the compatibility table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub component: String,
    pub detected: String,
    pub expected: String,
    pub status: Status,
    pub note: String,
}

/// A private call and where it was validated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CallSite {
    pub call: &'static str,
    pub image: &'static str,
    /// The host and runtime it was exercised on, and by what.
    pub validated_on: &'static str,
}

/// The table `doctor` prints: the host's components and the private calls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Compatibility {
    pub rows: Vec<Row>,
    pub call_sites: Vec<CallSite>,
}

/// Fails unless `detected`, the selected Xcode's version and build such as
/// "27.0 (27A266a)", is in the supported range.
pub fn check_xcode(detected: Option<&str>) -> Result<()> {
    let refuse = |detected: String| {
        Err(Error::UnsupportedXcode {
            detected,
            supported: supported(),
            notes: NOTES.into(),
        })
    };
    let Some(detected) = detected else {
        return refuse(String::new());
    };
    let first = Version::parse(FIRST_SUPPORTED).expect("a dotted version");
    let past = Version::parse(FIRST_UNSUPPORTED).expect("a dotted version");
    match detected.split_whitespace().next().and_then(Version::parse) {
        Some(version) if version >= first && version < past => Ok(()),
        _ => refuse(format!("Xcode {detected}")),
    }
}

/// Whether `xcode` is the version the driver was validated on.
pub fn validated_xcode(xcode: &str) -> bool {
    xcode.split_whitespace().next().and_then(Version::parse) == Version::parse(VALIDATED_XCODE)
}

/// The Xcode, CoreSimulator, SimulatorKit and runtime the spikes and this
/// crate's integration tests ran on.
pub const VALIDATED_XCODE: &str = "27.0";
pub const VALIDATED_CORE_SIMULATOR: &str = "1171.7";
pub const VALIDATED_SIMULATOR_KIT: &str = "1005.2";
pub const VALIDATED_RUNTIME: &str = "iOS 26.5";

/// Where the private calls the driver makes were validated.
pub fn call_sites() -> Vec<CallSite> {
    const S2: &str = "Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: spike S2, tests/lifecycle.rs and tests/frames.rs";
    const S3: &str = "Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: spike S3 and tests/dtuhid.rs";
    const INDIGO: &str =
        "Xcode 27.0, SimulatorKit 1005.2, iOS 26.5: spike S3 and tests/indigo.rs, on a fresh boot";
    const TEXT: &str = "Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: tests/text.rs";
    const PASTE: &str = "Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: task 10.31's benchmark and tests/text.rs; devicectl device pasteboard copy when it fails";
    const LIFECYCLE: &str = "Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: tests/lifecycle.rs";
    const READER: &str = "Xcode 27.0, CoreSimulator 1171.7, iOS 26.5: spike S4 and tests/tree.rs";
    const GUEST: &str = "iOS 26.5 runtime, inside the Simulator: spike S4 and tests/tree.rs, through simulator-reader";
    const TRANSLATOR: &str = "macOS 26.6, AccessibilityPlatformTranslation 545.18, iOS 26.5: spike S4 and tests/tree.rs (fallback)";
    let site = |call, image, validated_on| CallSite {
        call,
        image,
        validated_on,
    };
    vec![
        site(
            "+[SimServiceContext sharedServiceContextForDeveloperDir:error:], -defaultDeviceSetWithError:, -supportedRuntimes, -supportedDeviceTypes",
            "CoreSimulator",
            S2,
        ),
        site(
            "-[SimDeviceSet devices], -createDeviceAsyncWithType:runtime:name:completionQueue:completionHandler:, -deleteDeviceAsync:completionQueue:completionHandler:",
            "CoreSimulator",
            S2,
        ),
        site(
            "-[SimDevice UDID], -name, -state, -runtime, -deviceType, -bootStatus",
            "CoreSimulator",
            S2,
        ),
        site(
            "-[SimDevice bootAsyncWithOptions:completionQueue:completionHandler:], -shutdownAsyncWithCompletionQueue:completionHandler:",
            "CoreSimulator",
            S2,
        ),
        site(
            "-[SimDevice installApplication:withOptions:error:], -launchApplicationAsyncWithID:options:completionQueue:completionHandler:, -terminateApplicationWithID:error:",
            "CoreSimulator",
            S2,
        ),
        site("-[SimDevice dataPath]", "CoreSimulator", LIFECYCLE),
        site(
            "-[SimDevice installedAppsWithError:], -setPrivacyAccessForService:bundleID:granted:error: (kTCCServicePasteboard)",
            "CoreSimulator",
            TEXT,
        ),
        site(
            "-[SimDevice lookup:error:] for com.apple.coredevice.feature.pasteboard, the _4sim libxpc functions, its SET command and SET_REPLY",
            "CoreSimulator, libxpc, dtpasteboardd of CoreDevice 642.16 (in the guest)",
            PASTE,
        ),
        site(
            "-[SimDevice io], -[SimDeviceIOClient ioPorts], port -descriptor, descriptor -state and -displayClass",
            "CoreSimDeviceIO, ROCKit proxies",
            S2,
        ),
        site(
            "-[SimDisplayIOSurfaceRenderable framebufferSurface], -[SimScreen registerScreenCallbacksWithUUID:callbackQueue:frameCallback:surfacesChangedCallback:propertiesChangedCallback:], -unregisterScreenCallbacksWithUUID:",
            "CoreSimDeviceIO, ROCKit proxies",
            S2,
        ),
        site("-[SimDevice lookup:error:]", "CoreSimulator", S3),
        site(
            "xpc_endpoint_create_mach_port_4sim, xpc_connection_create_from_endpoint, xpc_connection_enable_sim2host_4sim",
            "libxpc",
            S3,
        ),
        site(
            "SimDeviceLegacyHIDClient -initWithDevice:error:, -sendWithMessage:freeWhenDone:completionQueue:completion:",
            "SimulatorKit",
            INDIGO,
        ),
        site(
            "IndigoHIDMessageForMouseNSEvent (six parameters; down, dragged and up)",
            "SimulatorKit",
            INDIGO,
        ),
        site(
            "-[SimDevice spawnWithPath:options:terminationQueue:terminationHandler:pid:error:]",
            "CoreSimulator",
            READER,
        ),
        site(
            "-[XCTAccessibilityFramework initForRemoteAccess], -userTestingSnapshotForElement:options:error:, +[XCAccessibilityElement elementWithProcessIdentifier:], -AXUIElement, XCTDefaultSnapshotParameters, XCAXAccessibilityAttributesForStringAttributes",
            "XCTAutomationSupport (in the guest)",
            GUEST,
        ),
        site(
            "+[AXElement primaryApp], -pid, AXValueGetValue, AXValueGetTypeID",
            "AXRuntime (in the guest)",
            GUEST,
        ),
        site(
            "+[AXSettings sharedInstance], -setAutomationEnabled:, _AXSAutomationEnabled",
            "AccessibilityUtilities, libAccessibility (in the guest)",
            GUEST,
        ),
        site(
            "+[AXPTranslator sharedInstance], -setBridgeTokenDelegate:, -frontmostApplicationWithDisplayId:bridgeDelegateToken:, -macPlatformElementFromTranslation:, AXPTranslationObject -setBridgeDelegateToken:, -pid",
            "AccessibilityPlatformTranslation",
            TRANSLATOR,
        ),
        site(
            "AXPMacPlatformElement -translation, -accessibilityRole, -accessibilitySubrole, -accessibilityLabel, -accessibilityIdentifier, -accessibilityValue, -accessibilityFrame, -accessibilityAttributeValue: AXTraits, -accessibilityChildren",
            "AccessibilityPlatformTranslation",
            TRANSLATOR,
        ),
        site(
            "-[SimDevice sendAccessibilityRequestAsync:completionQueue:completionHandler:]",
            "CoreSimulator",
            TRANSLATOR,
        ),
    ]
}

/// The supported range in words.
pub fn supported() -> String {
    format!("Xcode {FIRST_SUPPORTED} up to but not including {FIRST_UNSUPPORTED}")
}

impl Compatibility {
    /// The table as aligned text.
    pub fn render(&self) -> String {
        let status = |status: Status| match status {
            Status::Validated => "validated",
            Status::Supported => "supported",
            Status::Limited => "limited",
            Status::Unsupported => "unsupported",
        };
        let rows: Vec<[&str; 5]> = self
            .rows
            .iter()
            .map(|row| {
                [
                    row.component.as_str(),
                    row.detected.as_str(),
                    row.expected.as_str(),
                    status(row.status),
                    row.note.as_str(),
                ]
            })
            .collect();
        let mut out = table(
            ["Component", "Detected", "Expected", "Status", "Note"],
            &rows,
        );
        out.push('\n');
        let sites: Vec<[&str; 3]> = self
            .call_sites
            .iter()
            .map(|site| [site.call, site.image, site.validated_on])
            .collect();
        out.push_str(&table(["Private call", "Image", "Validated on"], &sites));
        out
    }

    /// Whether every row is usable.
    pub fn usable(&self) -> bool {
        self.rows
            .iter()
            .all(|row| row.status != Status::Unsupported)
    }
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
    fn xcode_26_and_27_are_supported() {
        for version in [
            "26.0 (17A324)",
            "26.4 (17E192)",
            "27.0 (27A266a)",
            "27.3 (27D5)",
        ] {
            assert_eq!(check_xcode(Some(version)), Ok(()), "{version}");
        }
    }

    #[test]
    fn unsupported_xcode() {
        let error = check_xcode(Some("28.0 (28A5)")).expect_err("Xcode 28 is past the range");
        assert_eq!(error.code(), "unsupported_xcode");
        let message = error.to_string();
        for part in [
            "28.0 (28A5)",
            "26.0",
            "28",
            "Compatibility section of crates/simulator/README.md",
        ] {
            assert!(
                message.contains(part),
                "{part:?} is missing from: {message}"
            );
        }
        assert!(
            check_xcode(Some("16.4 (16F6)")).is_err(),
            "before the range"
        );
    }

    #[test]
    fn a_host_without_xcode_is_refused() {
        let message = check_xcode(None).expect_err("no Xcode").to_string();
        assert!(message.contains("no Xcode"), "{message}");
        assert!(check_xcode(Some("garbage")).is_err());
    }

    #[test]
    fn the_table_renders_every_row_and_call_site() {
        let table = Compatibility {
            rows: vec![Row {
                component: "Xcode".into(),
                detected: "27.0 (27A266a)".into(),
                expected: supported(),
                status: Status::Validated,
                note: "the spikes ran here".into(),
            }],
            call_sites: vec![CallSite {
                call: "-[SimDevice lookup:error:]",
                image: "CoreSimulator",
                validated_on: "CoreSimulator 1171.7, iOS 26.5 (S3)",
            }],
        };
        let text = table.render();
        for part in [
            "Xcode",
            "27.0 (27A266a)",
            "validated",
            "the spikes ran here",
            "-[SimDevice lookup:error:]",
            "CoreSimulator 1171.7, iOS 26.5 (S3)",
        ] {
            assert!(text.contains(part), "{part:?} is missing from:\n{text}");
        }
        assert!(table.usable());
        let broken = Compatibility {
            rows: vec![Row {
                status: Status::Unsupported,
                ..table.rows[0].clone()
            }],
            call_sites: Vec::new(),
        };
        assert!(!broken.usable());
    }
}
