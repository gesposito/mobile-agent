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
    let _ = (detected, Version::parse(FIRST_SUPPORTED));
    let _ = Error::PlatformUnsupported;
    unimplemented!("task 6.9: Xcode gate")
}

/// The supported range in words.
pub fn supported() -> String {
    format!("Xcode {FIRST_SUPPORTED} up to but not including {FIRST_UNSUPPORTED}")
}

impl Compatibility {
    /// The table as aligned text.
    pub fn render(&self) -> String {
        unimplemented!("task 6.9: compatibility table")
    }

    /// Whether every row is usable.
    pub fn usable(&self) -> bool {
        unimplemented!("task 6.9: compatibility table")
    }
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
